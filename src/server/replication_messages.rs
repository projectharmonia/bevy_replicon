use std::{
    io::Cursor,
    mem,
    ops::{DerefMut, Range},
    time::Duration,
};

use bevy::{ecs::component::Tick, prelude::*, ptr::Ptr};
use bevy_renet::renet::{Bytes, ClientId, RenetServer};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintWriter;

use super::{
    clients_info::{ClientBuffers, ClientsInfo},
    replication_buffer::ReplicationBuffer,
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn send(
        &mut self,
        buffer: &mut ReplicationBuffer,
        server: &mut RenetServer,
        client_buffers: &mut ClientBuffers,
        mut last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<(LastChangeTick, ClientsInfo)> {
        if let Some((init_message, _)) = self.data.first() {
            if !init_message.is_empty() {
                last_change_tick.0 = replicon_tick;
            }
        }

        for ((init_message, update_message), client_info) in
            self.data.iter_mut().zip(self.clients_info.iter_mut())
        {
            init_message.send(buffer, server, replicon_tick, client_info.id())?;
            update_message.send(
                buffer,
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
#[derive(Default)]
pub(super) struct InitMessage {
    /// Slices of serialized data in form of ranges that points to [`ReplicationBuffer`].
    ranges: Vec<Range<usize>>,

    /// Index of range from the last call of [`Self::start_array`].
    array_index: usize,

    /// Length of the array that updated automatically after writing data.
    array_len: usize,

    /// The number of empty arrays at the end. Can be removed using [`Self::trim_empty_arrays`]
    trailing_empty_arrays: usize,

    /// Index of range from the last call of [`Self::start_entity_data`].
    entity_data_index: usize,

    /// Size in bytes of the component data stored for the currently-being-written entity.
    entity_data_size: usize,
}

impl InitMessage {
    /// Clears the message and assigns tick to it.
    ///
    /// Keeps allocated capacity for reuse.
    fn reset(&mut self) {
        self.ranges.clear();
        self.array_index = 0;
        self.array_len = 0;
        self.trailing_empty_arrays = 0;
        self.entity_data_index = 0;
        self.entity_data_size = 0;
    }

    /// Returns size in bytes of the current entity data.
    ///
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn entity_data_size(&self) -> usize {
        self.entity_data_size
    }

    /// Starts writing array by preallocating a range for it.
    ///
    /// Arrays can contain entity data or despawns inside.
    /// See also [`Self::end_array`], [`Self::write_client_mapping`], [`Self::write_entity`] and [`Self::start_entity_data`].
    pub(super) fn start_array(&mut self) {
        self.ranges.push(Default::default());
        self.array_index = self.ranges.len() - 1;
    }

    /// Ends writing array by writing its length and updating the preallocated range.
    ///
    /// See also [`Self::start_array`].
    pub(super) fn end_array(&mut self, buffer: &mut ReplicationBuffer) -> bincode::Result<()> {
        let range =
            buffer.write(|cursor| DefaultOptions::new().serialize_into(cursor, &self.array_len))?;

        self.ranges[self.array_index] = range;
        if self.array_len != 0 {
            self.array_len = 0;
            self.trailing_empty_arrays = 0;
        } else {
            self.trailing_empty_arrays += 1;
        }

        Ok(())
    }

    /// Serializes entity to entity mapping as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`].
    pub(super) fn write_client_mapping(
        &mut self,
        buffer: &mut ReplicationBuffer,
        mapping: &ClientMapping,
    ) -> bincode::Result<()> {
        let range = buffer.write(|cursor| {
            serialize_entity(cursor, mapping.server_entity)?;
            serialize_entity(cursor, mapping.client_entity)
        })?;
        self.ranges.push(range);
        self.array_len += 1;

        Ok(())
    }

    /// Serializes `entity` as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`].
    pub(super) fn write_entity(
        &mut self,
        buffer: &mut ReplicationBuffer,
        entity: Entity,
    ) -> bincode::Result<()> {
        let range = buffer.get_or_write(|cursor| serialize_entity(cursor, entity))?;
        self.ranges.push(range);
        self.array_len += 1;

        Ok(())
    }

    /// Starts writing entity and its data size as an array element by preallocating a range for it.
    ///
    /// Data can contain components with their IDs or IDs only.
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::end_entity_data`], [`Self::write_component`]
    /// and [`Self::write_component_id`].
    pub(super) fn start_entity_data(&mut self) {
        self.ranges.push(Default::default());
        self.entity_data_index = self.ranges.len() - 1;
    }

    /// Ends writing entity data by writing the entity with its size and updating the preallocated range.
    ///
    /// If the entity data is empty, nothing will be written unless `save_empty` is set to true.
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`], [`Self::write_component`] and
    /// [`Self::write_component_id`].
    pub(super) fn end_entity_data(
        &mut self,
        buffer: &mut ReplicationBuffer,
        entity: Entity,
        save_empty: bool,
    ) -> bincode::Result<()> {
        if !save_empty && self.entity_data_size == 0 {
            self.ranges.pop();
            return Ok(());
        }

        let range = buffer.write(|cursor| {
            serialize_entity(cursor, entity)?;
            DefaultOptions::new().serialize_into(cursor, &self.entity_data_size)
        })?;

        self.ranges[self.entity_data_index] = range;
        self.entity_data_size = 0;
        self.array_len += 1;

        Ok(())
    }

    /// Serializes component and its replication ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_component(
        &mut self,
        buffer: &mut ReplicationBuffer,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        let range = buffer.get_or_write(|mut cursor| {
            DefaultOptions::new().serialize_into(cursor.deref_mut(), &replication_id)?;
            (replication_info.serialize)(ptr, cursor)
        })?;
        self.entity_data_size += range.end - range.start;
        self.ranges.push(range);

        Ok(())
    }

    /// Serializes replication ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_replication_id(
        &mut self,
        buffer: &mut ReplicationBuffer,
        replication_id: ReplicationId,
    ) -> bincode::Result<()> {
        let range = buffer
            .get_or_write(|cursor| DefaultOptions::new().serialize_into(cursor, &replication_id))?;
        self.entity_data_size += range.end - range.start;

        self.ranges.push(range);

        Ok(())
    }

    /// Takes all ranges related to entity data from the update message.
    ///
    /// Ends entity data for the update message.
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn take_entity_data(&mut self, update_message: &mut UpdateMessage) {
        self.ranges.extend(
            update_message
                .ranges
                .drain(update_message.entity_data_index..)
                .skip(1), // Drop preallocated range for entity size.
        );
        self.entity_data_size += update_message.entity_data_size;
        update_message.entity_data_size = 0;
    }

    /// Returns `true` is message contains any written data.
    fn is_empty(&self) -> bool {
        self.ranges.len() - self.trailing_empty_arrays == 0
    }

    /// Trims empty arrays from the message and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    fn send(
        &self,
        buffer: &mut ReplicationBuffer,
        server: &mut RenetServer,
        replicon_tick: RepliconTick,
        client_id: ClientId,
    ) -> bincode::Result<()> {
        if self.is_empty() {
            trace!("no init data to send for client {client_id}");
            return Ok(());
        }

        let mut header = [0; mem::size_of::<RepliconTick>()];
        bincode::serialize_into(&mut header[..], &replicon_tick)?;
        let ranges = &self.ranges[..self.ranges.len() - self.trailing_empty_arrays];

        trace!("sending init message to client {client_id}");
        server.send_message(
            client_id,
            ReplicationChannel::Reliable,
            Bytes::from_iter(header.into_iter().chain(buffer.iter_ranges(ranges))),
        );

        Ok(())
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
#[derive(Default)]
pub(super) struct UpdateMessage {
    /// Entities and their sizes in the message with data.
    entities: Vec<(Entity, usize)>,

    /// Message data.
    ranges: Vec<Range<usize>>,

    /// Index of range from the last call of [`Self::start_entity_data`].
    entity_data_index: usize,

    /// Size in bytes of the component data stored for the currently-being-written entity.
    entity_data_size: usize,
}

impl UpdateMessage {
    /// Clears the message.
    ///
    /// Keeps allocated capacity for reuse.
    fn reset(&mut self) {
        self.entities.clear();
        self.ranges.clear();
        self.entity_data_index = 0;
        self.entity_data_size = 0;
    }

    /// Starts writing entity and its data size as an array element by preallocating a range for it.
    ///
    /// Data can contain components with their IDs.
    /// See also [`Self::end_entity_data`], [`Self::write_component`].
    pub(super) fn start_entity_data(&mut self) {
        self.ranges.push(Default::default());
        self.entity_data_index = self.ranges.len() - 1;
    }

    /// Ends writing entity data by writing the entity with its size and updating the preallocated range.
    ///
    /// If the entity data is empty, nothing will be written.
    /// See also [`Self::start_array`], [`Self::write_component`].
    pub(super) fn end_entity_data(
        &mut self,
        buffer: &mut ReplicationBuffer,
        entity: Entity,
    ) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.ranges.pop();
            return Ok(());
        }

        let range = buffer.write(|cursor| {
            serialize_entity(cursor, entity)?;
            DefaultOptions::new().serialize_into(cursor, &self.entity_data_size)
        })?;
        self.entities
            .push((entity, range.end - range.start + self.entity_data_size));
        self.ranges[self.entity_data_index] = range;
        self.entity_data_size = 0;

        Ok(())
    }

    /// Serializes component and its replication ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_component(
        &mut self,
        buffer: &mut ReplicationBuffer,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        let range = buffer.get_or_write(|mut cursor| {
            DefaultOptions::new().serialize_into(cursor.deref_mut(), &replication_id)?;
            (replication_info.serialize)(ptr, cursor)
        })?;
        self.entity_data_size += range.end - range.start;
        self.ranges.push(range);

        Ok(())
    }

    /// Returns `true` is message contains any written data.
    fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Splits message according to entities inside it and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    #[allow(clippy::too_many_arguments)]
    fn send(
        &self,
        buffer: &mut ReplicationBuffer,
        server: &mut RenetServer,
        client_buffers: &mut ClientBuffers,
        client_info: &mut ClientInfo,
        last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<()> {
        if self.is_empty() {
            trace!("no updates to send for client {}", client_info.id());
            return Ok(());
        }

        trace!("sending update message(s) to client {}", client_info.id());
        const TICKS_SIZE: usize = 2 * mem::size_of::<RepliconTick>();
        let mut header = [0; TICKS_SIZE + mem::size_of::<u16>()];
        bincode::serialize_into(&mut header[..], &(*last_change_tick, replicon_tick))?;

        let mut iter = buffer.iter_ranges(&self.ranges).peekable();
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
                let message_iter = iter.by_ref().take(message_size);
                message_size = data_size;

                bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

                server.send_message(
                    client_id,
                    ReplicationChannel::Unreliable,
                    Bytes::from_iter(header.into_iter().chain(message_iter)),
                );

                if iter.peek().is_some() {
                    (update_index, entities) =
                        client_info.register_update(client_buffers, tick, timestamp);
                }
            }
        }

        if iter.peek().is_some() {
            bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

            server.send_message(
                client_id,
                ReplicationChannel::Unreliable,
                Bytes::from_iter(header.into_iter().chain(iter)),
            );
        }

        Ok(())
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
