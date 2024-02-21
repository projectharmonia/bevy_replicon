use bevy::{ecs::entity::EntityHashMap, prelude::*, utils::hashbrown::hash_map::Entry};

use crate::replicon_core::replication_rules::Replication;

/// Maps server entities into client entities inside components.
///
/// Spawns new client entity if a mapping doesn't exists.
pub struct ClientMapper<'a> {
    world: &'a mut World,
    server_to_client: &'a mut EntityHashMap<Entity>,
    client_to_server: &'a mut EntityHashMap<Entity>,
}

impl<'a> ClientMapper<'a> {
    #[inline]
    pub fn new(world: &'a mut World, entity_map: &'a mut ServerEntityMap) -> Self {
        Self {
            world,
            server_to_client: &mut entity_map.server_to_client,
            client_to_server: &mut entity_map.client_to_server,
        }
    }
}

impl EntityMapper for ClientMapper<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self.server_to_client.entry(entity).or_insert_with(|| {
            let client_entity = self.world.spawn(Replication).id();
            self.client_to_server.insert(client_entity, entity);
            client_entity
        })
    }
}

/// Maps server entities to client entities and vice versa.
///
/// If [`ClientSet::Reset`](crate::client::ClientSet) is disabled, then this needs to be cleaned up manually
/// via [`Self::remove_by_client`] or [`Self::clear`].
#[derive(Default, Resource)]
pub struct ServerEntityMap {
    server_to_client: EntityHashMap<Entity>,
    client_to_server: EntityHashMap<Entity>,
}

impl ServerEntityMap {
    /// Inserts a server-client pair into the map.
    ///
    /// # Panics
    ///
    /// Panics if this mapping is already present.
    #[inline]
    pub fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        if let Some(existing_entity) = self.server_to_client.insert(server_entity, client_entity) {
            if client_entity != existing_entity {
                panic!("mapping {server_entity:?} to {client_entity:?}, but it's already mapped to {existing_entity:?}");
            } else {
                warn!("received duplicate mapping from {server_entity:?} to {client_entity:?}");
            }
        }
        self.client_to_server.insert(client_entity, server_entity);
    }

    pub(super) fn get_by_server_or_spawn<'a>(
        &mut self,
        world: &'a mut World,
        server_entity: Entity,
    ) -> EntityWorldMut<'a> {
        match self.server_to_client.entry(server_entity) {
            Entry::Occupied(entry) => world.entity_mut(*entry.get()),
            Entry::Vacant(entry) => {
                let client_entity = world.spawn(Replication);
                entry.insert(client_entity.id());
                self.client_to_server
                    .insert(client_entity.id(), server_entity);
                client_entity
            }
        }
    }

    pub(super) fn get_by_server<'a>(
        &mut self,
        world: &'a mut World,
        server_entity: Entity,
    ) -> Option<EntityWorldMut<'a>> {
        self.server_to_client
            .get(&server_entity)
            .map(|&entity| world.entity_mut(entity))
    }

    pub(super) fn remove_by_server(&mut self, server_entity: Entity) -> Option<Entity> {
        let client_entity = self.server_to_client.remove(&server_entity);
        if let Some(client_entity) = client_entity {
            self.client_to_server.remove(&client_entity);
        }
        client_entity
    }

    /// Removes an entry using the client entity.
    ///
    /// Useful for manual cleanup, e.g. after reconnects.
    pub fn remove_by_client(&mut self, client_entity: Entity) -> Option<Entity> {
        let server_entity = self.client_to_server.remove(&client_entity);
        if let Some(server_entity) = server_entity {
            self.server_to_client.remove(&server_entity);
        }
        server_entity
    }

    #[inline]
    pub fn to_client(&self) -> &EntityHashMap<Entity> {
        &self.server_to_client
    }

    #[inline]
    pub fn to_server(&self) -> &EntityHashMap<Entity> {
        &self.client_to_server
    }

    /// Clears the map.
    pub fn clear(&mut self) {
        self.client_to_server.clear();
        self.server_to_client.clear();
    }
}
