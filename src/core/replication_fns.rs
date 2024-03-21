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
    pub despawn: DespawnFn,

    /// Functions for component serialization/deserialization.
    serde: Vec<SerdeFns>,

    /// Functions for removing components.
    remove: Vec<RemoveFn>,
}

impl ReplicationFns {
    /// Registers [`SerdeFns`] for a component and returns its ID.
    ///
    /// Returned ID can be assigned to components inside
    /// [`ReplicatedArchetype`](super::replicated_archetypes::ReplicatedArchetype).
    ///
    /// Could be called multiple times for the same component with different functions.
    pub fn add_serde_fns(&mut self, serde_fns: SerdeFns) -> SerdeFnsId {
        self.serde.push(serde_fns);

        SerdeFnsId(self.serde.len() - 1)
    }

    /// Registers [`RemoveFn`] for a component and returns its ID.
    ///
    /// Returned ID can be assigned to components inside
    /// [`ReplicatedArchetype`](super::replicated_archetypes::ReplicatedArchetype).
    ///
    /// Could be called multiple times for the same component with different functions.
    pub fn add_remove_fn(&mut self, remove: RemoveFn) -> RemoveFnId {
        self.remove.push(remove);

        RemoveFnId(self.serde.len() - 1)
    }

    /// Returns a reference to registered serde functions.
    ///
    /// # Safety
    ///
    /// `id` should point to a valid item.
    pub(crate) unsafe fn serde_fn_unchecked(&self, id: SerdeFnsId) -> &SerdeFns {
        self.serde.get_unchecked(id.0)
    }

    /// Returns a reference to registered remove function.
    ///
    /// # Safety
    ///
    /// `id` should point to a valid item.
    pub(crate) unsafe fn remove_fn_unchecked(&self, id: RemoveFnId) -> &RemoveFn {
        self.remove.get_unchecked(id.0)
    }
}

impl Default for ReplicationFns {
    fn default() -> Self {
        Self {
            despawn: despawn_recursive,
            serde: Default::default(),
            remove: Default::default(),
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
pub type RemoveFn = fn(&mut EntityWorldMut, RepliconTick);

/// Signature of the entity despawn function.
pub type DespawnFn = fn(EntityWorldMut, RepliconTick);

/// Serialization and deserialization functions for a replicated component.
#[derive(Clone)]
pub struct SerdeFns {
    /// Function that serializes a component into bytes.
    pub serialize: SerializeFn,

    /// Function that deserializes a component from bytes and inserts it to [`EntityWorldMut`].
    pub deserialize: DeserializeFn,
}

/// Represents ID of [`SerdeFns`].
///
/// Can be obtained from [`ReplicationFns::add_serde_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SerdeFnsId(usize);

/// Represents ID of [`RemoveFn`].
///
/// Can be obtained from [`ReplicationFns::add_remove_fn`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct RemoveFnId(usize);

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}
