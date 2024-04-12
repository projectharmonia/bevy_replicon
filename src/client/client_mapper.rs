use bevy::{ecs::entity::EntityHashMap, prelude::*, utils::hashbrown::hash_map::Entry};

use crate::core::Replication;

/// Maps server entities into client entities inside components.
///
/// Spawns new client entity if a mapping doesn't exists.
pub struct ClientMapper<'a, 'w, 's> {
    pub commands: &'a mut Commands<'w, 's>,
    pub entity_map: &'a mut ServerEntityMap,
}

impl EntityMapper for ClientMapper<'_, '_, '_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self
            .entity_map
            .server_to_client
            .entry(entity)
            .or_insert_with(|| {
                let client_entity = self.commands.spawn(Replication).id();
                self.entity_map
                    .client_to_server
                    .insert(client_entity, entity);
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

    pub(super) fn get_by_server_or_insert(
        &mut self,
        server_entity: Entity,
        f: impl FnOnce() -> Entity,
    ) -> Entity {
        match self.server_to_client.entry(server_entity) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let client_entity = (f)();
                entry.insert(client_entity);
                self.client_to_server.insert(client_entity, server_entity);
                client_entity
            }
        }
    }

    pub(super) fn get_by_server(&mut self, server_entity: Entity) -> Option<Entity> {
        self.server_to_client.get(&server_entity).copied()
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
