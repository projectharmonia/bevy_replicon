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

    /// Registered functions for replicated components.
    commands: Vec<(ComponentId, CommandFns)>,

    serde: Vec<(usize, SerdeFns)>,

    /// Number of registered markers.
    ///
    /// Used to initialize new [`CommandFns`] with the registered number of slots.
    marker_slots: usize,
}

impl ReplicationFns {
    pub(super) fn add_marker_slots(&mut self, marker_id: CommandMarkerId) {
        self.marker_slots += 1;
        for (_, command_fns) in &mut self.commands {
            command_fns.add_marker_slot(marker_id);
        }
    }

    pub(super) fn register_marker<C: Component>(
        &mut self,
        world: &mut World,
        marker_id: CommandMarkerId,
        write: WriteFn,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_command_fns::<C>(world);
        let (_, command_fns) = unsafe { self.commands.get_unchecked_mut(index) };
        command_fns.set_marker_fns(marker_id, write, remove);
    }

    pub fn register_default_serde<C>(&mut self, world: &mut World) -> SerdeInfo
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.register_serde(
            world,
            serde_fns::serialize::<C>,
            serde_fns::deserialize::<C>,
            serde_fns::deserialize_in_place::<C>,
        )
    }

    pub fn register_default_mapped_serde<C>(&mut self, world: &mut World) -> SerdeInfo
    where
        C: Component + Serialize + DeserializeOwned + MapEntities,
    {
        self.register_serde(
            world,
            serde_fns::serialize::<C>,
            serde_fns::deserialize_mapped::<C>,
            serde_fns::deserialize_in_place::<C>,
        )
    }

    pub fn register_serde<C: Component>(
        &mut self,
        world: &mut World,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> SerdeInfo {
        let (index, component_id) = self.init_command_fns::<C>(world);
        let serde_fns = SerdeFns::new(serialize, deserialize, deserialize_in_place);
        self.serde.push((index, serde_fns));

        SerdeInfo {
            component_id,
            serde_id: SerdeFnsId(self.serde.len() - 1),
        }
    }

    fn init_command_fns<C: Component>(&mut self, world: &mut World) -> (usize, ComponentId) {
        let component_id = world.init_component::<C>();
        let index = self
            .commands
            .iter()
            .position(|&(id, _)| id == component_id)
            .unwrap_or_else(|| {
                self.commands
                    .push((component_id, CommandFns::new::<C>(self.marker_slots)));
                self.commands.len() - 1
            });

        (index, component_id)
    }

    pub(crate) fn get(&self, serde_id: SerdeFnsId) -> (&SerdeFns, &CommandFns) {
        let (index, serde_fns) = self
            .serde
            .get(serde_id.0)
            .expect("serde function IDs should be obtained from the same instance");
        let (_, command_fns) = unsafe { self.commands.get_unchecked(*index) };
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

#[derive(Clone, Copy)]
pub struct SerdeInfo {
    component_id: ComponentId,
    serde_id: SerdeFnsId,
}

impl SerdeInfo {
    pub(crate) fn component_id(&self) -> ComponentId {
        self.component_id
    }

    pub(crate) fn serde_id(&self) -> SerdeFnsId {
        self.serde_id
    }
}

/// Represents ID of [`ComponentFns`].
///
/// Can be obtained from [`ReplicationFns::register_component_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SerdeFnsId(usize);

/// Signature of the entity despawn function.
pub type DespawnFn = fn(EntityWorldMut, RepliconTick);

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}
