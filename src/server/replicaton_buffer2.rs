use std::{io::Cursor, ops::Range};

use bevy::{prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintWriter;

use super::ClientMapping;
use crate::replicon_core::{
    replication_rules::{ReplicationId, ReplicationInfo},
    replicon_tick::RepliconTick,
};

/// A reusable buffer with replicated data.
#[derive(Default)]
struct ReplicationBuffer {
    /// Serialized data.
    cursor: Cursor<Vec<u8>>,

    /// Index from the last call of [`Self::start_range`].
    range_start: Option<usize>,
}

impl ReplicationBuffer {
    /// Clears the buffer.
    ///
    /// Keeps allocated capacity for reuse.
    fn clear(&mut self) {
        self.cursor.set_position(0);
        self.range_start = None;
    }

    /// Returns current position inside the buffer.
    fn position(&self) -> usize {
        self.cursor.position() as usize
    }

    /// Remembers current position as the position for writing new data.
    ///
    /// See also [`Self::current_range`] and [`Self::end_range`].
    #[must_use]
    fn start_range(&mut self) -> usize {
        self.range_start = Some(self.position());
        self.position()
    }

    /// Returns the range from the last remembered position to the current one.
    ///
    /// When the range is exists, [`ReplicationSlices`] will use it instead of writing new data.
    /// See also [`Self::start_range`] and [`Self::end_range`].
    fn current_range(&self) -> Option<Range<usize>> {
        self.range_start.map(|begin| begin..self.position())
    }

    /// Clears last remembered position for data.
    ///
    /// See also [`Self::start_range`] and [`Self::current_range`].
    pub(super) fn end_range(&mut self) {
        self.range_start = None;
    }
}

/// Slices of data written into [`ReplicationBuffer`] represented in form of ranges.
#[derive(Default)]
struct ReplicationSlices {
    /// Ranges of data from the buffer.
    ranges: Vec<Range<usize>>,

    /// Index of range from the last call of [`Self::start_array`].
    array_index: usize,

    /// The number of empty arrays at the end. Can be removed using [`Self::trim_empty_arrays`]
    trailing_empty_arrays: usize,

    /// Index of range from the last call of [`Self::start_entity_data`].
    entity_data_index: usize,
}

impl ReplicationSlices {
    /// Returns number of stored ranges.
    pub(super) fn ranges_count(&self) -> usize {
        self.ranges.len()
    }

    /// Clears all ranges.
    ///
    /// Keeps allocated capacity for reuse.
    pub(super) fn clear(&mut self) {
        self.ranges.clear();
        self.trailing_empty_arrays = 0;
    }

    /// Returns an iterator over slices data from the buffer.
    pub(super) fn iter<'a>(
        &'a self,
        buffer: &'a ReplicationBuffer,
    ) -> impl Iterator<Item = u8> + 'a {
        self.ranges
            .iter()
            .flat_map(|range| &buffer.cursor.get_ref()[range.clone()])
            .copied()
    }

    /// Starts writing array by preallocating a range for it.
    ///
    /// See also [`Self::end_array`].
    pub(super) fn start_array(&mut self) {
        self.ranges.push(Default::default());
        self.array_index = self.ranges.len() - 1;
    }

    /// Ends writing array by writing its length and updating the preallocated range.
    ///
    /// See also [`Self::start_array`].
    pub(super) fn end_array(
        &mut self,
        buffer: &mut ReplicationBuffer,
        array_len: u16,
    ) -> bincode::Result<()> {
        let begin = buffer.position();
        bincode::serialize_into(&mut buffer.cursor, &array_len)?;
        let end = buffer.position();

        self.ranges[self.array_index] = begin..end;
        if array_len != 0 {
            self.trailing_empty_arrays = 0;
        } else {
            self.trailing_empty_arrays += 1;
        }

        Ok(())
    }

    /// Starts writing entity and its data length by preallocating a range for it.
    ///
    /// Later either [`Self::end_entity_data`] or [`Self::cancel_entity_data`] should be called.
    pub(super) fn start_entity_data(&mut self) {
        self.ranges.push(Default::default());
        self.entity_data_index = self.ranges.len() - 1;
    }

    /// Ends writing entity data by writing the entity with its length and updating the preallocated range.
    ///
    /// Should be called only after [`Self::start_entity_data`].
    /// See also [`Self::cancel_entity_data`].
    pub(super) fn end_entity_data(
        &mut self,
        buffer: &mut ReplicationBuffer,
        entity: Entity,
        data_len: u16,
    ) -> bincode::Result<()> {
        let begin = buffer.position();
        serialize_entity(&mut buffer.cursor, entity)?;
        bincode::serialize_into(&mut buffer.cursor, &data_len)?;
        let end = buffer.position();

        self.ranges[self.entity_data_index] = begin..end;

        Ok(())
    }

    /// Removes last preallocated range that was created for an entity data.
    ///
    /// Should be called only after [`Self::start_entity_data`].
    /// See also [`Self::end_entity_data`].
    pub(super) fn cancel_entity_data(&mut self) {
        debug_assert_eq!(self.entity_data_index, self.ranges.len() - 1);

        self.ranges.pop();
    }

    /// Serializes replicon tick into the buffer or reuses previously written range if exists.
    pub(super) fn write_tick(
        &mut self,
        buffer: &mut ReplicationBuffer,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        if let Some(range) = buffer.current_range() {
            self.extend_or_push(range);
        } else {
            let begin = buffer.start_range();
            bincode::serialize_into(&mut buffer.cursor, &replicon_tick)?;
            let end = buffer.position();
            self.ranges.push(begin..end);
        }

        Ok(())
    }

    /// Serializes entity to entity mapping into the buffer or reuses previously written range if exists.
    pub(super) fn write_client_mapping(
        &mut self,
        buffer: &mut ReplicationBuffer,
        mapping: &ClientMapping,
    ) -> bincode::Result<()> {
        if let Some(range) = buffer.current_range() {
            self.extend_or_push(range);
        } else {
            let begin = buffer.start_range();
            serialize_entity(&mut buffer.cursor, mapping.server_entity)?;
            serialize_entity(&mut buffer.cursor, mapping.client_entity)?;
            let end = buffer.position();
            self.ranges.push(begin..end);
        }

        Ok(())
    }

    /// Serializes entity into the buffer or reuses previously written range if exists.
    pub(super) fn write_entity(
        &mut self,
        buffer: &mut ReplicationBuffer,
        entity: Entity,
    ) -> bincode::Result<()> {
        if let Some(range) = buffer.current_range() {
            self.extend_or_push(range);
        } else {
            let begin = buffer.start_range();
            serialize_entity(&mut buffer.cursor, entity)?;
            let end = buffer.position();
            self.ranges.push(begin..end);
        }

        Ok(())
    }

    /// Serializes component and its replication ID into the buffer or reuses previously written range if exists.
    pub(super) fn write_component(
        &mut self,
        buffer: &mut ReplicationBuffer,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        if let Some(range) = buffer.current_range() {
            self.extend_or_push(range);
        } else {
            let begin = buffer.start_range();
            DefaultOptions::new().serialize_into(&mut buffer.cursor, &replication_id)?;
            (replication_info.serialize)(ptr, &mut buffer.cursor)?;
            let end = buffer.position();
            self.ranges.push(begin..end);
        }

        Ok(())
    }

    /// Serializes component replication ID into the buffer or reuses previously written range if exists.
    pub(super) fn write_replication_id(
        &mut self,
        buffer: &mut ReplicationBuffer,
        replication_id: ReplicationId,
    ) -> bincode::Result<()> {
        if let Some(range) = buffer.current_range() {
            self.extend_or_push(range);
        } else {
            let begin = buffer.start_range();
            DefaultOptions::new().serialize_into(&mut buffer.cursor, &replication_id)?;
            let end = buffer.position();
            self.ranges.push(begin..end);
        }

        Ok(())
    }

    /// Extends the last range if its end position is equal to the start of a new one, or inserts a new range.
    fn extend_or_push(&mut self, range: Range<usize>) {
        match self.ranges.last_mut() {
            Some(last_range) if last_range.end == range.start => {
                last_range.end = range.end;
            }
            _ => self.ranges.push(range),
        }
    }

    /// Takes all ranges related to entity data from `other`.
    pub(super) fn take_entity_data(&mut self, other: &mut Self) {
        self.ranges
            .extend(other.ranges.drain(other.entity_data_index..));
    }

    /// Crops empty arrays at the end.
    ///
    /// Should only be called after all arrays have been written, because
    /// arrays removed somewhere the middle cannot be detected during deserialization.
    pub(super) fn trim_empty_arrays(&mut self) {
        self.ranges
            .truncate(self.ranges.len() - self.trailing_empty_arrays);
    }
}

/// Serializes entity by writing its index and generation as separate varints.
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
        let mut buffer_ranges = ReplicationSlices::default();

        for _ in 0..3 {
            buffer_ranges.start_array();
            buffer_ranges.end_array(&mut buffer, 0)?;
        }

        buffer_ranges.trim_empty_arrays();

        assert_eq!(buffer_ranges.iter(&buffer).count(), 0);

        Ok(())
    }
}
