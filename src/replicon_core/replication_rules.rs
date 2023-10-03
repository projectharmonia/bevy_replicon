use std::{io::Cursor, marker::PhantomData};

use bevy::{
    ecs::{component::ComponentId, world::EntityMut},
    prelude::*,
    ptr::Ptr,
    utils::HashMap,
};
use bevy_renet::renet::Bytes;
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::NetworkTick;
use crate::client::{ClientMapper, NetworkEntityMap};

pub trait AppReplicationExt {
    /// Marks component for replication.
    ///
    /// Component will be serialized as is using bincode.
    /// It also registers [`Ignored<T>`] that can be used to exclude the component from replication.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned;

    /// Same as [`Self::replicate`], but maps component entities using [`MapNetworkEntities`] trait.
    ///
    /// Always use it for components that contains entities.
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapNetworkEntities;

    /// Same as [`Self::replicate`], but uses the specified functions for serialization, deserialization, and removal.
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveComponentFn,
    ) -> &mut Self
    where
        C: Component;
}

impl AppReplicationExt for App {
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.replicate_with::<C>(
            serialize_component::<C>,
            deserialize_component::<C>,
            remove_component::<C>,
        )
    }

    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapNetworkEntities,
    {
        self.replicate_with::<C>(
            serialize_component::<C>,
            deserialize_mapped_component::<C>,
            remove_component::<C>,
        )
    }

    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveComponentFn,
    ) -> &mut Self
    where
        C: Component,
    {
        let component_id = self.world.init_component::<C>();
        let ignored_id = self.world.init_component::<Ignored<C>>();
        let replicated_component = ReplicationInfo {
            ignored_id,
            serialize,
            deserialize,
            remove,
        };

        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules.infos.push(replicated_component);

        let replication_id = ReplicationId(replication_rules.infos.len() - 1);
        replication_rules.ids.insert(component_id, replication_id);

        self
    }
}

/// Stores information about which components will be serialized and how.
#[derive(Resource)]
pub struct ReplicationRules {
    /// Custom function to handle entity despawning.
    ///
    /// By default uses [`despawn_recursive`].
    /// Useful if you need to intercept despawns and handle them in a special way.
    pub despawn_fn: EntityDespawnFn,

    /// Maps component IDs to their replication IDs.
    ids: HashMap<ComponentId, ReplicationId>,

    /// Meta information about components that should be replicated.
    infos: Vec<ReplicationInfo>,

    /// ID of [`Replication`] component.
    marker_id: ComponentId,
}

impl ReplicationRules {
    /// ID of [`Replication`] component.
    pub(crate) fn get_marker_id(&self) -> ComponentId {
        self.marker_id
    }

    /// Returns mapping of replicated components to their replication IDs.
    pub(crate) fn get_ids(&self) -> &HashMap<ComponentId, ReplicationId> {
        &self.ids
    }

    /// Returns replication ID and meta information about the component if it's replicated.
    pub(crate) fn get(
        &self,
        component_id: ComponentId,
    ) -> Option<(ReplicationId, &ReplicationInfo)> {
        let replication_id = self.ids.get(&component_id).copied()?;
        // SAFETY: ID corresponds to a valid index because it obtained from `ids`.
        let replication_info = unsafe { self.infos.get_unchecked(replication_id.0) };

        Some((replication_id, replication_info))
    }

    /// Returns meta information about replicated component.
    ///
    /// # Safety
    ///
    /// `replication_id` should come from the same replication rules.
    pub(crate) unsafe fn get_info_unchecked(
        &self,
        replication_id: ReplicationId,
    ) -> &ReplicationInfo {
        self.infos.get_unchecked(replication_id.0)
    }
}

impl FromWorld for ReplicationRules {
    fn from_world(world: &mut World) -> Self {
        Self {
            infos: Default::default(),
            ids: Default::default(),
            marker_id: world.init_component::<Replication>(),
            despawn_fn: despawn_recursive,
        }
    }
}

/// Signature of component serialization functions.
pub type SerializeFn = fn(Ptr, &mut Cursor<Vec<u8>>) -> Result<(), bincode::Error>;

/// Signature of component deserialization functions.
pub type DeserializeFn = fn(
    &mut EntityMut,
    &mut NetworkEntityMap,
    &mut Cursor<Bytes>,
    NetworkTick,
) -> Result<(), bincode::Error>;

/// Signature of component removal functions.
pub type RemoveComponentFn = fn(&mut EntityMut, NetworkTick);

/// Signature of the entity despawn function.
pub type EntityDespawnFn = fn(EntityMut, NetworkTick);

/// Stores meta information about replicated component.
pub(crate) struct ReplicationInfo {
    /// ID of [`Ignored<T>`] component.
    pub(crate) ignored_id: ComponentId,

    /// Function that serializes component into bytes.
    pub(crate) serialize: SerializeFn,

    /// Function that deserializes component from bytes and inserts it to [`EntityMut`].
    pub(crate) deserialize: DeserializeFn,

    /// Function that removes specific component from [`EntityMut`].
    pub(crate) remove: RemoveComponentFn,
}

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default)]
pub struct Replication;

/// Replication will be ignored for `T` if this component is present on the same entity.
#[derive(Component)]
pub struct Ignored<T>(PhantomData<T>);

impl<T> Default for Ignored<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Same as [`ComponentId`], but consistent between server and clients.
///
/// Internally represents index of [`ReplicationInfo`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct ReplicationId(usize);

/// Maps entities inside component.
///
/// The same as [`bevy::ecs::entity::MapEntities`], but never creates new entities on mapping error.
pub trait MapNetworkEntities {
    /// Maps stored entities using specified map.
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T);
}

pub trait Mapper {
    fn map(&mut self, entity: Entity) -> Entity;
}

/// Default serialization function.
pub fn serialize_component<C: Component + Serialize>(
    component: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> Result<(), bincode::Error> {
    // SAFETY: Function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default deserialization function.
pub fn deserialize_component<C: Component + DeserializeOwned>(
    entity: &mut EntityMut,
    _entity_map: &mut NetworkEntityMap,
    cursor: &mut Cursor<Bytes>,
    _tick: NetworkTick,
) -> Result<(), bincode::Error> {
    let component: C = DefaultOptions::new().deserialize_from(cursor)?;
    entity.insert(component);

    Ok(())
}

/// Like [`deserialize_component`], but also maps entities before insertion.
pub fn deserialize_mapped_component<C: Component + DeserializeOwned + MapNetworkEntities>(
    entity: &mut EntityMut,
    entity_map: &mut NetworkEntityMap,
    cursor: &mut Cursor<Bytes>,
    _tick: NetworkTick,
) -> Result<(), bincode::Error> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;

    entity.world_scope(|world| {
        component.map_entities(&mut ClientMapper::new(world, entity_map));
    });

    entity.insert(component);

    Ok(())
}

/// Default component removal function.
pub fn remove_component<C: Component>(entity: &mut EntityMut, _tick: NetworkTick) {
    entity.remove::<C>();
}

/// Default entity despawn function.
pub fn despawn_recursive(entity: EntityMut, _tick: NetworkTick) {
    entity.despawn_recursive();
}
