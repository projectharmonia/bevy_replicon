use std::{io::Cursor, mem, ops::Range, time::Duration};

use bevy::{ecs::component::Tick, prelude::*};
use bincode::{DefaultOptions, Options};
use integer_encoding::{VarInt, VarIntWriter};

use super::serialized_data::SerializedData;
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
/// Cannot be applied on the client until the change message matching this message change tick
/// has been applied to the client world.
/// The message will be manually split into packets up to max size, and each packet will be applied
/// independently on the client.
/// Message splits only happen per-entity to avoid weird behavior from partial entity mutations.
///
/// The data is stored in the form of ranges from [`SerializedData`].
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

    /// Component mutations happened on this tick.
    ///
    /// Serialized as a list of pairs of entity chunk and multiple chunks with mutated components.
    /// Components are stored in multiple chunks because some clients may acknowledge mutations,
    /// while others may not.
    mutations: Vec<(Range<usize>, Vec<Range<usize>>)>,

    /// Indicates that an entity has been written since the
    /// last call of [`Self::start_entity_mutations`].
    mutations_written: bool,

    /// Intermediate buffer to reuse allocated memory from [`Self::mutations`].
    buffer: Vec<Vec<Range<usize>>>,

    /// Intermediate buffer with mutate index, message size and a range for [`Self::mutations`].
    ///
    /// We split messages first in order to know their count in advance.
    /// We plan to include it in the message in the future.
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
        self.mutations.push((entity_range, components));
        self.entities.push(entity);
        self.mutations_written = true;
    }

    /// Adds a component chunk to the last added entity from [`Self::add_mutated_entity`].
    pub(crate) fn add_mutated_component(&mut self, component: Range<usize>) {
        let (_, components) = self
            .mutations
            .last_mut()
            .expect("entity should be written before adding components");

        if let Some(last) = components.last_mut() {
            // Append to previous range if possible.
            if last.end == component.start {
                last.end = component.end;
                return;
            }
        }

        components.push(component);
    }

    /// Removes and returns last mutated entity with its component chunks.
    pub(super) fn pop_mutations(
        &mut self,
    ) -> Option<(Range<usize>, impl Iterator<Item = Range<usize>> + '_)> {
        let (entity, components) = self.mutations.pop()?;
        self.buffer.push(components);
        let components = self.buffer.last_mut().unwrap();
        Some((entity, components.drain(..)))
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
        server_tick: Range<usize>,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<()> {
        const MAX_TICK_SIZE: usize = mem::size_of::<RepliconTick>() + 1;
        let mut change_tick = Cursor::new([0; MAX_TICK_SIZE]);
        DefaultOptions::new().serialize_into(&mut change_tick, &client.change_tick())?;
        let change_tick_size = change_tick.position() as usize;
        let ticks_size = change_tick_size + server_tick.len();

        let (mut mutate_index, mut entities) =
            client.register_mutate_message(client_buffers, tick, timestamp);
        let mut message_size = ticks_size + mutate_index.required_space();
        let mut mutations_range = Range::<usize>::default();
        for (entity, (entity_range, components)) in self.entities.iter().zip(&self.mutations) {
            const MAX_PACKET_SIZE: usize = 1200; // TODO: make it configurable by the messaging backend.
            let components_size = components.iter().map(|range| range.len()).sum::<usize>();
            let data_size = entity_range.len() + components_size.required_space() + components_size;

            if message_size == 0 || message_size + data_size <= MAX_PACKET_SIZE {
                entities.push(*entity);
                mutations_range.end += 1;
                message_size += data_size;
            } else {
                self.messages
                    .push((mutate_index, message_size, mutations_range.clone()));

                mutations_range.start = mutations_range.end;
                (mutate_index, entities) =
                    client.register_mutate_message(client_buffers, tick, timestamp);
                entities.push(*entity);
                mutations_range.end += 1;
                message_size = ticks_size + mutate_index.required_space() + data_size;
            }
        }
        if !mutations_range.is_empty() {
            self.messages
                .push((mutate_index, message_size, mutations_range.clone()));
        }

        for (mutate_index, message_size, mutations_range) in self.messages.drain(..) {
            let mut message = Vec::with_capacity(message_size);

            message.extend_from_slice(&change_tick.get_ref()[..change_tick_size]);
            message.extend_from_slice(&serialized[server_tick.clone()]);
            message.write_varint(mutate_index)?;
            for (entity, components) in &self.mutations[mutations_range.clone()] {
                let components_size = components.iter().map(|range| range.len()).sum::<usize>();
                message.extend_from_slice(&serialized[entity.clone()]);
                message.write_varint(components_size)?;
                for component in components {
                    message.extend_from_slice(&serialized[component.clone()]);
                }
            }

            debug_assert_eq!(message.len(), message_size);

            server.send(client.id(), ReplicationChannel::Mutations, message);
        }

        Ok(())
    }

    /// Clears all chunks.
    ///
    /// Keeps allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.entities.clear();
        self.buffer
            .extend(self.mutations.drain(..).map(|(_, mut range)| {
                range.clear();
                range
            }));
    }
}
