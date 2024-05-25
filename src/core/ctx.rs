use bevy::{prelude::*, reflect::TypeRegistry};

use crate::{
    client::server_entity_map::ServerEntityMap, core::replicon_tick::RepliconTick, Replicated,
};

/// Replication context for serialization function.
#[non_exhaustive]
pub struct SerializeCtx {
    /// Current tick.
    pub server_tick: RepliconTick,
}

/// Replication context for writing and deserialization.
#[non_exhaustive]
pub struct WriteCtx<'a, 'w, 's> {
    /// A queue to perform structural changes to the [`World`].
    pub commands: &'a mut Commands<'w, 's>,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a mut ServerEntityMap,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,

    /// Disables mapping logic to avoid spawning entities for consume functions.
    pub(super) ignore_mapping: bool,
}

impl<'a, 'w, 's> WriteCtx<'a, 'w, 's> {
    pub(crate) fn new(
        commands: &'a mut Commands<'w, 's>,
        entity_map: &'a mut ServerEntityMap,
        message_tick: RepliconTick,
    ) -> Self {
        Self {
            commands,
            entity_map,
            message_tick,
            ignore_mapping: false,
        }
    }
}

impl EntityMapper for WriteCtx<'_, '_, '_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        if self.ignore_mapping {
            return entity;
        }

        self.entity_map
            .get_by_server_or_insert(entity, || self.commands.spawn(Replicated).id())
    }
}

/// Replication context for removal.
#[non_exhaustive]
pub struct RemoveCtx<'a, 'w, 's> {
    /// A queue to perform structural changes to the [`World`].
    pub commands: &'a mut Commands<'w, 's>,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

impl<'a, 'w, 's> RemoveCtx<'a, 'w, 's> {
    pub(crate) fn new(commands: &'a mut Commands<'w, 's>, message_tick: RepliconTick) -> Self {
        Self {
            commands,
            message_tick,
        }
    }
}

/// Replication context for despawn.
#[non_exhaustive]
pub struct DespawnCtx {
    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

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
}

impl EntityMapper for ClientReceiveCtx<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self
            .entity_map
            .to_client()
            .get(&entity)
            .unwrap_or_else(|| panic!("server {entity:?} should have a mapping"))
    }
}
