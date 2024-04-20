pub mod command_fns;
pub mod serde_fns;

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities},
    prelude::*,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{command_markers::CommandMarkerIndex, replicon_tick::RepliconTick};
use command_fns::{CommandFns, RemoveFn, WriteFn};
use serde_fns::{DeserializeFn, DeserializeInPlaceFn, SerdeFns, SerializeFn};

/// Stores configurable replication functions.
#[derive(Resource)]
pub struct ReplicationFns {
    /// Custom function to handle entity despawning.
    ///
    /// By default uses [`despawn_recursive`].
    /// Useful if you need to intercept despawns and handle them in a special way.
    pub despawn: DespawnFn,

    /// Read/write/remove functions for replicated components.
    ///
    /// Unique for each component.
    commands: Vec<(CommandFns, ComponentId)>,

    /// Serialization/deserialization functions for a component and
    /// the component's index in [`Self::commands`].
    ///
    /// Can be registered multiple times for the same component for a different
    /// [`ReplicationRule`].
    serde: Vec<(SerdeFns, usize)>,

    /// Number of registered markers.
    ///
    /// Used to initialize new [`CommandFns`] with the registered number of slots.
    marker_slots: usize,
}

impl ReplicationFns {
    /// Registers marker slot for command functions.
    ///
    /// Should be used after calling
    /// [`CommandMarkers::insert`](super::command_markers::CommandMarkers::insert)
    pub(super) fn register_marker(&mut self, marker_id: CommandMarkerIndex) {
        self.marker_slots += 1;
        for (command_fns, _) in &mut self.commands {
            command_fns.add_marker_slot(marker_id);
        }
    }

    /// Associates command functions with a marker for a component.
    ///
    /// **Must** be called **after** calling [`Self::register_marker`] with `marker_id`.
    ///
    /// See also [`Self::set_command_fns`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with all
    /// [`SerdeFns`] registered for `C` with other methods on this struct.
    ///
    /// # Panics
    ///
    /// Panics if the marker wasn't registered. Use [`Self::register_marker`] first.
    pub(super) unsafe fn set_marker_fns<C: Component>(
        &mut self,
        world: &mut World,
        marker_id: CommandMarkerIndex,
        write: WriteFn,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_command_fns::<C>(world);
        let (command_fns, _) = &mut self.commands[index];

        // SAFETY: `command_fns` was created for `C` and the caller ensured
        // that `write` can be safely called with a `SerdeFns` created for `C`.
        command_fns.set_marker_fns(marker_id, write, remove);
    }

    /// Sets default functions for a component when there are no markers.
    ///
    /// See also [`Self::set_marker_fns`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with all
    /// [`SerdeFns`] registered for `C` with other methods on this struct.
    pub(super) unsafe fn set_command_fns<C: Component>(
        &mut self,
        world: &mut World,
        write: WriteFn,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_command_fns::<C>(world);
        let (command_fns, _) = &mut self.commands[index];

        // SAFETY: `command_fns` was created for `C` and the caller ensured
        // that `write` can be safely called with a `SerdeFns` created for `C`.
        command_fns.set_fns(write, remove);
    }

    /// Same as [`Self::register_serde_fns`], but uses default functions for a component.
    ///
    /// If your component contains any [`Entity`] inside, use [`Self::register_mapped_serde_fns`].
    pub fn register_default_serde_fns<C>(&mut self, world: &mut World) -> FnsInfo
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.register_serde_fns(
            world,
            serde_fns::default_serialize::<C>,
            serde_fns::default_deserialize::<C>,
            serde_fns::in_place_as_deserialize::<C>,
        )
    }

    /// Same as [`Self::register_serde_fns`], but uses default functions for a mapped component.
    ///
    /// Always use it for components that contain entities.
    ///
    /// See also [`Self::register_default_serde_fns`].
    pub fn register_mapped_serde_fns<C>(&mut self, world: &mut World) -> FnsInfo
    where
        C: Component + Serialize + DeserializeOwned + MapEntities,
    {
        self.register_serde_fns(
            world,
            serde_fns::default_serialize::<C>,
            serde_fns::default_deserialize_mapped::<C>,
            serde_fns::in_place_as_deserialize::<C>,
        )
    }

    /// Registers serialization/deserialization functions for a component.
    ///
    /// Returned data can be assigned to a
    /// [`ReplicationRule`](super::replication_rules::ReplicationRule)
    pub fn register_serde_fns<C: Component>(
        &mut self,
        world: &mut World,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> FnsInfo {
        let (index, component_id) = self.init_command_fns::<C>(world);
        let serde_fns = SerdeFns::new(serialize, deserialize, deserialize_in_place);
        self.serde.push((serde_fns, index));

        FnsInfo {
            component_id,
            fns_id: FnsId(self.serde.len() - 1),
        }
    }

    /// Initializes [`CommandFns`] for a component and returns its index and ID.
    ///
    /// If a [`CommandFns`] has already been created for this component,
    /// then it returns its index instead of creating a new one.
    fn init_command_fns<C: Component>(&mut self, world: &mut World) -> (usize, ComponentId) {
        let component_id = world.init_component::<C>();
        let index = self
            .commands
            .iter()
            .position(|&(_, id)| id == component_id)
            .unwrap_or_else(|| {
                self.commands
                    .push((CommandFns::new::<C>(self.marker_slots), component_id));
                self.commands.len() - 1
            });

        (index, component_id)
    }

    /// Returns associates functions.
    ///
    /// See also [`Self::register_serde_fns`].
    pub fn get(&self, fns_id: FnsId) -> (&CommandFns, &SerdeFns) {
        let (serde_fns, index) = self
            .serde
            .get(fns_id.0)
            .expect("serde function IDs should be obtained from the same instance");

        // SAFETY: index obtained from `serde` is always valid.
        let (command_fns, _) = unsafe { self.commands.get_unchecked(*index) };

        (command_fns, serde_fns)
    }
}

