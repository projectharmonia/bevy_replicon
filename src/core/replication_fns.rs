use std::io::Cursor;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::replicon_tick::RepliconTick;
use crate::client::client_mapper::{ClientMapper, ServerEntityMap};

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
    ///
    /// Unlike `serde` functions, removals are registered per replication rule.
    remove: Vec<RemoveFn>,
}

impl ReplicationFns {
    /// Registers [`SerdeFns`] for a component and returns its ID.
    ///
    /// Returned ID can be assigned to components inside
    /// [`ReplicationRule`](super::replication_rules::ReplicationRule).
    ///
    /// Could be called multiple times for the same component with different functions.
    pub fn register_serde_fns(&mut self, serde_fns: SerdeFns) -> SerdeFnsId {
        self.serde.push(serde_fns);

        SerdeFnsId(self.serde.len() - 1)
    }

    /// Registers removal functions a component group and returns its ID.
    ///
    /// Returned ID can be assigned to
    /// [`ReplicationRule`](super::replication_rules::ReplicationRule).
    ///
    /// Could be called multiple times for a replication rule with different functions.
    pub fn register_remove_fn(&mut self, remove: RemoveFn) -> RemoveFnId {
        self.remove.push(remove);

        RemoveFnId(self.remove.len() - 1)
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
/// Can be obtained from [`ReplicationFns::register_serde_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SerdeFnsId(usize);

/// Represents ID of [`Vec<RemoveFn>`].
///
/// Can be obtained from [`ReplicationFns::register_remove_fn`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct RemoveFnId(usize);

/// Default serialization function.
pub fn serialize<C: Component + Serialize>(
    component: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    // SAFETY: function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default deserialization function.
pub fn deserialize<C: Component + DeserializeOwned>(
    entity: &mut EntityWorldMut,
    _entity_map: &mut ServerEntityMap,
    cursor: &mut Cursor<&[u8]>,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let component: C = DefaultOptions::new().deserialize_from(cursor)?;
    entity.insert(component);

    Ok(())
}

/// Like [`deserialize`], but also maps entities before insertion.
pub fn deserialize_mapped<C: Component + DeserializeOwned + MapEntities>(
    entity: &mut EntityWorldMut,
    entity_map: &mut ServerEntityMap,
    cursor: &mut Cursor<&[u8]>,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;

    entity.world_scope(|world| {
        component.map_entities(&mut ClientMapper::new(world, entity_map));
    });

    entity.insert(component);

    Ok(())
}

/// Default components removal function.
pub fn remove<B: Bundle>(entity: &mut EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.remove::<B>();
}

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}
