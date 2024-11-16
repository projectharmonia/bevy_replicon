use std::ops::Range;

use bevy::prelude::*;
use integer_encoding::{FixedIntWriter, VarInt, VarIntWriter};

use super::{serialized_data::SerializedData, update_message::UpdateMessage};
use crate::core::{
    channels::ReplicationChannel,
    replication::{
        replicated_clients::{client_visibility::Visibility, ReplicatedClient},
        InitMessageArrays,
    },
    replicon_server::RepliconServer,
};

/// A message with replicated data.
///
/// Contains tick, mappings, insertions, removals and despawns that
/// happened on this tick.
///
/// The data is stored in the form of ranges from [`SerializedData`].
///
/// Sent over [`ReplicationChannel::Init`] channel.
///
/// All sizes are serialized as `usize`, but we use variable integer encoding,
/// so they are correctly deserialized even on a client with a different pointer size.
/// However, if the server sends a value larger than what a client can fit into `usize`
/// (which is very unlikely), the client will panic. This is expected,
/// as it means the client can't have an array of such a size anyway.
///
/// Stored inside [`ReplicationMessages`](super::ReplicationMessages).
#[derive(Default)]
pub(crate) struct InitMessage {
    /// Mappings for client's pre-spawned entities.
    ///
    /// Serialized as single continuous chunk of entity pairs.
    ///
    /// Mappings should be processed first, so all referenced entities after it will behave correctly.
    ///
    /// See aslo [`ClientEntityMap`](crate::server::client_entity_map::ClientEntityMap).
    mappings: Range<usize>,

    /// Number pairs encoded in [`Self::mappings`].
    mappings_len: usize,

    /// Despawn happened on this tick.
    ///
    /// Since clients may see different entities, it's serialized as multiple chunks of entities.
    /// I.e. serialized despawns may have holes due to visibility differences.
    despawns: Vec<Range<usize>>,

    /// Number of depspawned entities.
    ///
    /// May not be equal to the length of [`Self::despawns`] since adjacent ranges are merged together.
    despawns_len: usize,

    /// Component removals happened on this tick.
    ///
    /// Serialized as a list of pairs of entity chunk and a list of
    /// [`FnsId`](crate::core::replication::replication_registry::FnsId)
    /// serialized as a single chunk.
    ///
    /// For entities, we serialize their count like other data, but for IDs,
    /// we serialize their size in bytes.
    removals: Vec<(Range<usize>, Range<usize>)>,

    /// Component insertions or changes happened on this tick.
    ///
    /// Serialized as a list of pairs of entity chunk and multiple chunks with changed components.
    /// Components are stored in multiple chunks because newly connected clients may need to serialize all components,
    /// while previously connected clients only need the components spawned during this tick.
    ///
    /// For entities, we serialize their count like other data, but for IDs,
    /// we serialize their size in bytes.
    ///
    /// Usually changes stored in [`UpdateMessage`], but if an entity have any insertion or removal,
    /// we serialize it as part of the init message to keep entity changes atomic.
    changes: Vec<(Range<usize>, Vec<Range<usize>>)>,

    /// Visibility of the entity for which changes are being written.
    ///
    /// Updated after [`Self::start_entity_changes`].
    entity_visibility: Visibility,

    /// Indicates that an entity has been written since the
    /// last call of [`Self::start_entity_changes`].
    entity_written: bool,

    /// Intermediate buffer to reuse allocated memory from [`Self::changes`].
    buffer: Vec<Vec<Range<usize>>>,
}

impl InitMessage {
    pub(crate) fn set_mappings(&mut self, mappings: Range<usize>, len: usize) {
        self.mappings = mappings;
        self.mappings_len = len;
    }

    pub(crate) fn add_despawn(&mut self, entity: Range<usize>) {
        self.despawns_len += 1;
        if let Some(last) = self.despawns.last_mut() {
            // Append to previous range if possible.
            if last.end == entity.start {
                last.end = entity.end;
                return;
            }
        }
        self.despawns.push(entity);
    }

    pub(crate) fn add_removals(&mut self, entity: Range<usize>, fn_ids: Range<usize>) {
        self.removals.push((entity, fn_ids));
    }

    /// Updates internal state to start writing changes for an entity with the given visibility.
    ///
    /// Entities and their data written lazily during the iteration.
    /// See [`Self::add_changed_entity`] and [`Self::add_changed_component`].
    pub(crate) fn start_entity_changes(&mut self, visibility: Visibility) {
        self.entity_visibility = visibility;
        self.entity_written = false;
    }

    /// Visibility from the last call of [`Self::start_entity_changes`].
    pub(crate) fn entity_visibility(&self) -> Visibility {
        self.entity_visibility
    }

