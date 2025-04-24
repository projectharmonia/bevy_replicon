use core::{ops::Range, time::Duration};

use bevy::{ecs::component::Tick, prelude::*};
use postcard::experimental::{max_size::MaxSize, serialized_size};

use super::{change_ranges::ChangeRanges, serialized_data::SerializedData};
use crate::shared::{
    backend::{replicon_channels::ReplicationChannel, replicon_server::RepliconServer},
    postcard_utils,
    replication::{
        client_ticks::{ClientTicks, EntityBuffer},
        mutate_index::MutateIndex,
    },
    replicon_tick::RepliconTick,
};

/// Component mutations for the current tick.
///
/// The data is serialized manually and stored in the form of ranges
/// from [`SerializedData`].
///
/// Can be packed into messages using [`Self::send`].
#[derive(Default, Component)]
pub(crate) struct Mutations {
    /// List of entity values for [`Self::mutations`].
    ///
    /// Used to associate entities with the mutate index that the client
    /// needs to acknowledge to consider entity mutations as received.
    entities: Vec<Entity>,

    /// Component mutations that happened in this tick.
    ///
    /// Serialized as a list of pairs of entity chunk and multiple chunks with mutated components.
    /// Components are stored in multiple chunks because some clients may acknowledge mutations,
    /// while others may not.
    ///
    /// Unlike [`Updates`](super::updates::Updates), we serialize the number
    /// of chunk bytes instead of the number of components. This is because, during deserialization,
    /// some entities may be skipped if they have already been updated (as mutations are sent until
    /// the client acknowledges them).
    mutations: Vec<ChangeRanges>,

    /// Indicates that an entity has been written since the
    /// last call of [`Self::start_entity`].
    entity_added: bool,

    /// Intermediate buffer to reuse allocated memory from [`Self::mutations`].
    buffer: Vec<Vec<Range<usize>>>,

    /// Intermediate buffer with mutate index, message size and a range for [`Self::mutations`].
    ///
    /// We split messages first in order to know their count in advance.
    messages: Vec<(MutateIndex, usize, Range<usize>)>,
}

impl Mutations {
    /// Updates internal state to start writing mutated components for an entity.
    ///
    /// Entities and their data written lazily during the iteration.
    /// See [`Self::add_entity`] and [`Self::add_component`].
    pub(crate) fn start_entity(&mut self) {
        self.entity_added = false;
    }

    /// Returns `true` if [`Self::add_entity`] were called since the last
    /// call of [`Self::start_entity`].
    pub(crate) fn entity_added(&mut self) -> bool {
        self.entity_added
    }

    /// Adds an entity chunk.
    pub(crate) fn add_entity(&mut self, entity: Entity, entity_range: Range<usize>) {
        let components = self.buffer.pop().unwrap_or_default();
        self.mutations.push(ChangeRanges {
            entity: entity_range,
            components_len: 0,
            components,
        });
        self.entities.push(entity);
        self.entity_added = true;
    }

    /// Adds a component chunk to the last added entity from [`Self::add_entity`].
    pub(crate) fn add_component(&mut self, component: Range<usize>) {
        let changes = self
            .mutations
            .last_mut()
            .expect("entity should be written before adding components");

        changes.add_component(component);
    }

    /// Returns written mutations for the last entity from [`Self::add_entity`].
    pub(super) fn last(&mut self) -> Option<&ChangeRanges> {
        self.mutations.last()
    }

