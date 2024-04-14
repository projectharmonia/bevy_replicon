pub mod command_fns;
pub mod serde_fns;

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities},
    prelude::*,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{command_markers::CommandMarkerId, replicon_tick::RepliconTick};
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
    /// index of its [`CommandFns`].
    ///
    /// Can be registered multiple times for the same component.
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
    pub(super) fn register_marker(&mut self, marker_id: CommandMarkerId) {
        self.marker_slots += 1;
        for (command_fns, _) in &mut self.commands {
            command_fns.add_marker_slot(marker_id);
        }
    }

    /// Associates command functions with a marker.
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with a
    /// [`SerdeFns`] created for `C`.
    ///
    /// # Panics
    ///
    /// Panics if marker wasn't registered. Use [`Self::register_marker`] first.
    pub(super) unsafe fn register_marker_fns<C: Component>(
        &mut self,
        world: &mut World,
        marker_id: CommandMarkerId,
        write: WriteFn,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_command_fns::<C>(world);

        // SAFETY: index obtained from `Self::init_command_fns` is always valid.
        let (command_fns, _) = self.commands.get_unchecked_mut(index);

        // SAFETY: `command_fns` was created for `C` and the caller ensured
        // that `write` can be safely called with a `SerdeFns` created for `C`.
        command_fns.set_marker_fns(marker_id, write, remove);
    }

    /// Same as [`Self::register_serde_fns`], but uses default functions for a component.
    ///
    /// If your component contains any [`Entity`] inside, use [`Self::register_default_serde_fns`].
    pub fn register_default_serde_fns<C>(&mut self, world: &mut World) -> FnsInfo
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.register_serde_fns(
            world,
            serde_fns::serialize::<C>,
            serde_fns::deserialize::<C>,
            serde_fns::deserialize_in_place::<C>,
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
            serde_fns::serialize::<C>,
            serde_fns::deserialize_mapped::<C>,
            serde_fns::deserialize_in_place::<C>,
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
    pub(crate) fn get(&self, fns_id: FnsId) -> (&SerdeFns, &CommandFns) {
        let (serde_fns, index) = self
            .serde
            .get(fns_id.0)
            .expect("serde function IDs should be obtained from the same instance");

        // SAFETY: index obtained from `serde` is always valid.
        let (command_fns, _) = unsafe { self.commands.get_unchecked(*index) };

        (serde_fns, command_fns)
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

/// IDs of registered replication function and its component.
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

/// Signature of entity despawn function.
pub type DespawnFn = fn(EntityWorldMut, RepliconTick);

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_serde_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_default_serde_fns::<DummyComponent>(&mut world);
        replication_fns.register_mapped_serde_fns::<DummyComponent>(&mut world);

        assert_eq!(replication_fns.serde.len(), 2);
        assert_eq!(
            replication_fns.commands.len(),
            1,
            "different serde registrations for the same component should result only in a single command functions instance"
        );
    }

    #[derive(Component, Serialize, Deserialize)]
    struct DummyComponent;

    impl MapEntities for DummyComponent {
        fn map_entities<M: EntityMapper>(&mut self, _entity_mapper: &mut M) {}
    }
}
