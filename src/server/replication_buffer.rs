use std::{io::Cursor, mem};

use bevy::{ecs::component::Tick, prelude::*, ptr::Ptr};
use bevy_renet::renet::{Bytes, RenetServer};
use bincode::{DefaultOptions, Options};
use varint_rs::{VarintReader, VarintWriter};

use crate::replicon_core::{
    replication_rules::{ReplicationId, ReplicationInfo},
    replicon_tick::RepliconTick,
};

/*
/// Packet headers contain a RepliconTick, and some flags, masked into a u64, written as varint.
pub(super) struct PacketHeader {
    /// does this packet contain an array at the start of client entity mappings?
    /// if so, expect an array of [(ServerEntity, ClientEntity)]
    client_entity_mappings_flag: bool,
    replicon_tick: RepliconTick,
}

impl PacketHeader {
    fn write(&self, message: &mut Cursor<Vec<u8>>) -> Result<(), bincode::Error> {
        let mut flagged_tick = (self.replicon_tick.get() as u64) << 1;
        flagged_tick |= self.client_entity_mappings_flag as u64;

        message.write_u64_varint(flagged_tick)?;
        Ok(())
    }
    fn read(cursor: &mut Cursor<Bytes>) -> Result<PacketHeader, bincode::Error> {
        let flagged_tick = cursor.read_u64_varint()?;
        let client_entity_mappings_flag = flagged_tick & 1 > 0;
        let tick_value = (flagged_tick >> 1) as u32;
        let replicon_tick = RepliconTick(tick_value);
        Ok(PacketHeader {
            client_entity_mappings_flag,
            replicon_tick,
        })
    }
}
*/

/// A reusable buffer with replicated data for a client.
///
/// See also [Limits](../index.html#limits)
pub(crate) struct ReplicationBuffer {
    /// ID of a client for which this buffer is written.
    client_id: u64,

    /// Last system tick acknowledged by the client.
    ///
    /// Used for changes preparation.
    system_tick: Tick,

    /// Buffer with serialized data.
    message: Cursor<Vec<u8>>,

    /// Position of the array from last call of [`Self::start_array`].
    array_pos: u64,

    /// Length of the array that updated automatically after writing data.
    array_len: u16,

    /// The number of non-empty arrays stored.
    arrays_with_data: usize,

    /// The number of empty arrays at the end. Can be removed using [`Self::trim_empty_arrays`]
    trailing_empty_arrays: usize,

    /// Position of entity after [`Self::start_entity_data`] or its data after [`Self::write_data_entity`].
    entity_data_pos: u64,

    /// Length of the data for entity that updated automatically after writing data.
    entity_data_len: u8,

    /// Entity from last call of [`Self::start_entity_data`].
    data_entity: Entity,
}

impl ReplicationBuffer {
    /// Creates a new buffer with assigned client ID and acknowledged system tick
    /// and writes current server tick into buffer data.
    pub(super) fn new(
        client_id: u64,
        system_tick: Tick,
        replicon_tick: RepliconTick,
    ) -> Result<Self, bincode::Error> {
        let mut result = Self {
            client_id,
            system_tick,
            message: Default::default(),
            array_pos: Default::default(),
            array_len: Default::default(),
            arrays_with_data: Default::default(),
            trailing_empty_arrays: Default::default(),
            entity_data_pos: Default::default(),
            entity_data_len: Default::default(),
            data_entity: Entity::PLACEHOLDER,
        };
        result.write_replicon_tick(&replicon_tick)?;
        Ok(result)
    }

    #[inline]
    fn write_replicon_tick(&mut self, replicon_tick: &RepliconTick) -> Result<(), bincode::Error> {
        bincode::serialize_into(&mut self.message, &replicon_tick)?;
        Ok(())
    }

    #[inline]
    pub(crate) fn read_replicon_tick(
        cursor: &mut Cursor<Bytes>,
    ) -> Result<RepliconTick, bincode::Error> {
        let tick = bincode::deserialize_from(cursor)?;
        Ok(tick)
    }

    #[inline]
    pub(crate) fn client_id(&self) -> u64 {
        self.client_id
    }

    /// Read access to the buffer's system tick (this client's last acked replicon tick).
    pub(super) fn system_tick(&self) -> Tick {
        self.system_tick
    }

