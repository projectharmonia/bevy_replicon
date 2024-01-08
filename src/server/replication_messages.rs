use std::{
    io::{Cursor, Write},
    mem,
    time::Duration,
};

use bevy::{ecs::component::Tick, prelude::*, ptr::Ptr};
use bevy_renet::renet::{Bytes, ClientId, RenetServer};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintWriter;

use super::{
    clients_info::{ClientBuffers, ClientsInfo},
    ClientInfo, ClientMapping, LastChangeTick,
};
use crate::replicon_core::{
    replication_rules::{ReplicationId, ReplicationInfo},
    replicon_tick::RepliconTick,
    ReplicationChannel,
};

/// Accumulates replication messages and sends them to clients.
///
/// Messages are serialized and deserialized manually because using an intermediate structure
/// leads to allocations and according to our benchmarks it's much slower.
///
/// Reuses allocated memory from older messages.
#[derive(Default)]
pub(crate) struct ReplicationMessages {
    clients_info: ClientsInfo,
    data: Vec<(InitMessage, UpdateMessage)>,
}

impl ReplicationMessages {
    /// Initializes messages for each client.
    ///
    /// Reuses already allocated messages.
    /// Creates new messages if the number of clients is bigger then the number of allocated messages.
    /// If there are more messages than the number of clients, then the extra messages remain untouched
    /// and iteration methods will not include them.
    pub(super) fn prepare(&mut self, clients_info: ClientsInfo) {
        self.data.reserve(clients_info.len());

        for index in 0..clients_info.len() {
            if let Some((init_message, update_message)) = self.data.get_mut(index) {
                init_message.reset();
                update_message.reset();
            } else {
                self.data.push(Default::default());
            }
        }

        self.clients_info = clients_info;
    }

    /// Returns iterator over messages for each client.
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut (InitMessage, UpdateMessage)> {
        self.data.iter_mut().take(self.clients_info.len())
    }

    /// Same as [`Self::iter_mut`], but also iterates over clients info.
    pub(super) fn iter_mut_with_info(
        &mut self,
    ) -> impl Iterator<Item = (&mut InitMessage, &mut UpdateMessage, &mut ClientInfo)> {
        self.data.iter_mut().zip(self.clients_info.iter_mut()).map(
            |((init_message, update_message), client_info)| {
                (init_message, update_message, client_info)
            },
        )
    }

    /// Sends cached messages to clients specified in the last [`Self::prepare`] call.
    ///
    /// Returns the server's last change tick, which will equal the latest replicon tick if any init
    /// messages were sent to clients. If only update messages were sent (or no messages at all) then
    /// it will equal the input `last_change_tick`.
    pub(super) fn send(
        &mut self,
        server: &mut RenetServer,
        client_buffers: &mut ClientBuffers,
        mut last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<(LastChangeTick, ClientsInfo)> {
        if let Some((init_message, _)) = self.data.first() {
            if !init_message.as_slice().is_empty() {
                last_change_tick.0 = replicon_tick;
            }
        }

        for ((init_message, update_message), client_info) in
            self.data.iter_mut().zip(self.clients_info.iter_mut())
        {
            init_message.send(server, replicon_tick, client_info.id())?;
            update_message.send(
                server,
                client_buffers,
                client_info,
                last_change_tick,
                replicon_tick,
                tick,
                timestamp,
            )?;
        }

        let clients_info = mem::take(&mut self.clients_info);

        Ok((last_change_tick, clients_info))
    }
}

/// A reusable message with replicated data.
///
/// Contains tick and mappings, insertions, removals and despawns that
/// happened on this tick.
/// Sent over [`ReplicationChannel::Reliable`] channel.
///
/// See also [Limits](../index.html#limits)
pub(super) struct InitMessage {
    /// Serialized data.
    cursor: Cursor<Vec<u8>>,

    /// Length of the array that updated automatically after writing data.
    array_len: u16,

    /// Position of the array from last call of [`Self::start_array`].
    array_pos: u64,

    /// The number of empty arrays at the end.
    trailing_empty_arrays: usize,

    /// Entity from last call of [`Self::start_entity_data`].
    data_entity: Entity,

