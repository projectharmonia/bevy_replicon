use std::{io::Cursor, mem};

use bevy::{
    ecs::{
        archetype::{ArchetypeGeneration, Archetypes},
        component::ComponentId,
        entity::MapEntities,
    },
    prelude::*,
    ptr::Ptr,
    utils::HashMap,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::{
    replication_fns::{
        DeserializeFn, RemoveFn, RemoveFnId, ReplicationFns, SerdeFns, SerdeFnsId, SerializeFn,
    },
    replicon_tick::RepliconTick,
};
use crate::client::client_mapper::{ClientMapper, ServerEntityMap};

pub trait AppReplicationExt {
    /// Marks component for replication.
    ///
    /// Component will be serialized as is using bincode.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned;

    /// Same as [`Self::replicate`], but additionally maps server entities to client inside the component after receiving.
    ///
    /// Always use it for components that contain entities.
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapEntities;

    /// Same as [`Self::replicate`], but uses the specified functions for serialization, deserialization, and removal.
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveFn,
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
        C: Component + Serialize + DeserializeOwned + MapEntities,
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
        remove: RemoveFn,
    ) -> &mut Self
    where
        C: Component,
    {
        let component_id = self.world.init_component::<C>();
        let serde_fns = SerdeFns {
            serialize,
            deserialize,
        };

        let mut replication_fns = self.world.resource_mut::<ReplicationFns>();
        let serde_id = replication_fns.add_serde_fns(serde_fns);
        let remove_id = replication_fns.add_remove_fn(remove);

        let mut component_rules = self.world.resource_mut::<ComponentRules>();
        component_rules.serde_ids.insert(component_id, serde_id);
        component_rules.remove_ids.insert(component_id, remove_id);

        self
    }
}

/// Stores information about which components will be replicated.
#[derive(Resource)]
pub struct ComponentRules {
    /// Maps component IDs to their serde functions IDs.
    serde_ids: HashMap<ComponentId, SerdeFnsId>,

    /// Maps component IDs to their remove function IDs.
    remove_ids: HashMap<ComponentId, RemoveFnId>,

    /// ID of [`Replication`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    ///
    /// See also [`Self::update_generation`].
    generation: ArchetypeGeneration,
}

impl ComponentRules {
    /// Replaces stored generation with the highest archetype ID and returns previous.
    ///
    /// This should be used to iterate over newly introduced
    /// [`Archetype`](bevy::ecs::archetype::Archetypes)s since the last time this function was called.
    pub(crate) fn update_generation(&mut self, archetypes: &Archetypes) -> ArchetypeGeneration {
        mem::replace(&mut self.generation, archetypes.generation())
    }

    /// ID of [`Replication`] component.
    #[must_use]
    pub(crate) fn marker_id(&self) -> ComponentId {
        self.marker_id
    }

    /// Returns mapping of replicated components to their serde function IDs.
    #[must_use]
    pub(crate) fn serde_ids(&self) -> &HashMap<ComponentId, SerdeFnsId> {
        &self.serde_ids
    }

    /// Returns mapping of replicated components to their remove function IDs.
    #[must_use]
    pub(crate) fn remove_ids(&self) -> &HashMap<ComponentId, RemoveFnId> {
        &self.remove_ids
    }
}

impl FromWorld for ComponentRules {
    fn from_world(world: &mut World) -> Self {
        Self {
            serde_ids: Default::default(),
            remove_ids: Default::default(),
            marker_id: world.init_component::<Replication>(),
            generation: ArchetypeGeneration::initial(),
        }
    }
}

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replication;

/// Default serialization function.
pub fn serialize_component<C: Component + Serialize>(
    component: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    // SAFETY: Function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default deserialization function.
pub fn deserialize_component<C: Component + DeserializeOwned>(
    entity: &mut EntityWorldMut,
    _entity_map: &mut ServerEntityMap,
    cursor: &mut Cursor<&[u8]>,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let component: C = DefaultOptions::new().deserialize_from(cursor)?;
    entity.insert(component);

    Ok(())
}

/// Like [`deserialize_component`], but also maps entities before insertion.
pub fn deserialize_mapped_component<C: Component + DeserializeOwned + MapEntities>(
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

/// Default component removal function.
pub fn remove_component<C: Component>(entity: &mut EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.remove::<C>();
}
