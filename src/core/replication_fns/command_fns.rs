use std::io::Cursor;

use bevy::{ecs::system::EntityCommands, prelude::*, ptr::Ptr};

use super::serde_fns::SerdeFns;
use crate::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::{command_markers::CommandMarkerIndex, replicon_tick::RepliconTick},
};

/// Functions that operate on components like [`Commands`].
///
/// Unlike [`SerdeFns`] which are selected on the server via
/// [`ReplicationRules`](crate::core::replication_rules::ReplicationRule), the remove/remove
/// functions in `markers` here are selected on the client via marker components.
/// For details see [`AppMarkerExt`](crate::core::command_markers::AppMarkerExt).
pub struct CommandFns {
    read: ReadFn,
    write: WriteFn,
    remove: RemoveFn,
    markers: Vec<Option<(WriteFn, RemoveFn)>>,
}

impl CommandFns {
    /// Creates a new instance for `C` with default functions and the specified number of empty marker function slots.
    pub(super) fn new<C: Component>(marker_slots: usize) -> Self {
        Self {
            read: default_read::<C>,
            write: default_write::<C>,
            remove: default_remove::<C>,
            markers: vec![None; marker_slots],
        }
    }

    /// Adds new empty slot for a marker.
    ///
    /// Use [`Self::set_marker_fns`] to assign functions to it.
    pub(super) fn add_marker_slot(&mut self, marker_id: CommandMarkerIndex) {
        self.markers.insert(*marker_id, None);
    }

    /// Assigns functions to a marker slot.
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with all
    /// [`SerdeFns`] created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if there is no such slot for the marker. Use [`Self::add_marker_slot`] to assign.
    pub(super) unsafe fn set_marker_fns(
        &mut self,
        marker_id: CommandMarkerIndex,
        write: WriteFn,
        remove: RemoveFn,
    ) {
        let fns = self
            .markers
            .get_mut(*marker_id)
            .unwrap_or_else(|| panic!("command fns should have a slot for {marker_id:?}"));

        debug_assert!(
            fns.is_none(),
            "function for {marker_id:?} can't be set twice"
        );

        *fns = Some((write, remove));
    }

    /// Sets default functions when there are no markers.
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with all
    /// [`SerdeFns`] created for the same type as this instance.
    pub(super) unsafe fn set_fns(&mut self, write: WriteFn, remove: RemoveFn) {
        self.write = write;
        self.remove = remove;
    }

    /// Calls [`default_read`] on the type for which this instance was created.
    ///
    /// It's a non-overridable function that is used to restore the erased type from [`Ptr`].
    /// To customize serialization behavior, [`SerdeFns`] should be used instead.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` and `serde_fns` were created for the same type as this instance.
    pub unsafe fn read(
        &self,
        serde_fns: &SerdeFns,
        ptr: Ptr,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        (self.read)(serde_fns, ptr, cursor)
    }

    /// Calls the assigned writing function based on entity markers.
    ///
    /// Entity markers store information about which markers are present on an entity.
    /// The first-found write function whose marker is present on the entity will be selected
    /// (the functions are sorted by priority).
    /// If there is no such function, it will use the [`default_write`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `serde_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` is enabled and `entity_markers` has a different length than the number of marker slots.
    pub unsafe fn write(
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

    /// Same as [`Self::write`], but calls the assigned remove function.
    pub fn remove(
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
            "entity markers length and marker functions slots should match"
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

/// Dereferences a component from a pointer and calls the passed serialization function.
///
/// # Safety
///
/// The caller must ensure that `ptr` and `serde_fns` were created for `C`.
pub unsafe fn default_read<C: Component>(
    serde_fns: &SerdeFns,
    ptr: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    serde_fns.serialize(ptr.deref::<C>(), cursor)
}

/// Default component writing function.
///
/// If the component does not exist on the entity, it will be deserialized with [`SerdeFns::deserialize`] and inserted via [`Commands`].
/// If the component exists on the entity, [`SerdeFns::deserialize_in_place`] will be used directly on the entity's component.
///
/// # Safety
///
/// The caller must ensure that `serde_fns` was created for `C`.
pub unsafe fn default_write<C: Component>(
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
pub fn default_remove<C: Component>(
    mut entity_commands: EntityCommands,
    _replicon_tick: RepliconTick,
) {
    entity_commands.remove::<C>();
}
