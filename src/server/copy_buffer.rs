use std::io::{Cursor, Write};

use bevy::{prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintWriter;

use crate::replicon_core::replication_rules::{ReplicationId, ReplicationInfo};

/// Efficiency-oriented buffer for writing entities and components.
pub(crate) struct CopyBuffer {
    entity_set: bool,
    entity: Cursor<Vec<u8>>,

    component_set: bool,
    component: Cursor<Vec<u8>>,
}

impl CopyBuffer {
    pub(super) fn reset(&mut self) {
        self.end_entity();
        self.end_component();
    }

    /// Serializes the entity if unset, then writes to the cursor.
    ///
    /// If there is a saved entity, the `entity` passed in must equal it (unchecked invariant).
    pub(super) fn write_entity(&mut self, cursor: &mut Cursor<Vec<u8>>, entity: Entity) -> bincode::Result<()> {
        if !self.entity_set {
            self.entity_set = true;
            serialize_entity(&mut self.entity, entity)?;
        }

        cursor.write_all(&self.entity.get_ref()[..]).unwrap();

        Ok(())
    }

    /// Serializes the component if unset, then writes to the cursor.
    ///
    /// If there is a saved component, the `component` passed in must equal it (unchecked invariant).
    pub(super) fn write_component(
        &mut self, 
        cursor: &mut Cursor<Vec<u8>>,
        replication_id: ReplicationId,
        replication_info: &ReplicationInfo,
        ptr: Ptr<'_>
    ) -> bincode::Result<u16> {
        if !self.component_set {
            self.component_set = true;
            serialize_component(&mut self.component, replication_id, replication_info, ptr)?;
        }

        cursor.write_all(&self.component.get_ref()[..]).unwrap();

        Ok(self.component.position() as u16)
    }

    /// Unsets the currently-saved entity.
    pub(super) fn end_entity(&mut self) {
        self.entity_set = false;
        self.entity.set_position(0);
        self.entity.get_mut().clear();
    }

    /// Unsets the currently-saved component.
    pub(super) fn end_component(&mut self) {
        self.component_set = false;
        self.component.set_position(0);
        self.component.get_mut().clear();
    }
}

impl Default for CopyBuffer {
    fn default() -> Self {
        Self{
            entity_set: false,
            entity: Cursor::new(Vec::new()),
            component_set: false,
            component: Cursor::new(Vec::new()),
        }
    }
}

/// Serializes `entity` by writing its index and generation as separate varints.
///
/// The index is first prepended with a bit flag to indicate if the generation
/// is serialized or not (it is not serialized if equal to zero).
pub(super) fn serialize_entity(cursor: &mut Cursor<Vec<u8>>, entity: Entity) -> bincode::Result<()> {
    let mut flagged_index = (entity.index() as u64) << 1;
    let flag = entity.generation() > 0;
    flagged_index |= flag as u64;

    cursor.write_u64_varint(flagged_index)?;
    if flag {
        cursor.write_u32_varint(entity.generation())?;
    }

    Ok(())
}

/// Serializes component with replication ID and returns serialized size.
fn serialize_component(
    cursor: &mut Cursor<Vec<u8>>,
    replication_id: ReplicationId,
    replication_info: &ReplicationInfo,
    ptr: Ptr<'_>,
) -> bincode::Result<u16> {
    let previous_pos = cursor.position();
    DefaultOptions::new().serialize_into(&mut *cursor, &replication_id)?;
    (replication_info.serialize)(ptr, cursor)?;
    let component_size = (cursor.position() - previous_pos)
        .try_into()
        .map_err(|_| bincode::ErrorKind::SizeLimit)?;

    Ok(component_size)
}