    /// Reassigns current client ID and acknowledged system tick to the buffer
    /// and replaces buffer data with current server tick.
    ///
    /// Keeps allocated capacity of the buffer data.
    pub(super) fn reset(
        &mut self,
        client_id: u64,
        system_tick: Tick,
        replicon_tick: RepliconTick,
    ) -> Result<(), bincode::Error> {
        self.client_id = client_id;
        self.system_tick = system_tick;
        self.message.set_position(0);
        self.message.get_mut().clear();
        self.arrays_with_data = 0;
        self.trailing_empty_arrays = 0;

        self.write_replicon_tick(&replicon_tick)?;

        Ok(())
    }

    /// Writes array of entity->entity mappings
    ///
    /// Includes a marker byte to say whether the array was written or not.
    pub(super) fn write_entity_mappings(
        &mut self,
        mappings: Option<Vec<(Entity, Entity)>>,
    ) -> Result<(), bincode::Error> {
        self.start_array();
        if let Some(mappings) = mappings {
            for (server_entity, client_entity) in &mappings {
                self.write_entity(*server_entity)?;
                self.write_entity(*client_entity)?;
                self.array_len = self
                    .array_len
                    .checked_add(1)
                    .ok_or(bincode::ErrorKind::SizeLimit)?;
            }
        }
        self.end_array()?;
        Ok(())
    }

    /// Reads entity mapping list
    ///
    /// Static counterpart of `write_entity_mappings()`
    pub(crate) fn read_entity_mappings(
        cursor: &mut Cursor<Bytes>,
    ) -> Result<Vec<(Entity, Entity)>, bincode::Error> {
        let mut mappings = Vec::new();
        let array_len: u16 = bincode::deserialize_from(&mut *cursor)?;
        for _ in 0..array_len {
            let server_entity = Self::read_entity(cursor)?;
            let client_entity = Self::read_entity(cursor)?;
            mappings.push((server_entity, client_entity));
        }
        Ok(mappings)
    }

    /// Starts writing array by remembering its position to write length after.
    ///
    /// Arrays can contain entity data or despawns inside.
    /// Length will be increased automatically after writing data.
    /// See also [`Self::end_array`], [`Self::start_entity_data`] and [`Self::write_despawn`].
    pub(super) fn start_array(&mut self) {
        debug_assert_eq!(self.array_len, 0);

        self.array_pos = self.message.position();
        self.message
            .set_position(self.array_pos + mem::size_of_val(&self.array_len) as u64);
    }

    /// Ends writing array by writing its length into the last remembered position.
    ///
    /// See also [`Self::start_array`].
    pub(super) fn end_array(&mut self) -> Result<(), bincode::Error> {
        if self.array_len != 0 {
            let previous_pos = self.message.position();
            self.message.set_position(self.array_pos);

            bincode::serialize_into(&mut self.message, &self.array_len)?;

            self.message.set_position(previous_pos);
            self.array_len = 0;
            self.arrays_with_data += 1;
            self.trailing_empty_arrays = 0;
        } else {
            self.trailing_empty_arrays += 1;
            self.message.set_position(self.array_pos);
            bincode::serialize_into(&mut self.message, &self.array_len)?;
        }

        Ok(())
    }