    /// Size in bytes of the component data stored for the currently-being-written entity.
    entity_data_size: u16,

    /// Position of entity from last call of [`Self::start_entity_data`].
    entity_data_pos: u64,

    /// Position of entity data length from last call of [`Self::write_data_entity`].
    entity_data_size_pos: u64,
}

impl InitMessage {
    /// Clears the message.
    ///
    /// Keeps allocated capacity for reuse.
    fn reset(&mut self) {
        self.cursor.set_position(0);
        self.trailing_empty_arrays = 0;
    }

    /// Returns size in bytes of the current entity data.
    ///
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn entity_data_size(&self) -> u16 {
        self.entity_data_size
    }

    /// Starts writing array by remembering its position to write length after.
    ///
    /// Arrays can contain entity data or despawns inside.
    /// See also [`Self::end_array`], [`Self::write_client_mapping`], [`Self::write_entity`] and [`Self::start_entity_data`].
    pub(super) fn start_array(&mut self) {
        debug_assert_eq!(self.array_len, 0);

        self.array_pos = self.cursor.position();
        self.cursor
            .set_position(self.array_pos + mem::size_of_val(&self.array_len) as u64);
    }

    /// Ends writing array by writing its length into the last remembered position.
    ///
    /// See also [`Self::start_array`].
    pub(super) fn end_array(&mut self) -> bincode::Result<()> {
        if self.array_len != 0 {
            let previous_pos = self.cursor.position();
            self.cursor.set_position(self.array_pos);

            bincode::serialize_into(&mut self.cursor, &self.array_len)?;

            self.cursor.set_position(previous_pos);
            self.array_len = 0;
            self.trailing_empty_arrays = 0;
        } else {
            self.trailing_empty_arrays += 1;
            self.cursor.set_position(self.array_pos);
            bincode::serialize_into(&mut self.cursor, &self.array_len)?;
        }

        Ok(())
    }

