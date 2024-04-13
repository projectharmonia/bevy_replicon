use std::io::Cursor;

use bevy::{ecs::system::EntityCommands, prelude::*, ptr::Ptr};

use super::serde_fns::SerdeFns;
use crate::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::{command_markers::CommandMarkerId, replicon_tick::RepliconTick},
};

pub(crate) struct CommandFns {
    read: ReadFn,
    write: WriteFn,
    remove: RemoveFn,
    markers: Vec<Option<(WriteFn, RemoveFn)>>,
}

impl CommandFns {
    /// Creates a new instance with default functions and the specified number of empty marker function slots.
    pub(super) fn new<C: Component>(marker_slots: usize) -> Self {
        Self {
            read: read::<C>,
            write: write::<C>,
            remove: remove::<C>,
            markers: vec![None; marker_slots],
        }
    }

    pub(super) fn add_marker_slot(&mut self, marker_id: CommandMarkerId) {
        self.markers.insert(*marker_id, None);
    }

    pub(super) fn set_marker_fns(
        &mut self,
        marker_id: CommandMarkerId,
        write: WriteFn,
        remove: RemoveFn,
    ) {
        let fns = self
            .markers
            .get_mut(*marker_id)
            .unwrap_or_else(|| panic!("command fns should have a slot for {marker_id:?}"));
        *fns = Some((write, remove));
    }

    pub(crate) unsafe fn read(
        &self,
        serde_fns: &SerdeFns,
        ptr: Ptr,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        (self.read)(serde_fns, ptr, cursor)
    }

    pub(crate) unsafe fn write(
        &self,
        serde_fns: &SerdeFns,
        entity_markers: &[bool],
        commands: &mut Commands,
        entity: &mut EntityMut,
        cursor: &mut Cursor<&[u8]>,
        entity_map: &mut ServerEntityMap,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        let write = self
            .marker_fns(entity_markers)
            .map(|(write, _)| write)
            .unwrap_or(self.write);

        (write)(
            serde_fns,
            commands,
            entity,
            cursor,
            entity_map,
            replicon_tick,
        )
    }

    pub(crate) fn remove(
        &self,
        entity_markers: &[bool],
        entity_commands: EntityCommands,
        replicon_tick: RepliconTick,
    ) {
        let remove = self
            .marker_fns(entity_markers)
            .map(|(_, remove)| remove)
            .unwrap_or(self.remove);

        (remove)(entity_commands, replicon_tick)
    }

    fn marker_fns(&self, entity_markers: &[bool]) -> Option<(WriteFn, RemoveFn)> {
        self.markers
            .iter()
            .zip(entity_markers)
            .find_map(|(fns, &enabled)| fns.filter(|_| enabled))
    }
}

/// Signature of component reading function.
pub type ReadFn = unsafe fn(&SerdeFns, Ptr, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component writing function.
pub type WriteFn = unsafe fn(
    &SerdeFns,
    &mut Commands,
    &mut EntityMut,
    &mut Cursor<&[u8]>,
    &mut ServerEntityMap,
    RepliconTick,
) -> bincode::Result<()>;

/// Signature of component removal functions.
pub type RemoveFn = fn(EntityCommands, RepliconTick);

/// Default component serialization function.
///
/// # Safety
///
/// `C` must be the erased pointee type for this [`Ptr`].
unsafe fn read<C: Component>(
    serde_fns: &SerdeFns,
    ptr: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    serde_fns.serialize(ptr.deref::<C>(), cursor)
}

/// Default component writing function.
unsafe fn write<C: Component>(
    serde_fns: &SerdeFns,
    commands: &mut Commands,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
    entity_map: &mut ServerEntityMap,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let mut mapper = ClientMapper {
        commands,
        entity_map,
    };

    if let Some(mut component) = entity.get_mut::<C>() {
        serde_fns.deserialize_in_place(&mut *component, cursor, &mut mapper)?;
    } else {
        let component: C = serde_fns.deserialize(cursor, &mut mapper)?;
        commands.entity(entity.id()).insert(component);
    }

    Ok(())
}

/// Default component removal function.
fn remove<C: Component>(mut entity_commands: EntityCommands, _replicon_tick: RepliconTick) {
    entity_commands.remove::<C>();
}
