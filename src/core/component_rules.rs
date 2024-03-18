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
        ComponentFns, ComponentFnsIndex, DeserializeFn, RemoveFn, ReplicationFns, SerializeFn,
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
        let component_fns = ComponentFns {
            serialize,
            deserialize,
            remove,
        };

        let mut replication_fns = self.world.resource_mut::<ReplicationFns>();
        let fns_index = replication_fns.add_component_fns(component_fns);

        let mut component_rules = self.world.resource_mut::<ComponentRules>();
        component_rules.ids.insert(component_id, fns_index);

        self
    }
}

/// Stores information about which components will be replicated.
#[derive(Resource)]
pub(crate) struct ComponentRules {
    /// Maps component IDs to their function IDs.
    ids: HashMap<ComponentId, ComponentFnsIndex>,

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

    /// Returns mapping of replicated components to their function IDs.
    #[must_use]
    pub(crate) fn ids(&self) -> &HashMap<ComponentId, ComponentFnsIndex> {
        &self.ids
    }
}

impl FromWorld for ComponentRules {
    fn from_world(world: &mut World) -> Self {
        Self {
            ids: Default::default(),
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
