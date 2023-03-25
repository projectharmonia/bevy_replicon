use bevy::{
    ecs::entity::{EntityMap, MapEntities, MapEntitiesError},
    prelude::*,
    reflect::FromType,
};

use crate::replication_core::Replication;

/// Maps server entities to client entities and vice versa.
///
/// Used only on client.
#[derive(Default, Resource)]
pub(crate) struct NetworkEntityMap {
    server_to_client: EntityMap,
    client_to_server: EntityMap,
}

impl NetworkEntityMap {
    #[cfg(test)]
    pub(crate) fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        self.server_to_client.insert(server_entity, client_entity);
        self.client_to_server.insert(client_entity, server_entity);
    }

    pub(crate) fn get_by_server_or_spawn(
        &mut self,
        world: &mut World,
        server_entity: Entity,
    ) -> Entity {
        *self
            .server_to_client
            .entry(server_entity)
            .or_insert_with(|| {
                let client_entity = world.spawn(Replication).id();
                self.client_to_server.insert(client_entity, server_entity);
                client_entity
            })
    }

    pub(crate) fn remove_by_server(
        &mut self,
        server_entity: Entity,
    ) -> Result<Entity, MapEntitiesError> {
        let client_entity = self.server_to_client.remove(server_entity);
        if let Some(client_entity) = client_entity {
            self.client_to_server.remove(client_entity);
        }
        client_entity.ok_or(MapEntitiesError::EntityNotFound(server_entity))
    }

    pub(crate) fn to_client(&self) -> &EntityMap {
        &self.server_to_client
    }

    pub(crate) fn to_server(&self) -> &EntityMap {
        &self.client_to_server
    }
}

/// Like [`bevy::ecs::reflect::ReflectMapEntities`], but maps only a single entity instead of all entities from [`EntityMap`].
#[derive(Clone)]
pub struct ReflectMapEntity {
    map_entities: fn(&mut World, &EntityMap, Entity) -> Result<(), MapEntitiesError>,
}

impl ReflectMapEntity {
    pub(crate) fn map_entities(
        &self,
        world: &mut World,
        entity_map: &EntityMap,
        entity: Entity,
    ) -> Result<(), MapEntitiesError> {
        (self.map_entities)(world, entity_map, entity)
    }
}

impl<C: Component + MapEntities> FromType<C> for ReflectMapEntity {
    fn from_type() -> Self {
        ReflectMapEntity {
            map_entities: |world, entity_map, entity| {
                let mut component = world
                    .get_mut::<C>(entity)
                    .expect("entity should have reflected component");
                component.map_entities(entity_map)?;
                Ok(())
            },
        }
    }
}