    /// Starts writing entity and its data by remembering `entity`.
    ///
    /// Arrays can contain component changes or removals inside.
    /// Length will be increased automatically after writing data.
    /// Entity will be written lazily after first data write and its position will be remembered to write length later.
    /// See also [`Self::end_entity_data`], [`Self::write_current_entity`], [`Self::write_change`]
    /// and [`Self::write_removal`].
    pub(super) fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_len, 0);

        self.data_entity = entity;
        self.entity_data_pos = self.message.position();
    }

    /// Writes entity for current data and updates remembered position for it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_data_entity(&mut self) -> Result<(), bincode::Error> {
        self.write_entity(self.data_entity)?;

        self.entity_data_pos = self.message.position();
        self.message
            .set_position(self.entity_data_pos + mem::size_of_val(&self.entity_data_len) as u64);

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written.
    /// See also [`Self::start_array`], [`Self::write_current_entity`], [`Self::write_change`] and
    /// [`Self::write_removal`].
    pub(super) fn end_entity_data(&mut self) -> Result<(), bincode::Error> {
        if self.entity_data_len != 0 {
            let previous_pos = self.message.position();
            self.message.set_position(self.entity_data_pos);

            bincode::serialize_into(&mut self.message, &self.entity_data_len)?;

            self.message.set_position(previous_pos);
            self.entity_data_len = 0;
            self.array_len = self
                .array_len
                .checked_add(1)
                .ok_or(bincode::ErrorKind::SizeLimit)?;
        } else {
            self.message.set_position(self.entity_data_pos);
        }

        Ok(())
    }

    /// Serializes `replication_id` and component from `ptr` into the buffer data.
    ///
    /// Should be called only inside entity data.
    /// Increases entity data length by 1.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_change(
        &mut self,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> Result<(), bincode::Error> {
        if self.entity_data_len == 0 {
            self.write_data_entity()?;
        }

        DefaultOptions::new().serialize_into(&mut self.message, &replication_id)?;
        (replication_info.serialize)(ptr, &mut self.message)?;
        self.entity_data_len += 1;

        Ok(())
    }

    /// Serializes `replication_id` of the removed component into the buffer data.
    ///
    /// Should be called only inside entity data.
    /// Increases entity data length by 1.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_removal(
        &mut self,
        replication_id: ReplicationId,
    ) -> Result<(), bincode::Error> {
        if self.entity_data_len == 0 {
            self.write_data_entity()?;
        }

        DefaultOptions::new().serialize_into(&mut self.message, &replication_id)?;
        self.entity_data_len += 1;

        Ok(())
    }

    /// Serializes despawned `entity`.
    ///
    /// Should be called only inside array.
    /// Increases array length by 1.
    /// See also [`Self::start_array`].
    pub(super) fn write_despawn(&mut self, entity: Entity) -> Result<(), bincode::Error> {
        self.write_entity(entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes `entity` by writing its index and generation as separate varints.
    ///
    /// The index is first prepended with a bit flag to indicate if the generation
    /// is serialized or not (it is not serialized if equal to zero).
    pub(crate) fn write_entity(&mut self, entity: Entity) -> Result<(), bincode::Error> {
        let mut flagged_index = (entity.index() as u64) << 1;
        let flag = entity.generation() > 0;
        flagged_index |= flag as u64;

        self.message.write_u64_varint(flagged_index)?;
        if flag {
            self.message.write_u32_varint(entity.generation())?;
        }

        Ok(())
    }

    /// Deserializes `entity` from compressed index and generation
    /// Static counterpart to [`write_entity()`].
    pub(crate) fn read_entity(cursor: &mut Cursor<Bytes>) -> Result<Entity, bincode::Error> {
        let flagged_index: u64 = cursor.read_u64_varint()?;
        let has_generation = (flagged_index & 1) > 0;
        let generation = if has_generation {
            cursor.read_u32_varint()?
        } else {
            0u32
        };

        let bits = (generation as u64) << 32 | (flagged_index >> 1);

        Ok(Entity::from_bits(bits))
    }

    /// Send the buffer contents into a renet server channel.
    ///
    /// [`Self::reset`] should be called after it to use this buffer again.
    pub(super) fn send_to(&mut self, server: &mut RenetServer, replication_channel_id: u8) {
        debug_assert_eq!(self.array_len, 0);
        debug_assert_eq!(self.entity_data_len, 0);

        if self.arrays_with_data > 0 {
            self.trim_empty_arrays();

            server.send_message(
                self.client_id,
                replication_channel_id,
                Bytes::copy_from_slice(self.message.get_ref()),
            );
        }
    }

    /// Crops empty arrays at the end.
    ///
    /// Should only be called after all arrays have been written, because
    /// removed array somewhere the middle cannot be detected during deserialization.
    fn trim_empty_arrays(&mut self) {
        let used_len = self.message.get_ref().len()
            - self.trailing_empty_arrays * mem::size_of_val(&self.array_len);
        self.message.get_mut().truncate(used_len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_empty_arrays() -> Result<(), bincode::Error> {
        let mut buffer = ReplicationBuffer::new(0, Tick::new(0), RepliconTick(0))?;

        let begin_len = buffer.message.get_ref().len();
        for _ in 0..3 {
            buffer.start_array();
            buffer.end_array()?;
        }

        buffer.trim_empty_arrays();

        assert_eq!(buffer.message.get_ref().len(), begin_len);

        Ok(())
    }

    #[test]
    fn mappings() -> Result<(), bincode::Error> {
        let mut buffer = ReplicationBuffer::new(0, Tick::new(0), RepliconTick(0))?;
        let mappings = vec![(Entity::PLACEHOLDER, Entity::PLACEHOLDER)];
        buffer.write_entity_mappings(Some(mappings.clone()))?;

        let slice: &[u8] = buffer.message.get_ref();
        let mut cursor = Cursor::new(Bytes::copy_from_slice(slice));
        // skip the 4 byte replicon tick at the start of the buffer
        let _ = ReplicationBuffer::read_replicon_tick(&mut cursor)?;
        let received_mappings = ReplicationBuffer::read_entity_mappings(&mut cursor)?;
        assert_eq!(mappings, received_mappings);

        Ok(())
    }
}
