pub mod command_fns;
pub mod serde_fns;

use bevy::{ecs::component::ComponentId, prelude::*};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::replicon_tick::RepliconTick;
use command_fns::CommandFns;
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
    pub fn register_default_serde_fns<C: Component + Serialize + DeserializeOwned>(
        &mut self,
        world: &mut World,
    ) -> SerdeInfo {
        self.register_serde_fns(
            world,
            serde_fns::serialize::<C>,
            serde_fns::deserialize::<C>,
            serde_fns::deserialize_in_place::<C>,
        )
    }

    pub fn register_serde_fns<C: Component>(
        &mut self,
        world: &mut World,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> SerdeInfo {
        let component_id = world.init_component::<C>();

        let index = self
            .commands
            .iter()
            .position(|&(id, _)| id == component_id)
            .unwrap_or_else(|| {
                self.commands.push((component_id, CommandFns::new::<C>()));
                self.commands.len() - 1
            });

        let serde_fns = SerdeFns::new(serialize, deserialize, deserialize_in_place);

        self.serde.push((CommandFnsId(index), serde_fns));

        SerdeInfo {
            component_id,
            serde_id: SerdeFnsId(self.serde.len() - 1),
        }
    }

    pub(crate) fn fns(&self, serde_id: SerdeFnsId) -> (&SerdeFns, &CommandFns) {
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
