use std::{
    io::{Cursor, Write},
    mem,
};

use bevy::{prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use serde::Serialize;
use varint_rs::VarintWriter;

use crate::{
    replicon_core::replication_rules::{ReplicationId, ReplicationInfo},
    server::ClientMapping,
};

/// A reusable buffer with replicated data.
pub(super) struct ReplicationBuffer {
    /// Serialized data.
    cursor: Cursor<Vec<u8>>,

    /// An indicator of whether the array is currently writing.
    inside_array: bool,

    /// Position of the array from last call of [`Self::start_array`].
    array_pos: u64,

    /// Length of the array that updated automatically after writing data.
    array_len: u16,

    /// The number of arrays excluding empty arrays.
    arrays_with_data: usize,

    /// The number of empty arrays at the end. Can be removed using [`Self::trim_empty_arrays`]
    trailing_empty_arrays: usize,

    /// Position of entity from last call of [`Self::start_entity_data`].
    entity_data_pos: u64,

    /// Position of entity data length from last call of [`Self::write_data_entity`].
    entity_data_len_pos: u64,

    /// Length in bytes of the component data stored for the currently-being-written entity.
    entity_data_len: u16,

    /// Entity from last call of [`Self::start_entity_data`].
    data_entity: Entity,
}

impl ReplicationBuffer {
    /// Clears the buffer.
    ///
    /// Keeps allocated capacity for reuse.
    pub(super) fn reset(&mut self) {
        self.cursor.set_position(0);
        self.arrays_with_data = 0;
        self.trailing_empty_arrays = 0;
    }

    /// Returns the number of arrays excluding empty arrays.
    pub(super) fn arrays_with_data(&self) -> usize {
        self.arrays_with_data
    }

    /// Returns position from the last [`Self::start_entity_data`] call.
    pub(super) fn entity_data_pos(&self) -> u64 {
        self.entity_data_pos
    }

    /// Returns length in bytes of the current entity data.
    ///
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn entity_data_len(&self) -> u16 {
        self.entity_data_len
    }

    /// Returns entity from last call of [`Self::start_entity_data`].
    ///
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn data_entity(&self) -> Entity {
        self.data_entity
    }

    /// Returns the buffer as a byte array.
    pub(super) fn as_slice(&self) -> &[u8] {
        let slice = self.cursor.get_ref();
        let position = self.cursor.position() as usize;
        &slice[..position]
    }

    /// Writes the `value` into the buffer.
    ///
    /// Should happen outside of array or entity data and the buffer shouldn't contain trailing empty arrays.
    /// See also [`Self::start_array`] and [`Self::start_entity_data`].
    pub(super) fn write(&mut self, value: &impl Serialize) -> bincode::Result<()> {
        debug_assert!(!self.inside_array);
        debug_assert_eq!(self.array_len, 0);
        debug_assert_eq!(self.entity_data_len, 0);
        debug_assert_eq!(self.trailing_empty_arrays, 0);

        bincode::serialize_into(&mut self.cursor, value)
    }

    /// Starts writing array by remembering its position to write length after.
    ///
    /// Arrays can contain entity data or despawns inside.
    /// Length will be increased automatically after writing data.
    /// See also [`Self::end_array`], [`Self::write_client_mapping`], [`Self::write_entity`] and [`Self::start_entity_data`].
    pub(super) fn start_array(&mut self) {
        debug_assert_eq!(self.array_len, 0);
        debug_assert!(!self.inside_array);

        self.array_pos = self.cursor.position();
        self.inside_array = true;
        self.cursor
            .set_position(self.array_pos + mem::size_of_val(&self.array_len) as u64);
    }

    /// Ends writing array by writing its length into the last remembered position.
    ///
    /// See also [`Self::start_array`].
    pub(super) fn end_array(&mut self) -> bincode::Result<()> {
        debug_assert!(self.inside_array);

        if self.array_len != 0 {
            let previous_pos = self.cursor.position();
            self.cursor.set_position(self.array_pos);

            bincode::serialize_into(&mut self.cursor, &self.array_len)?;

            self.cursor.set_position(previous_pos);
            self.array_len = 0;
            self.arrays_with_data += 1;
            self.trailing_empty_arrays = 0;
        } else {
            self.trailing_empty_arrays += 1;
            self.cursor.set_position(self.array_pos);
            bincode::serialize_into(&mut self.cursor, &self.array_len)?;
        }
        self.inside_array = false;

        Ok(())
    }

    /// Serializes entity to entity mapping as an array element.
    ///
    /// Should be called only inside array.
    /// Increases array length by 1.
    /// See also [`Self::start_array`].
    pub(super) fn write_client_mapping(&mut self, mapping: &ClientMapping) -> bincode::Result<()> {
        debug_assert!(self.inside_array);

        serialize_entity(&mut self.cursor, mapping.server_entity)?;
        serialize_entity(&mut self.cursor, mapping.client_entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes `entity` as an array element.
    ///
    /// Should be called only inside array.
    /// Increases array length by 1.
    /// See also [`Self::start_array`].
    pub(super) fn write_entity(&mut self, entity: Entity) -> bincode::Result<()> {
        debug_assert!(self.inside_array);

        serialize_entity(&mut self.cursor, entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Starts writing entity and its data.
    ///
    /// Data can contain components with their IDs or IDs only.
    /// Length will be increased automatically after writing data.
    /// Entity will be written lazily after first data write.
    /// Can be called inside and outside of an array.
    /// See also [`Self::end_entity_data`], [`Self::write_component`]
    /// and [`Self::write_component_id`].
    pub(super) fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_len, 0);

        self.data_entity = entity;
        self.entity_data_pos = self.cursor.position();
    }

    /// Writes entity for the current data and remembers the position after it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_data_entity(&mut self) -> bincode::Result<()> {
        serialize_entity(&mut self.cursor, self.data_entity)?;
        self.entity_data_len_pos = self.cursor.position();
        self.cursor.set_position(
            self.entity_data_len_pos + mem::size_of_val(&self.entity_data_len) as u64,
        );

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written unless `save_empty` is set to true.
    /// Increases array length if writing is done inside an array.
    /// See also [`Self::start_array`], [`Self::write_component`] and
    /// [`Self::write_component_id`].
    pub(super) fn end_entity_data(&mut self, save_empty: bool) -> bincode::Result<()> {
        // Abort if empty and unwanted.
        if !save_empty && self.entity_data_len == 0 {
            self.cursor.set_position(self.entity_data_pos);
            return Ok(());
        }

        // Record entity if it has not been written previously.
        if self.entity_data_len == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        self.cursor.set_position(self.entity_data_len_pos);

        bincode::serialize_into(&mut self.cursor, &self.entity_data_len)?;

        self.cursor.set_position(previous_pos);
        self.entity_data_len = 0;
        if self.inside_array {
            self.array_len = self
                .array_len
                .checked_add(1)
                .ok_or(bincode::ErrorKind::SizeLimit)?;
        }

        Ok(())
    }

    /// Serializes `replication_id` and its component from `ptr` as an element of entity data.
    ///
    /// Should be called only inside entity data.
    /// Increases entity data length by 1.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_component(
        &mut self,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        if self.entity_data_len == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        DefaultOptions::new().serialize_into(&mut self.cursor, &replication_id)?;
        (replication_info.serialize)(ptr, &mut self.cursor)?;

        let component_len = (self.cursor.position() - previous_pos)
            .try_into()
            .map_err(|_| bincode::ErrorKind::SizeLimit)?;
        self.entity_data_len = self
            .entity_data_len
            .checked_add(component_len)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes `replication_id` as an element of entity data.
    ///
    /// Should be called only inside entity data.
    /// Increases entity data length by 1.
    /// See also [`Self::start_entity_data`].
    pub(super) fn write_replication_id(
        &mut self,
        replication_id: ReplicationId,
    ) -> bincode::Result<()> {
        if self.entity_data_len == 0 {
            self.write_data_entity()?;
        }

        DefaultOptions::new().serialize_into(&mut self.cursor, &replication_id)?;
        self.entity_data_len += 1;

        Ok(())
    }

    /// Removes entity data elements from `other` and copies it.
    ///
    /// Ends entity data for `other`.
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(super) fn take_entity_data(&mut self, other: &mut Self) {
        if other.entity_data_len != 0 {
            let slice = other.as_slice();
            let offset =
                other.entity_data_len_pos as usize + mem::size_of_val(&other.entity_data_len);
            self.cursor.write_all(&slice[offset..]).unwrap();
            self.entity_data_len += other.entity_data_len;

            other.entity_data_len = 0;
        }

        other.cursor.set_position(other.entity_data_pos);
    }

    /// Crops empty arrays at the end.
    ///
    /// Should only be called after all arrays have been written, because
    /// arrays removed somewhere the middle cannot be detected during deserialization.
    pub(super) fn trim_empty_arrays(&mut self) {
        debug_assert!(!self.inside_array);
        debug_assert_eq!(self.array_len, 0);
        debug_assert_eq!(self.entity_data_len, 0);

        let extra_len = self.trailing_empty_arrays * mem::size_of_val(&self.array_len);
        self.cursor
            .set_position(self.cursor.position() - extra_len as u64);
    }
}

impl Default for ReplicationBuffer {
    fn default() -> Self {
        Self {
            cursor: Default::default(),
            array_pos: Default::default(),
            array_len: Default::default(),
            inside_array: Default::default(),
            arrays_with_data: Default::default(),
            trailing_empty_arrays: Default::default(),
            entity_data_pos: Default::default(),
            entity_data_len_pos: Default::default(),
            entity_data_len: Default::default(),
            data_entity: Entity::PLACEHOLDER,
        }
    }
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
    fn trimming_arrays() -> bincode::Result<()> {
        let mut buffer = ReplicationBuffer::default();

        for _ in 0..3 {
            buffer.start_array();
            buffer.end_array()?;
        }

        buffer.trim_empty_arrays();

        assert!(buffer.as_slice().is_empty());

        Ok(())
    }
}
