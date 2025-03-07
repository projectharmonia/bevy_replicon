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
    fn get_mapped(&mut self, source: Entity) -> Entity {
        *self
            .entity_map
            .to_server()
            .get(&source)
            .unwrap_or_else(|| panic!("client {source:?} should have a mapping"))
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {
        unimplemented!()
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

    /// Entities that couldn't be mapped by [`EntityMapper::get_mapped`].
    ///
    /// We needed it because [`EntityMapper`] doesn't provide a way to handle errors.
    pub(crate) invalid_entities: Vec<Entity>,
}

impl EntityMapper for ClientReceiveCtx<'_> {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        if let Some(mapped_entity) = self.entity_map.to_client().get(&source) {
            *mapped_entity
        } else {
            self.invalid_entities.push(source);
            Entity::PLACEHOLDER
        }
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {
        unimplemented!()
    }
}