    /// Removes last added entity from [`Self::add_entity`] with associated components.
    ///
    /// keeps allocated memory for reuse.
    pub(super) fn pop(&mut self) {
        self.entities.pop();
        if let Some(mut changes) = self.mutations.pop() {
            changes.components.clear();
            self.buffer.push(changes.components);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    /// Packs mutations into messages.
    ///
    /// Sent over the [`ReplicationChannel::Mutations`] channel. If the message gets lost, we try to resend it manually,
    /// using the last up-to-date mutations to avoid re-sending old values.
    ///
    /// Contains update tick, current tick, mutate index and component mutations since
    /// the last acknowledged tick for each entity.
    ///
    /// Cannot be applied on the client until the update message matching this message's update tick
    /// has been applied to the client world.
    /// The message will be manually split into packets up to max size, and each packet will be applied
    /// independently on the client.
    /// Message splits only happen per-entity to avoid weird behavior from partial entity mutations.
    ///
    /// After sendining all data in the component will be cleared.
    pub(crate) fn send(
        &mut self,
        server: &mut RepliconServer,
        client_entity: Entity,
        client: &mut ClientTicks,
        entity_buffer: &mut EntityBuffer,
        serialized: &SerializedData,
        track_mutate_messages: bool,
        server_tick: Range<usize>,
        tick: Tick,
        timestamp: Duration,
        max_size: usize,
    ) -> Result<usize> {
        debug_assert_eq!(self.entities.len(), self.mutations.len());

        const MAX_COUNT_SIZE: usize = usize::POSTCARD_MAX_SIZE;
        let mut tick_buffer = [0; RepliconTick::POSTCARD_MAX_SIZE];
        let update_tick = postcard::to_slice(&client.update_tick(), &mut tick_buffer)?;
        let mut metadata_size = update_tick.len() + server_tick.len();
        if track_mutate_messages {
            metadata_size += MAX_COUNT_SIZE;
        }

        let (mut mutate_index, mut entities) =
            client.register_mutate_message(entity_buffer, tick, timestamp);
        let mut header_size = metadata_size + serialized_size(&mutate_index)?;
        let mut body_size = 0;
        let mut mutations_range = Range::<usize>::default();
        for (entity, changes) in self.entities.iter().zip(&self.mutations) {
            let changes_size = changes.size_with_components_size()?;

            // Try to pack back first, then try to pack forward.
            if body_size != 0
                && !can_pack(header_size + body_size, changes_size, max_size)
                && !can_pack(header_size + changes_size, body_size, max_size)
            {
                self.messages.push((
                    mutate_index,
                    body_size + header_size,
                    mutations_range.clone(),
                ));

                mutations_range.start = mutations_range.end;
                (mutate_index, entities) =
                    client.register_mutate_message(entity_buffer, tick, timestamp);
                header_size = metadata_size + serialized_size(&mutate_index)?; // Recalculate since the mutate index changed.
                body_size = 0;
            }

            entities.push(*entity);
            mutations_range.end += 1;
            body_size += changes_size;
        }
        if !mutations_range.is_empty() || track_mutate_messages {
            // When the loop ends, pack all leftovers into a message.
            // Or create an empty message if tracking mutate messages is enabled.
            self.messages.push((
                mutate_index,
                body_size + header_size,
                mutations_range.clone(),
            ));
        }

        let messages_count = self.messages.len();
        for (mutate_index, mut message_size, mutations_range) in self.messages.drain(..) {
            if track_mutate_messages {
                // Update message counter size based on actual value.
                message_size -= MAX_COUNT_SIZE - serialized_size(&messages_count)?;
            }
            let mut message = Vec::with_capacity(message_size);

            message.extend_from_slice(update_tick);
            message.extend_from_slice(&serialized[server_tick.clone()]);
            if track_mutate_messages {
                postcard_utils::to_extend_mut(&messages_count, &mut message)?;
            }
            postcard_utils::to_extend_mut(&mutate_index, &mut message)?;
            for changes in &self.mutations[mutations_range.clone()] {
                message.extend_from_slice(&serialized[changes.entity.clone()]);
                postcard_utils::to_extend_mut(&changes.components_size(), &mut message)?;
                for component in &changes.components {
                    message.extend_from_slice(&serialized[component.clone()]);
                }
            }

            debug_assert_eq!(message.len(), message_size);

            server.send(client_entity, ReplicationChannel::Mutations, message);
        }

        self.clear();

        Ok(messages_count)
    }

    /// Clears all chunks.
    ///
    /// Keeps allocated memory for reuse.
    fn clear(&mut self) {
        self.entities.clear();
        self.buffer
            .extend(self.mutations.drain(..).map(|mut changes| {
                changes.components.clear();
                changes.components
            }));
    }
}

fn can_pack(message_size: usize, add: usize, mtu: usize) -> bool {
    let dangling = message_size % mtu;
    (dangling > 0) && ((dangling + add) <= mtu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing() {
        const MAX_SIZE: usize = 1200;

        assert!(can_pack(10, 5, MAX_SIZE));
        assert!(can_pack(10, 1190, MAX_SIZE));
        assert!(!can_pack(10, 1191, MAX_SIZE));
        assert!(!can_pack(10, 3000, MAX_SIZE));

        assert!(can_pack(1199, 1, MAX_SIZE));
        assert!(!can_pack(1200, 0, MAX_SIZE));
        assert!(!can_pack(1200, 1, MAX_SIZE));
        assert!(!can_pack(1200, 3000, MAX_SIZE));
    }
}
