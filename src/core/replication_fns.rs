use std::{io::Cursor, mem::MaybeUninit};

use bevy::{
    ecs::{entity::MapEntities, system::EntityCommands},
    prelude::*,
    ptr::{Ptr, PtrMut},
};
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

    /// Registered functions for replicated components.
    components: Vec<ComponentFns>,
}

impl ReplicationFns {
    /// Registers [`ComponentFns`] for a component and returns its ID.
    ///
    /// Returned ID can be assigned to a component inside
    /// [`ReplicationRule`](super::replication_rules::ReplicationRule).
    ///
    /// Could be called multiple times for the same component with different functions.
    pub fn register_component_fns(&mut self, fns: ComponentFns) -> ComponentFnsId {
        self.components.push(fns);

        ComponentFnsId(self.components.len() - 1)
    }

    /// Returns a reference to registered component functions.
    ///
    /// # Panics
    ///
    /// If functions ID points to an invalid item.
    pub(crate) fn component_fns(&self, fns_id: ComponentFnsId) -> &ComponentFns {
        self.components
            .get(fns_id.0)
            .expect("function IDs should should always be valid if obtained from the same instance")
    }
}

impl Default for ReplicationFns {
    fn default() -> Self {
        Self {
            despawn: despawn_recursive,
            components: Default::default(),
        }
    }
}

/// Signature of component serialization functions.
pub type SerializeFn = unsafe fn(Ptr, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component deserialization functions.
pub type DeserializeFn =
    unsafe fn(PtrMut, &mut Cursor<&[u8]>, &mut ClientMapper) -> bincode::Result<()>;

/// Signature of component writing functions.
pub type WriteFn = unsafe fn(
    &mut Commands,
    &mut EntityMut,
    &mut Cursor<&[u8]>,
    &mut ServerEntityMap,
    RepliconTick,
    DeserializeFn,
) -> bincode::Result<()>;

/// Signature of component removal functions.
pub type RemoveFn = fn(EntityCommands, RepliconTick);

/// Signature of the entity despawn function.
pub type DespawnFn = fn(EntityWorldMut, RepliconTick);

/// Functions for a replicated component.
#[derive(Clone)]
pub struct ComponentFns {
    /// Function that serializes a component into bytes.
    pub serialize: SerializeFn,

    /// Function that deserializes a component from bytes.
    pub deserialize: DeserializeFn,

    /// Function that calls [Self::deserialize] and inserts a component to [`EntityWorldMut`].
    pub write: WriteFn,

    /// Function that removes a component from [`EntityWorldMut`].
    pub remove: RemoveFn,
}

impl ComponentFns {
    /// Creates a new instance with [`serialize`], [`deserialize`] and [`remove`] functions.
    ///
    /// If your component contains any [`Entity`] inside, use [`Self::default_mapped_fns`].
    pub fn default_fns<C>() -> Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        Self {
            serialize: serialize::<C>,
            deserialize: deserialize::<C>,
            write: write::<C>,
            remove: remove::<C>,
        }
    }

    /// Creates a new instance with [`serialize`], [`deserialize_mapped`] and [`remove`] functions.
    ///
    /// Always use it for components that contain entities.
    pub fn default_mapped_fns<C>() -> Self
    where
        C: Component + Serialize + DeserializeOwned + MapEntities,
    {
        Self {
            serialize: serialize::<C>,
            deserialize: deserialize_mapped::<C>,
            write: write::<C>,
            remove: remove::<C>,
        }
    }
}

/// Represents ID of [`ComponentFns`].
///
/// Can be obtained from [`ReplicationFns::register_component_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ComponentFnsId(usize);

/// Default component serialization function.
///
/// # Safety
///
/// `C` must be the erased pointee type for this [`Ptr`].
pub unsafe fn serialize<C: Component + Serialize>(
    ptr: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    let component: &C = ptr.deref();
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default component deserialization function.
///
/// # Safety
///
/// [`MaybeUninit<C>`] must be the erased pointee type for this [`Ptr`].
pub unsafe fn deserialize<C: Component + DeserializeOwned>(
    ptr: PtrMut,
    cursor: &mut Cursor<&[u8]>,
    _mapper: &mut ClientMapper,
) -> bincode::Result<()> {
    let component: C = DefaultOptions::new().deserialize_from(cursor)?;
    *ptr.deref_mut() = MaybeUninit::new(component);

    Ok(())
}

/// Like [`deserialize`], but also maps entities before insertion.
///
/// # Safety
///
/// [`MaybeUninit<C>`] must be the erased pointee type for this [`Ptr`].
pub unsafe fn deserialize_mapped<C: Component + DeserializeOwned + MapEntities>(
    ptr: PtrMut,
    cursor: &mut Cursor<&[u8]>,
    mapper: &mut ClientMapper,
) -> bincode::Result<()> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;

    component.map_entities(mapper);

    *ptr.deref_mut() = MaybeUninit::new(component);

    Ok(())
}

/// Default component writing function.
///
/// # Safety
///
/// `deserialize` must be safely callable with [`MaybeUninit<C>`] as [`Ptr`]
/// and should initialize passed component.
pub unsafe fn write<C: Component + DeserializeOwned>(
    commands: &mut Commands,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
    entity_map: &mut ServerEntityMap,
    _replicon_tick: RepliconTick,
    deserialize: DeserializeFn,
) -> bincode::Result<()> {
    let mut mapper = ClientMapper {
        commands,
        entity_map,
    };

    // if let Some(mut component) = entity.get_mut::<C>() {
    //     (deserialize)(PtrMut::from(&mut component), cursor, &mut mapper)?;
    // } else {
    let mut component = MaybeUninit::<C>::uninit();
    (deserialize)(PtrMut::from(&mut component), cursor, &mut mapper)?;
    commands.entity(entity.id()).insert(component.assume_init());
    // }

    Ok(())
}

/// Default component removal function.
pub fn remove<C: Component>(mut entity_commands: EntityCommands, _replicon_tick: RepliconTick) {
    entity_commands.remove::<C>();
}

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.despawn_recursive();
}
