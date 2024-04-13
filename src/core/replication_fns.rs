pub mod command_fns;
pub mod serde_fns;

use bevy::{ecs::component::ComponentId, prelude::*};
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

    serde: Vec<(CommandFnsId, SerdeFns)>,
}

impl ReplicationFns {
    pub(super) fn add_marker_slots(&mut self, marker_id: CommandMarkerId) {
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
        let (_, commands_id) = self.init_command_fns::<C>(world);
        let (_, command_fns) = unsafe { self.commands.get_unchecked_mut(commands_id.0) };
        command_fns.set_marker_fns(marker_id, write, remove);
    }

    pub fn register_default_serde<C: Component + Serialize + DeserializeOwned>(
        &mut self,
        world: &mut World,
    ) -> SerdeInfo {
        self.register_serde(
            world,
            serde_fns::serialize::<C>,
            serde_fns::deserialize::<C>,
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
        let (component_id, commands_id) = self.init_command_fns::<C>(world);
        let serde_fns = SerdeFns::new(serialize, deserialize, deserialize_in_place);
        self.serde.push((commands_id, serde_fns));

        SerdeInfo {
            component_id,
            serde_id: SerdeFnsId(self.serde.len() - 1),
        }
    }

    fn init_command_fns<C: Component>(&mut self, world: &mut World) -> (ComponentId, CommandFnsId) {
        let component_id = world.init_component::<C>();
        let index = self
            .commands
            .iter()
            .position(|&(id, _)| id == component_id)
            .unwrap_or_else(|| {
                self.commands.push((component_id, CommandFns::new::<C>()));
                self.commands.len() - 1
            });

        (component_id, CommandFnsId(index))
    }

    pub(crate) fn get(&self, serde_id: SerdeFnsId) -> (&SerdeFns, &CommandFns) {
        let (commands_id, serde_fns) = self
            .serde
            .get(serde_id.0)
            .expect("serde function IDs should be obtained from the same instance");
        let (_, command_fns) = unsafe { self.commands.get_unchecked(commands_id.0) };
        (serde_fns, command_fns)
    }
}

impl Default for ReplicationFns {
    fn default() -> Self {
        Self {
            despawn: despawn_recursive,
            commands: Default::default(),
            serde: Default::default(),
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
struct CommandFnsId(usize);

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
