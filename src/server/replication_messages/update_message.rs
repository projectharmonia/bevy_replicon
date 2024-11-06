use std::{io::Cursor, mem, time::Duration};

use bevy::{ecs::component::Tick, prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;

use crate::core::{
    channels::ReplicationChannel,
    ctx::SerializeCtx,
    replicated_clients::{ClientBuffers, ReplicatedClient},
    replication_registry::{component_fns::ComponentFns, rule_fns::UntypedRuleFns, FnsId},
    replicon_server::RepliconServer,
    replicon_tick::RepliconTick,
};

/// A reusable message with replicated component updates.
///
/// Contains change tick, current tick and component updates since the last acknowledged tick for each entity.
/// Cannot be applied on the client until the init message matching this update message's change tick
/// has been applied to the client world.
/// The message will be manually split into packets up to max size, and each packet will be applied
/// independently on the client.
/// Message splits only happen per-entity to avoid weird behavior from partial entity updates.
/// Sent over the [`ReplicationChannel::Update`] channel.
///
/// See also [Limits](../index.html#limits)
pub(crate) struct UpdateMessage {
    /// Serialized data.
    pub(super) cursor: Cursor<Vec<u8>>,

    /// Entities and their sizes in the message with data.
    entities: Vec<(Entity, usize)>,

    /// Entity from last call of [`Self::start_entity_data`].
    data_entity: Entity,

    /// Size in bytes of the component data stored for the currently-being-written entity.
    pub(super) entity_data_size: u16,

    /// Position of entity from last call of [`Self::start_entity_data`].
    pub(super) entity_data_pos: u64,

    /// Position of entity data length from last call of [`Self::write_data_entity`].
    pub(super) entity_data_size_pos: u64,
}

impl UpdateMessage {
    /// Clears the message.
    ///
    /// Keeps allocated capacity for reuse.
    pub(super) fn reset(&mut self) {
        self.cursor.set_position(0);
        self.entities.clear();
    }

    /// Starts writing entity and its data.
    ///
    /// Data can contain components with their IDs.
    /// Entity will be written lazily after first data write.
    /// See also [`Self::end_entity_data`] and [`Self::write_component`].
    pub(crate) fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_size, 0);

        self.data_entity = entity;
        self.entity_data_pos = self.cursor.position();
    }

    /// Writes entity for the current data and remembers the position after it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_data_entity(&mut self) -> bincode::Result<()> {
        super::serialize_entity(&mut self.cursor, self.data_entity)?;
        self.entity_data_size_pos = self.cursor.position();
        self.cursor.set_position(
            self.entity_data_size_pos + mem::size_of_val(&self.entity_data_size) as u64,
        );

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written and the cursor will reset.
    /// See also [`Self::start_array`] and [`Self::write_component`].
    pub(crate) fn end_entity_data(&mut self) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.cursor.set_position(self.entity_data_pos);
            return Ok(());
        }

        let previous_pos = self.cursor.position();
        self.cursor.set_position(self.entity_data_size_pos);

        bincode::serialize_into(&mut self.cursor, &self.entity_data_size)?;

        self.cursor.set_position(previous_pos);

        let data_size = self.cursor.position() - self.entity_data_pos;
        self.entities.push((self.data_entity, data_size as usize));

        self.entity_data_size = 0;

        Ok(())
    }

    /// Serializes component and its replication functions ID as an element of entity data.
    ///
    /// Reuses previously shared bytes if they exist, or updates them.
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(crate) fn write_component<'a>(
        &'a mut self,
        shared_bytes: &mut Option<&'a [u8]>,
        rule_fns: &UntypedRuleFns,
        component_fns: &ComponentFns,
        ctx: &SerializeCtx,
        fns_id: FnsId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let size = super::write_with(shared_bytes, &mut self.cursor, |cursor| {
            DefaultOptions::new().serialize_into(&mut *cursor, &fns_id)?;
            // SAFETY: `component_fns`, `ptr` and `rule_fns` were created for the same component type.
            unsafe { component_fns.serialize(ctx, rule_fns, ptr, cursor) }
        })?;

        self.entity_data_size = self
            .entity_data_size
            .checked_add(size)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Returns the serialized data as a byte array.
    pub(super) fn as_slice(&self) -> &[u8] {
        let slice = self.cursor.get_ref();
        let position = self.cursor.position() as usize;
        &slice[..position]
    }

    /// Splits message according to entities inside it and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    pub(super) fn send(
        &mut self,
        server: &mut RepliconServer,
        client_buffers: &mut ClientBuffers,
        client: &mut ReplicatedClient,
        server_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<()> {
        debug_assert_eq!(self.entity_data_size, 0);

        let mut slice = self.as_slice();
        if slice.is_empty() {
            trace!("no updates to send for {:?}", client.id());
            return Ok(());
        }

        trace!("sending update message(s) to {:?}", client.id());
        const TICKS_SIZE: usize = 2 * mem::size_of::<RepliconTick>();
        let mut header = [0; TICKS_SIZE + mem::size_of::<u16>()];
        bincode::serialize_into(&mut header[..], &(client.init_tick(), server_tick))?;

        let mut message_size = 0;
        let client_id = client.id();
        let (mut update_index, mut entities) =
            client.register_update(client_buffers, tick, timestamp);
        for &(entity, data_size) in &self.entities {
            // Try to pack back first, then try to pack forward.
            if message_size == 0
                || can_pack(header.len(), message_size, data_size)
                || can_pack(header.len(), data_size, message_size)
            {
                entities.push(entity);
                message_size += data_size;
            } else {
                let (message, remaining) = slice.split_at(message_size);
                slice = remaining;
                message_size = data_size;

                bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

                server.send(
                    client_id,
                    ReplicationChannel::Update,
                    Bytes::from([&header, message].concat()),
                );

                if !slice.is_empty() {
                    (update_index, entities) =
                        client.register_update(client_buffers, tick, timestamp);
                }
            }
        }

        if !slice.is_empty() {
            bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

            server.send(
                client_id,
                ReplicationChannel::Update,
                Bytes::from([&header, slice].concat()),
            );
        }

        Ok(())
    }
}

impl Default for UpdateMessage {
    fn default() -> Self {
        Self {
            cursor: Default::default(),
            entities: Default::default(),
            entity_data_size: Default::default(),
            entity_data_pos: Default::default(),
            entity_data_size_pos: Default::default(),
            data_entity: Entity::PLACEHOLDER,
        }
    }
}

fn can_pack(header_size: usize, base: usize, add: usize) -> bool {
    const MAX_PACKET_SIZE: usize = 1200; // TODO: make it configurable by the messaging backend.

    let dangling = (base + header_size) % MAX_PACKET_SIZE;
    (dangling > 0) && ((dangling + add) <= MAX_PACKET_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing() {
        assert!(can_pack(10, 0, 5));
        assert!(can_pack(10, 0, 1190));
        assert!(!can_pack(10, 0, 1191));
        assert!(!can_pack(10, 0, 3000));

        assert!(can_pack(10, 1189, 1));
        assert!(!can_pack(10, 1190, 0));
        assert!(!can_pack(10, 1190, 1));
        assert!(!can_pack(10, 1190, 3000));
    }
}
