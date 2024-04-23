use std::io::Cursor;

use bevy::{ecs::system::EntityCommands, prelude::*, ptr::Ptr};

use super::{command_fns::UntypedCommandFns, rule_fns::UntypedRuleFns};
use crate::{
    client::client_mapper::ServerEntityMap,
    core::{command_markers::CommandMarkerIndex, replicon_tick::RepliconTick},
};

/// Type-erased functions for a component.
///
/// Stores type-erased command functions and functions that will restore original types.
pub struct ComponentFns {
    serialize: UntypedSerializeFn,
    write: UntypedWriteFn,
    commands: UntypedCommandFns,
    markers: Vec<Option<UntypedCommandFns>>,
}

impl ComponentFns {
    /// Creates a new instance for `C` with the specified number of empty marker function slots.
    pub(super) fn new<C: Component>(marker_slots: usize) -> Self {
        Self {
            serialize: type_serialize::<C>,
            write: type_write::<C>,
            commands: UntypedCommandFns::default_fns::<C>(),
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
    /// The caller must ensure that `command_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if there is no such slot for the marker. Use [`Self::add_marker_slot`] to assign.
    pub(super) unsafe fn set_marker_fns(
        &mut self,
        marker_id: CommandMarkerIndex,
        command_fns: UntypedCommandFns,
    ) {
        let fns = self
            .markers
            .get_mut(*marker_id)
            .unwrap_or_else(|| panic!("command fns should have a slot for {marker_id:?}"));

        debug_assert!(
            fns.is_none(),
            "function for {marker_id:?} can't be set twice"
        );

        *fns = Some(command_fns);
    }

    /// Sets default functions that will be called when there are no marker matches.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `command_fns` was created for the same type as this instance.
    pub(super) unsafe fn set_command_fns(&mut self, command_fns: UntypedCommandFns) {
        self.commands = command_fns;
    }

    /// Restores erased type from `ptr` and `rule_fns` to the type for which this instance was created.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` and `rule_fns` were created for the same type as this instance.
    pub unsafe fn serialize(
        &self,
        rule_fns: &UntypedRuleFns,
        ptr: Ptr,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        (self.serialize)(rule_fns, ptr, cursor)
    }

    /// Calls the assigned writing function based on entity markers.
    ///
    /// Entity markers store information about which markers are present on an entity.
    /// The first-found write function whose marker is present on the entity will be selected
    /// (the functions are sorted by priority).
    /// If there is no such function, it will use the default function.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `rule_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` is enabled and `entity_markers` has a different length than the number of marker slots.
    pub unsafe fn write(
        &self,
        rule_fns: &UntypedRuleFns,
        entity_markers: &[bool],
        commands: &mut Commands,
        entity: &mut EntityMut,
        cursor: &mut Cursor<&[u8]>,
        entity_map: &mut ServerEntityMap,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        let command_fns = self.marker_fns(entity_markers).unwrap_or(self.commands);
        (self.write)(
            &command_fns,
            rule_fns,
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
        let command_fns = self.marker_fns(entity_markers).unwrap_or(self.commands);
        command_fns.remove(entity_commands, replicon_tick)
    }

    /// Picks assigned functions based on markers present on an entity.
    pub(super) fn marker_fns(&self, entity_markers: &[bool]) -> Option<UntypedCommandFns> {
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

/// Signature of component serialization functions that restore the original type.
type UntypedSerializeFn =
    unsafe fn(&UntypedRuleFns, Ptr, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component writing functions that restore the original type.
type UntypedWriteFn = unsafe fn(
    &UntypedCommandFns,
    &UntypedRuleFns,
    &mut Commands,
    &mut EntityMut,
    &mut Cursor<&[u8]>,
    &mut ServerEntityMap,
    RepliconTick,
) -> bincode::Result<()>;

/// Dereferences a component from a pointer and calls the passed serialization function.
///
/// # Safety
///
/// The caller must ensure that `ptr` and `rule_fns` were created for `C`.
unsafe fn type_serialize<C: Component>(
    rule_fns: &UntypedRuleFns,
    ptr: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    let rule_fns = rule_fns.typed::<C>();
    rule_fns.serialize(ptr.deref::<C>(), cursor)
}

/// Resolves `rule_fns` to `C` and calls [`UntypedCommandFns::write`] for `C`.
///
/// # Safety
///
/// The caller must ensure that `rule_fns` was created for `C`.
unsafe fn type_write<C: Component>(
    command_fns: &UntypedCommandFns,
    rule_fns: &UntypedRuleFns,
    commands: &mut Commands,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
    entity_map: &mut ServerEntityMap,
    replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    command_fns.write::<C>(
        &rule_fns.typed::<C>(),
        commands,
        entity,
        cursor,
        entity_map,
        replicon_tick,
    )
}