    /// Returns `true` if [`Self::add_changed_entity`] were called since the last
    /// call of [`Self::start_entity_changes`].
    pub(crate) fn entity_written(&mut self) -> bool {
        self.entity_written
    }

    /// Adds an entity chunk.
    pub(crate) fn add_changed_entity(&mut self, entity: Range<usize>) {
        let components = self.buffer.pop().unwrap_or_default();
        self.changes.push((entity, components));
        self.entity_written = true;
    }

    /// Adds a component chunk to the last added entity from [`Self::add_changed_entity`].
    pub(crate) fn add_changed_component(&mut self, component: Range<usize>) {
        let (_, components) = self
            .changes
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

    /// Takes last changed entity with its component chunks.
    ///
    /// Needs to be called if an entity have any removal or insertion to keep entity updates atomic.
    pub(crate) fn take_changes(&mut self, update_message: &mut UpdateMessage) {
        if update_message.changes_written() {
            let (entity, components_iter) = update_message
                .pop_changes()
                .expect("entity should be written");

            if !self.entity_written {
                let mut components = self.buffer.pop().unwrap_or_default();
                components.extend(components_iter);
                self.changes.push((entity, components));
            } else {
                let (_, components) = self.changes.last_mut().unwrap();
                components.extend(components_iter);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.mappings.is_empty()
            && self.despawns.is_empty()
            && self.removals.is_empty()
            && self.changes.is_empty()
    }

    pub(crate) fn send(
        &self,
        server: &mut RepliconServer,
        client: &ReplicatedClient,
        serialized: &SerializedData,
        server_tick: Range<usize>,
    ) -> bincode::Result<()> {
        let mut arrays = InitMessageArrays::default();
        let mut message_size = size_of::<InitMessageArrays>() + server_tick.len();

        if !self.mappings.is_empty() {
            arrays |= InitMessageArrays::MAPPINGS;
            message_size += self.mappings_len.required_space() + self.mappings.len();
        }
        if !self.despawns.is_empty() {
            arrays |= InitMessageArrays::DESPAWNS;
            message_size += self.despawns_len.required_space();
            message_size += self.despawns.iter().map(|range| range.len()).sum::<usize>();
        }
        if !self.removals.is_empty() {
            arrays |= InitMessageArrays::REMOVALS;
            message_size += self.removals.len().required_space();
            message_size += self
                .removals
                .iter()
                .map(|(entity, components)| {
                    entity.len() + components.len().required_space() + components.len()
                })
                .sum::<usize>();
        }
        if !self.changes.is_empty() {
            arrays |= InitMessageArrays::CHANGES;
            message_size += self.changes.len().required_space();
            message_size += self
                .changes
                .iter()
                .map(|(entity, components)| {
                    let components_size = components.iter().map(|range| range.len()).sum::<usize>();
                    entity.len() + components_size.required_space() + components_size
                })
                .sum::<usize>();
        }

        let mut message = Vec::with_capacity(message_size);

        message.write_fixedint(arrays.bits())?;
        message.extend_from_slice(&serialized[server_tick]);

        if !self.mappings.is_empty() {
            message.write_varint(self.mappings_len)?;
            message.extend_from_slice(&serialized[self.mappings.clone()]);
        }
        if !self.despawns.is_empty() {
            message.write_varint(self.despawns_len)?;
            for range in &self.despawns {
                message.extend_from_slice(&serialized[range.clone()]);
            }
        }
        if !self.removals.is_empty() {
            message.write_varint(self.removals.len())?;
            for (entity, components) in &self.removals {
                message.extend_from_slice(&serialized[entity.clone()]);
                message.write_varint(components.len())?;
                message.extend_from_slice(&serialized[components.clone()]);
            }
        }
        if !self.changes.is_empty() {
            message.write_varint(self.changes.len())?;
            for (entity, components) in &self.changes {
                let components_size = components.iter().map(|range| range.len()).sum::<usize>();
                message.extend_from_slice(&serialized[entity.clone()]);
                message.write_varint(components_size)?;
                for component in components {
                    message.extend_from_slice(&serialized[component.clone()]);
                }
            }
        }

        debug_assert_eq!(message.len(), message_size);

        trace!("sending init message to {:?}", client.id());
        server.send(client.id(), ReplicationChannel::Init, message);

        Ok(())
    }

    /// Clears all chunks.
    ///
    /// Keeps allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.mappings = Default::default();
        self.mappings_len = 0;
        self.despawns.clear();
        self.despawns_len = 0;
        self.removals.clear();
        self.buffer
            .extend(self.changes.drain(..).map(|(_, mut range)| {
                range.clear();
                range
            }));
    }
}
