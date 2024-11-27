use std::{io::Cursor, mem, ops::Range, time::Duration};

use bevy::{ecs::component::Tick, prelude::*};
use integer_encoding::{VarInt, VarIntWriter};

use super::{component_changes::ComponentChanges, serialized_data::SerializedData};
use crate::core::{
    channels::ReplicationChannel,
    replication::replicated_clients::{ClientBuffers, ReplicatedClient},
    replicon_server::RepliconServer,
    replicon_tick::RepliconTick,
};

/// A message with replicated component mutations.
///
/// Contains change tick, current tick, mutate index and component mutations since
/// the last acknowledged tick for each entity.
///
/// Cannot be applied on the client until the change message matching this message's change tick
/// has been applied to the client world.
/// The message will be manually split into packets up to max size, and each packet will be applied
/// independently on the client.
/// Message splits only happen per-entity to avoid weird behavior from partial entity mutations.
///
/// The data is serialized manually and stored in the form of ranges
/// from [`SerializedData`].
///
/// Sent over the [`ReplicationChannel::Mutations`] channel. If the message gets lost, we try to resend it manually,
/// using the last up-to-date mutations to avoid re-sending old values.
///
/// Stored inside [`ReplicationMessages`](super::ReplicationMessages).
#[derive(Default)]
pub(crate) struct MutateMessage {
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
    /// Unlike [`ChangeMessage`](super::change_message::ChangeMessage), we serialize the number
    /// of chunk bytes instead of the number of components. This is because, during deserialization,
    /// some entities may be skipped if they have already been updated (as mutations are sent until
    /// the client acknowledges them).
    mutations: Vec<ComponentChanges>,

    /// Indicates that an entity has been written since the
    /// last call of [`Self::start_entity_mutations`].
    mutations_written: bool,

    /// Intermediate buffer to reuse allocated memory from [`Self::mutations`].
    buffer: Vec<Vec<Range<usize>>>,

    /// Intermediate buffer with mutate index, message size and a range for [`Self::mutations`].
    ///
    /// We split messages first in order to know their count in advance.
    messages: Vec<(u16, usize, Range<usize>)>,
}

impl MutateMessage {
    /// Updates internal state to start writing mutated components for an entity.
    ///
    /// Entities and their data written lazily during the iteration.
    /// See [`Self::add_mutated_entity`] and [`Self::add_mutated_component`].
    pub(crate) fn start_entity_mutations(&mut self) {
        self.mutations_written = false;
    }

    /// Returns `true` if [`Self::add_mutated_entity`] were called since the last
    /// call of [`Self::start_entity_mutations`].
    pub(crate) fn mutations_written(&mut self) -> bool {
        self.mutations_written
    }

    /// Adds an entity chunk.
    pub(crate) fn add_mutated_entity(&mut self, entity: Entity, entity_range: Range<usize>) {
        let components = self.buffer.pop().unwrap_or_default();
        self.mutations.push(ComponentChanges {
            entity: entity_range,
            components_len: 0,
            components,
        });
        self.entities.push(entity);
        self.mutations_written = true;
    }

    /// Adds a component chunk to the last added entity from [`Self::add_mutated_entity`].
    pub(crate) fn add_mutated_component(&mut self, component: Range<usize>) {
        let mutations = self
            .mutations
            .last_mut()
            .expect("entity should be written before adding components");

        mutations.add_component(component);
    }

    /// Returns written mutations for the last entity from [`Self::add_mutated_entity`].
    pub(super) fn last_mutations(&mut self) -> Option<&ComponentChanges> {
        self.mutations.last()
    }

    /// Removes last added entity from [`Self::add_mutated_entity`] with associated components.
    pub(super) fn pop_mutations(&mut self) {
        self.entities.pop();
        if let Some(mut mutations) = self.mutations.pop() {
            mutations.components.clear();
            self.buffer.push(mutations.components);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    pub(crate) fn send(
        &mut self,
        server: &mut RepliconServer,
        client: &mut ReplicatedClient,
        client_buffers: &mut ClientBuffers,
        serialized: &SerializedData,
        track_mutate_messages: bool,
        server_tick: Range<usize>,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<usize> {
        debug_assert_eq!(self.entities.len(), self.mutations.len());

        const MAX_COUNT_SIZE: usize = mem::size_of::<usize>() + 1;
        let mut change_tick = Cursor::new([0; mem::size_of::<RepliconTick>()]);
        bincode::serialize_into(&mut change_tick, &client.change_tick())?;
        let mut metadata_size = change_tick.get_ref().len() + server_tick.len();
        if track_mutate_messages {
            metadata_size += MAX_COUNT_SIZE;
        }

        let (mut mutate_index, mut entities) =
            client.register_mutate_message(client_buffers, tick, timestamp);
        let mut header_size = metadata_size + mutate_index.required_space();
        let mut body_size = 0;
        let mut mutations_range = Range::<usize>::default();
        for (entity, mutations) in self.entities.iter().zip(&self.mutations) {
            let components_size = mutations.components_size();
            let mutations_size =
                mutations.entity.len() + components_size.required_space() + components_size;

            // Try to pack back first, then try to pack forward.
            if body_size != 0
                && !can_pack(header_size + body_size, mutations_size)
                && !can_pack(header_size + mutations_size, body_size)
            {
                self.messages.push((
                    mutate_index,
                    body_size + header_size,
                    mutations_range.clone(),
                ));

                mutations_range.start = mutations_range.end;
                (mutate_index, entities) =
                    client.register_mutate_message(client_buffers, tick, timestamp);
                header_size = metadata_size + mutate_index.required_space(); // Recalculate since the mutate index changed.
                body_size = 0;
            }

            entities.push(*entity);
            mutations_range.end += 1;
            body_size += mutations_size;
        }
        if !mutations_range.is_empty() || track_mutate_messages {
            // When the loop ends, pack all leftovers into a message.
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
                message_size -= MAX_COUNT_SIZE - messages_count.required_space();
            }
            let mut message = Vec::with_capacity(message_size);

            message.extend_from_slice(change_tick.get_ref());
            message.extend_from_slice(&serialized[server_tick.clone()]);
            if track_mutate_messages {
                message.write_varint(messages_count)?;
            }
            message.write_varint(mutate_index)?;
            for mutations in &self.mutations[mutations_range.clone()] {
                message.extend_from_slice(&serialized[mutations.entity.clone()]);
                message.write_varint(mutations.components_size())?;
                for component in &mutations.components {
                    message.extend_from_slice(&serialized[component.clone()]);
                }
            }

            debug_assert_eq!(message.len(), message_size);

            server.send(client.id(), ReplicationChannel::Mutations, message);
        }

        Ok(messages_count)
    }

    /// Clears all chunks.
    ///
    /// Keeps allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.entities.clear();
        self.buffer
            .extend(self.mutations.drain(..).map(|mut mutations| {
                mutations.components.clear();
                mutations.components
            }));
    }
}

fn can_pack(message_size: usize, add: usize) -> bool {
    const MAX_PACKET_SIZE: usize = 1200; // TODO: make it configurable by the messaging backend.

    let dangling = message_size % MAX_PACKET_SIZE;
    (dangling > 0) && ((dangling + add) <= MAX_PACKET_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing() {
        assert!(can_pack(10, 5));
        assert!(can_pack(10, 1190));
        assert!(!can_pack(10, 1191));
        assert!(!can_pack(10, 3000));

        assert!(can_pack(1199, 1));
        assert!(!can_pack(1200, 0));
        assert!(!can_pack(1200, 1));
        assert!(!can_pack(1200, 3000));
    }
}