impl Default for ReplicationFns {
    fn default() -> Self {
        Self {
            despawn: despawn_recursive,
            commands: Default::default(),
            serde: Default::default(),
            marker_slots: 0,
        }
    }
}

/// IDs of a registered replication function and its component.
///
/// Can be obtained from [`ReplicationFns::register_serde_fns`].
#[derive(Clone, Copy)]
pub struct FnsInfo {
    component_id: ComponentId,
    fns_id: FnsId,
}

impl FnsInfo {
    pub(crate) fn component_id(&self) -> ComponentId {
        self.component_id
    }

    pub(crate) fn fns_id(&self) -> FnsId {
        self.fns_id
    }
}

/// ID of replicaton functions for a component.
///
/// Can be obtained from [`ReplicationFns::register_serde_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct FnsId(usize);

/// Signature of the entity despawn function.
pub type DespawnFn = fn(EntityWorldMut, RepliconTick);

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::command_markers::{CommandMarker, CommandMarkers};

    #[test]
    fn marker() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        let mut command_markers = CommandMarkers::default();

        let marker_a = command_markers.insert(CommandMarker {
            component_id: world.init_component::<MarkerA>(),
            priority: 0,
        });
        replication_fns.register_marker(marker_a);

        // SAFETY: `write` can be safely called with a `SerdeFns` created for `ComponentA`.
        unsafe {
            replication_fns.set_marker_fns::<ComponentA>(
                &mut world,
                marker_a,
                command_fns::default_write::<ComponentA>,
                command_fns::default_remove::<ComponentA>,
            );
        }

        let (command_fns_a, _) = &replication_fns.commands[0];
        assert!(command_fns_a.marker_fns(&[false]).is_none());
        assert!(command_fns_a.marker_fns(&[true]).is_some());
    }

    #[test]
    fn two_markers() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        let mut command_markers = CommandMarkers::default();

        let marker_a = command_markers.insert(CommandMarker {
            component_id: world.init_component::<MarkerA>(),
            priority: 1,
        });
        replication_fns.register_marker(marker_a);

        let marker_b = command_markers.insert(CommandMarker {
            component_id: world.init_component::<MarkerB>(),
            priority: 0,
        });
        replication_fns.register_marker(marker_b);

        // SAFETY: `write` can be safely called with `SerdeFns` for
        // `ComponentA` and `ComponentA` for each call respectively.
        unsafe {
            replication_fns.set_marker_fns::<ComponentA>(
                &mut world,
                marker_a,
                command_fns::default_write::<ComponentA>,
                command_fns::default_remove::<ComponentA>,
            );
            replication_fns.set_marker_fns::<ComponentB>(
                &mut world,
                marker_b,
                command_fns::default_write::<ComponentB>,
                command_fns::default_remove::<ComponentB>,
            );
        }

        let (command_fns_a, _) = &replication_fns.commands[0];
        assert!(command_fns_a.marker_fns(&[false, false]).is_none());
        assert!(command_fns_a.marker_fns(&[true, false]).is_some());
        assert!(command_fns_a.marker_fns(&[false, true]).is_none());
        assert!(command_fns_a.marker_fns(&[true, true]).is_some());

        let (command_fns_b, _) = &replication_fns.commands[1];
        assert!(command_fns_b.marker_fns(&[false, false]).is_none());
        assert!(command_fns_b.marker_fns(&[true, false]).is_none());
        assert!(command_fns_b.marker_fns(&[false, true]).is_some());
        assert!(command_fns_b.marker_fns(&[true, true]).is_some());
    }

    #[test]
    fn default_serde_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_default_serde_fns::<ComponentA>(&mut world);
        assert_eq!(replication_fns.serde.len(), 1);
        assert_eq!(replication_fns.commands.len(), 1);
    }

    #[test]
    fn mapped_serde_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_mapped_serde_fns::<ComponentB>(&mut world);
        assert_eq!(replication_fns.serde.len(), 1);
        assert_eq!(replication_fns.commands.len(), 1);
    }

    #[test]
    fn duplicate_serde_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_default_serde_fns::<ComponentA>(&mut world);
        replication_fns.register_default_serde_fns::<ComponentA>(&mut world);

        assert_eq!(replication_fns.serde.len(), 2);
        assert_eq!(
            replication_fns.commands.len(),
            1,
            "multiple serde registrations for the same component should result only in a single command functions instance"
        );
    }

    #[test]
    fn different_serde_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_default_serde_fns::<ComponentA>(&mut world);
        replication_fns.register_mapped_serde_fns::<ComponentB>(&mut world);

        assert_eq!(replication_fns.serde.len(), 2);
        assert_eq!(replication_fns.commands.len(), 2);
    }

    #[derive(Component, Serialize, Deserialize)]
    struct ComponentA;

    #[derive(Component, Deserialize, Serialize)]
    struct ComponentB;

    impl MapEntities for ComponentB {
        fn map_entities<M: EntityMapper>(&mut self, _entity_mapper: &mut M) {}
    }

    #[derive(Component)]
    struct MarkerA;

    #[derive(Component)]
    struct MarkerB;
}