    /// Serializes entity to entity mapping as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`].
    pub(super) fn write_client_mapping(&mut self, mapping: &ClientMapping) -> bincode::Result<()> {
        serialize_entity(&mut self.cursor, mapping.server_entity)?;
        serialize_entity(&mut self.cursor, mapping.client_entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes entity as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// Reuses the serialized data from the buffer from the previous call unless [`ReplicationBuffer::end_write`] is called.
    /// See also [`Self::start_array`].
    pub(super) fn write_entity(&mut self, entity: Entity) -> bincode::Result<()> {
        serialize_entity(&mut self.cursor, entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Starts writing entity and its data as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// Data can contain components with their IDs or IDs only.
    /// Entity will be written lazily after first data write.
    /// See also [`Self::end_entity_data`] and [`Self::write_component`].
    pub(super) fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_size, 0);

        self.data_entity = entity;
        self.entity_data_pos = self.cursor.position();
    }

    /// Writes entity for the current data and remembers the position after it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_data_entity(&mut self) -> bincode::Result<()> {
        serialize_entity(&mut self.cursor, self.data_entity)?;
        self.entity_data_size_pos = self.cursor.position();
        self.cursor.set_position(
            self.entity_data_size_pos + mem::size_of_val(&self.entity_data_size) as u64,
        );

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written unless `save_empty` is set to true.
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`], [`Self::write_component`] and
    /// [`Self::write_component_id`].
    pub(super) fn end_entity_data(&mut self, save_empty: bool) -> bincode::Result<()> {
        if !save_empty && self.entity_data_size == 0 {
            self.cursor.set_position(self.entity_data_pos);
            return Ok(());
        }

        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        self.cursor.set_position(self.entity_data_size_pos);

        bincode::serialize_into(&mut self.cursor, &self.entity_data_size)?;

        self.cursor.set_position(previous_pos);
        self.entity_data_size = 0;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes component and its replication ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_component(
        &mut self,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        DefaultOptions::new().serialize_into(&mut self.cursor, &replication_id)?;
        (replication_info.serialize)(ptr, &mut self.cursor)?;

        let component_size = (self.cursor.position() - previous_pos)
            .try_into()
            .map_err(|_| bincode::ErrorKind::SizeLimit)?;
        self.entity_data_size = self
            .entity_data_size
            .checked_add(component_size)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes replication ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_replication_id(
        &mut self,
        replication_id: ReplicationId,
    ) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        DefaultOptions::new().serialize_into(&mut self.cursor, &replication_id)?;

        let id_size = self.cursor.position() - previous_pos;
        self.entity_data_size = self
            .entity_data_size
            .checked_add(id_size as u16)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Removes entity data elements from update message and copies it.
    ///
    /// Ends entity data for the update message.
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn take_entity_data(
        &mut self,
        update_message: &mut UpdateMessage,
    ) -> bincode::Result<()> {
        if update_message.entity_data_size != 0 {
            let slice = update_message.as_slice();
            let offset = update_message.entity_data_size_pos as usize
                + mem::size_of_val(&update_message.entity_data_size);
            self.cursor.write_all(&slice[offset..]).unwrap();

            self.entity_data_size = self
                .entity_data_size
                .checked_add(update_message.entity_data_size)
                .ok_or(bincode::ErrorKind::SizeLimit)?;
            update_message.entity_data_size = 0;
        }

        update_message
            .cursor
            .set_position(update_message.entity_data_pos);

        Ok(())
    }

    /// Returns the serialized data, excluding trailing empty arrays, as a byte array.
    fn as_slice(&self) -> &[u8] {
        let slice = self.cursor.get_ref();
        let position = self.cursor.position() as usize;
        let extra_len = self.trailing_empty_arrays * mem::size_of_val(&self.array_len);
        &slice[..position - extra_len]
    }

    /// Sends the message, excluding trailing empty arrays, to the specified client.
    ///
    /// Does nothing if there is no data to send.
    fn send(
        &self,
        server: &mut RenetServer,
        replicon_tick: RepliconTick,
        client_id: ClientId,
    ) -> bincode::Result<()> {
        debug_assert_eq!(self.array_len, 0);
        debug_assert_eq!(self.entity_data_size, 0);

        let slice = self.as_slice();
        if slice.is_empty() {
            trace!("no init data to send for client {client_id}");
            return Ok(());
        }

        let mut header = [0; mem::size_of::<RepliconTick>()];
        bincode::serialize_into(&mut header[..], &replicon_tick)?;

        trace!("sending init message to client {client_id}");
        server.send_message(
            client_id,
            ReplicationChannel::Reliable,
            Bytes::from_iter(header.into_iter().chain(slice.iter().copied())),
        );

        Ok(())
    }
}

impl Default for InitMessage {
    fn default() -> Self {
        Self {
            cursor: Default::default(),
            array_len: Default::default(),
            array_pos: Default::default(),
            trailing_empty_arrays: Default::default(),
            entity_data_size: Default::default(),
            entity_data_pos: Default::default(),
            entity_data_size_pos: Default::default(),
            data_entity: Entity::PLACEHOLDER,
        }
    }
}

/// A reusable message with replicated component updates.
///
/// Contains last change tick, current tick and component updates since the last acknowledged tick for each entity.
/// Cannot be applied on the client until the init message matching this update message's last change tick
/// has been applied to the client world.
/// The message will be manually split into packets up to max size, and each packet will be applied
/// independently on the client.
/// Message splits only happen per-entity to avoid weird behavior from partial entity updates.
/// Sent over the [`ReplicationChannel::Unreliable`] channel.
///
/// See also [Limits](../index.html#limits)
pub(super) struct UpdateMessage {
    /// Serialized data.
    cursor: Cursor<Vec<u8>>,

    /// Entities and their sizes in the message with data.
    entities: Vec<(Entity, usize)>,

    /// Entity from last call of [`Self::start_entity_data`].
    data_entity: Entity,

    /// Size in bytes of the component data stored for the currently-being-written entity.
    entity_data_size: u16,

    /// Position of entity from last call of [`Self::start_entity_data`].
    entity_data_pos: u64,

    /// Position of entity data length from last call of [`Self::write_data_entity`].
    entity_data_size_pos: u64,
}

impl UpdateMessage {
    /// Clears the message.
    ///
    /// Keeps allocated capacity for reuse.
    fn reset(&mut self) {
        self.cursor.set_position(0);
        self.entities.clear();
    }

    /// Starts writing entity and its data.
    ///
    /// Data can contain components with their IDs.
    /// Entity will be written lazily after first data write.
    /// See also [`Self::end_entity_data`] and [`Self::write_component`].
    pub(super) fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_size, 0);

        self.data_entity = entity;
        self.entity_data_pos = self.cursor.position();
    }

    /// Writes entity for the current data and remembers the position after it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_data_entity(&mut self) -> bincode::Result<()> {
        serialize_entity(&mut self.cursor, self.data_entity)?;
        self.entity_data_size_pos = self.cursor.position();
        self.cursor.set_position(
            self.entity_data_size_pos + mem::size_of_val(&self.entity_data_size) as u64,
        );

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written.
    /// See also [`Self::start_array`] and [`Self::write_component`].
    pub(super) fn end_entity_data(&mut self) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.cursor.set_position(self.entity_data_pos);
            return Ok(());
        }

        if self.entity_data_size == 0 {
            self.write_data_entity()?;
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

    /// Serializes component and its replication ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_component(
        &mut self,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        DefaultOptions::new().serialize_into(&mut self.cursor, &replication_id)?;
        (replication_info.serialize)(ptr, &mut self.cursor)?;

        let component_size = (self.cursor.position() - previous_pos)
            .try_into()
            .map_err(|_| bincode::ErrorKind::SizeLimit)?;
        self.entity_data_size = self
            .entity_data_size
            .checked_add(component_size)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Returns the serialized data as a byte array.
    fn as_slice(&self) -> &[u8] {
        let slice = self.cursor.get_ref();
        let position = self.cursor.position() as usize;
        &slice[..position]
    }

    /// Splits message according to entities inside it and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    #[allow(clippy::too_many_arguments)]
    fn send(
        &mut self,
        server: &mut RenetServer,
        client_buffers: &mut ClientBuffers,
        client_info: &mut ClientInfo,
        last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<()> {
        debug_assert_eq!(self.entity_data_size, 0);

        let mut slice = self.as_slice();
        if slice.is_empty() {
            trace!("no updates to send for client {}", client_info.id());
            return Ok(());
        }

        trace!("sending update message(s) to client {}", client_info.id());
        const TICKS_SIZE: usize = 2 * mem::size_of::<RepliconTick>();
        let mut header = [0; TICKS_SIZE + mem::size_of::<u16>()];
        bincode::serialize_into(&mut header[..], &(*last_change_tick, replicon_tick))?;

        let mut message_size = 0;
        let client_id = client_info.id();
        let (mut update_index, mut entities) =
            client_info.register_update(client_buffers, tick, timestamp);
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

                server.send_message(
                    client_id,
                    ReplicationChannel::Unreliable,
                    Bytes::from_iter(header.into_iter().chain(message.iter().copied())),
                );

                if !slice.is_empty() {
                    (update_index, entities) =
                        client_info.register_update(client_buffers, tick, timestamp);
                }
            }
        }

        if !slice.is_empty() {
            bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

            server.send_message(
                client_id,
                ReplicationChannel::Unreliable,
                Bytes::from_iter(header.into_iter().chain(slice.iter().copied())),
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
    const MAX_PACKET_SIZE: usize = 1200; // https://github.com/lucaspoffo/renet/blob/acee8b470e34c70d35700d96c00fb233d9cf6919/renet/src/packet.rs#L7

    let dangling = (base + header_size) % MAX_PACKET_SIZE;
    (dangling > 0) && ((dangling + add) <= MAX_PACKET_SIZE)
}

/// Serializes `entity` by writing its index and generation as separate varints.
///
/// The index is first prepended with a bit flag to indicate if the generation
/// is serialized or not (it is not serialized if equal to zero).
fn serialize_entity(cursor: &mut Cursor<Vec<u8>>, entity: Entity) -> bincode::Result<()> {
    let mut flagged_index = (entity.index() as u64) << 1;
    let flag = entity.generation() > 0;
    flagged_index |= flag as u64;

    cursor.write_u64_varint(flagged_index)?;
    if flag {
        cursor.write_u32_varint(entity.generation())?;
    }

    Ok(())
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
