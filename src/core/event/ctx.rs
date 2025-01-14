use bevy::{prelude::*, reflect::TypeRegistry};

use crate::core::server_entity_map::ServerEntityMap;

/// Event sending context for client.
#[non_exhaustive]
pub struct ClientSendCtx<'a> {
    /// Registry of reflected types.
    pub registry: &'a TypeRegistry,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a ServerEntityMap,
}

impl EntityMapper for ClientSendCtx<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self
            .entity_map
            .to_server()
            .get(&entity)
            .unwrap_or_else(|| panic!("client {entity:?} should have a mapping"))
    }
}

/// Event receiving context for server.
#[non_exhaustive]
pub struct ServerReceiveCtx<'a> {
    /// Registry of reflected types.
    pub registry: &'a TypeRegistry,
}

/// Event sending context for server.
#[non_exhaustive]
pub struct ServerSendCtx<'a> {
    /// Registry of reflected types.
    pub registry: &'a TypeRegistry,
}

/// Event receiving context for client.
#[non_exhaustive]
pub struct ClientReceiveCtx<'a> {
    /// Registry of reflected types.
    pub registry: &'a TypeRegistry,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a ServerEntityMap,

    /// Entities that couldn't be mapped by [`EntityMapper::map_entity`].
    ///
    /// We needed it because [`EntityMapper`] doesn't provide a way to handle errors.
    pub(crate) invalid_entities: Vec<Entity>,
}

impl EntityMapper for ClientReceiveCtx<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        if let Some(mapped_entity) = self.entity_map.to_client().get(&entity) {
            *mapped_entity
        } else {
            self.invalid_entities.push(entity);
            Entity::PLACEHOLDER
        }
    }
}
