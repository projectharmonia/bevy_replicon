use std::io::Cursor;

use bevy::{prelude::*, ptr::Ptr};
use serde::{Deserialize, Serialize};

use super::replicon_tick::RepliconTick;
use crate::client::client_mapper::ServerEntityMap;

/// Stores configurable replication functions.
#[derive(Resource)]
pub struct ReplicationFns {
    /// Custom function to handle entity despawning.
    ///
    /// By default uses [`despawn_recursive`].
    /// Useful if you need to intercept despawns and handle them in a special way.
    pub despawn: EntityDespawnFn,

    /// Functions for replicated components.
    components: Vec<ComponentFns>,
}

impl ReplicationFns {
    /// Registers [`ComponentFns`] for a component and returns its index.
    ///
    /// Returned index can be assigned for components inside
    /// [`ReplicatedArchetype`](crate::server::replicated_archetypes::ReplicatedArchetype).
    ///
    /// Could be called multiple times for the same component with different functions.
    pub fn add_component_fns(&mut self, component_fns: ComponentFns) -> ComponentFnsIndex {
        self.components.push(component_fns);

        ComponentFnsIndex(self.components.len() - 1)
    }

    /// Returns meta information about replicated component.
    ///
    /// # Safety
    ///
    /// `index` should point to a valid item.
    pub(crate) unsafe fn get_unchecked(&self, index: ComponentFnsIndex) -> &ComponentFns {
        self.components.get_unchecked(index.0)
    }
}

impl Default for ReplicationFns {
    fn default() -> Self {
        Self {
            components: Default::default(),
            despawn: despawn_recursive,
        }
    }
}

/// Signature of component serialization functions.
pub type SerializeFn = fn(Ptr, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component deserialization functions.
pub type DeserializeFn = fn(
    &mut EntityWorldMut,
    &mut ServerEntityMap,
    &mut Cursor<&[u8]>,
    RepliconTick,
) -> bincode::Result<()>;

/// Signature of component removal functions.
pub type RemoveComponentFn = fn(&mut EntityWorldMut, RepliconTick);

/// Signature of the entity despawn function.
pub type EntityDespawnFn = fn(EntityWorldMut, RepliconTick);

/// Stores functions for replicated component.
#[derive(Clone)]
pub struct ComponentFns {
    /// Function that serializes component into bytes.
    pub serialize: SerializeFn,

    /// Function that deserializes component from bytes and inserts it to [`EntityWorldMut`].
    pub deserialize: DeserializeFn,

    /// Function that removes specific component from [`EntityWorldMut`].
    pub remove: RemoveComponentFn,
}

/// Represents index of [`ComponentFns`].
///
/// Can be obtained from [`ReplicationFns::add_component_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ComponentFnsIndex(usize);

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}
