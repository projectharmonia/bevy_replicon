use std::io::Cursor;

use bevy::{ecs::system::EntityCommands, prelude::*, ptr::Ptr};

use super::serde_fns::SerdeFns;
use crate::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::{command_markers::CommandMarkerId, replicon_tick::RepliconTick},
};

/// Functions that operate on components like [`Commands`].
///
/// Unlike [`SerdeFns`], registered for each component type instead of
/// [`ReplicationRule`](crate::core::replication_rules::ReplicationRule).
///
/// User can override default functions per-entity by providing a marker,
/// see [`CommandMarkers`](crate::core::command_markers::CommandMarkers)
pub(crate) struct CommandFns {
    read: ReadFn,
    write: WriteFn,
    remove: RemoveFn,
    markers: Vec<Option<(WriteFn, RemoveFn)>>,
}

impl CommandFns {
    /// Creates a new instance for `C` with default functions and the specified number of empty marker function slots.
    pub(super) fn new<C: Component>(marker_slots: usize) -> Self {
        Self {
            read: read::<C>,
            write: write::<C>,
            remove: remove::<C>,
            markers: vec![None; marker_slots],
        }
    }

    /// Adds new empty slot for a marker.
    ///
    /// Use [`Self::set_marker_fns`] to assign functions to it.
    pub(super) fn add_marker_slot(&mut self, marker_id: CommandMarkerId) {
        self.markers.insert(*marker_id, None);
    }

    /// Assigns functions to a marker slots.
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with a
    /// [`SerdeFns`] created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if there is no such slot for the marker. Use [`Self::add_marker_slot`] to assign.
    pub(super) unsafe fn set_marker_fns(
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

    /// Calls [`read`] on the type for which this instance was created.
    ///
    /// It's a non-overridable function that used to just restore the erased type from [`Ptr`].
    /// To customize serialization behavior, [`SerdeFns`] should be used instead.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` and `serde_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` enabled and `entity_markers` have different length then the number of marker slots.
    pub(crate) unsafe fn read(
        &self,
        serde_fns: &SerdeFns,
        ptr: Ptr,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        (self.read)(serde_fns, ptr, cursor)
    }

    /// Calls assigned writing function based on entity markers.
    ///
    /// Entity markers stores information about which marker is present on an entity.
    /// The function will pick first assigned write function whose marker is present on the entity.
    /// If there is no such function, it will use the default [`write`].
    ///
    /// See also [`Self::set_marker_fns`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `serde_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` enabled and `entity_markers` have different length then the number of marker slots.
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

    /// Same as [`Self::write`], but calls assigned remove function.
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

    /// Picks assigned functions based on markers present on an entity.
    pub(super) fn marker_fns(&self, entity_markers: &[bool]) -> Option<(WriteFn, RemoveFn)> {
        debug_assert_eq!(
            entity_markers.len(),
            self.markers.len(),
            "entity markers lenght and marker functions slots should match"
        );

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

/// Dereferences a component from a pointer and calls passed serialization function.
///
/// # Safety
///
/// The caller must ensure that `ptr` and `serde_fns` was created for `C`.
unsafe fn read<C: Component>(
    serde_fns: &SerdeFns,
    ptr: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    serde_fns.serialize(ptr.deref::<C>(), cursor)
}

/// Default component writing function.
///
/// If such a component did not exist, it will be deserialized with [`SerdeFns::deserialize`] and added as a command.
/// If such a component exists, [`SerdeFns::deserialize_in_place`] will be used directly on entity's component.
///
/// # Safety
///
/// The caller must ensure that `serde_fns` was created for `C`.
pub unsafe fn write<C: Component>(
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
pub fn remove<C: Component>(mut entity_commands: EntityCommands, _replicon_tick: RepliconTick) {
    entity_commands.remove::<C>();
}
