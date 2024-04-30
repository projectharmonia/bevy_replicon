use bevy::{ecs::entity::EntityHashMap, prelude::*, utils::hashbrown::hash_map::Entry};

use crate::Replicated;

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

    /// Converts server entity into client entity or inserts a new mapping with `f`
    ///
    /// # Examples
    ///
    /// ```
    /// # use bevy::{ecs::system::CommandQueue, prelude::*};
    /// # use bevy_replicon::{client::server_entity_map::ServerEntityMap, prelude::*};
    /// # let mut entity_map = ServerEntityMap::default();
    /// # let mut queue = CommandQueue::default();
    /// # let world = World::default();
    /// # let mut commands = Commands::new(&mut queue, &world);
    /// # let server_entity = Entity::PLACEHOLDER;
    /// entity_map.get_by_server_or_insert(server_entity, || commands.spawn(Replicated).id());
    /// ```
    pub fn get_by_server_or_insert(
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

/// Maps server entities into client entities inside components.
///
/// Spawns new client entity using [`Commands`] if a mapping doesn't exists.
/// See also [`ComponentWorldMapper`].
pub struct ComponentMapper<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    entity_map: &'a mut ServerEntityMap,
}

impl<'a, 'w, 's> ComponentMapper<'a, 'w, 's> {
    pub fn new(commands: &'a mut Commands<'w, 's>, entity_map: &'a mut ServerEntityMap) -> Self {
        Self {
            commands,
            entity_map,
        }
    }
}

impl EntityMapper for ComponentMapper<'_, '_, '_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        self.entity_map
            .get_by_server_or_insert(entity, || self.commands.spawn(Replicated).id())
    }
}

/// Maps server entities into client entities inside components.
///
/// Spawns new client entity if a mapping doesn't exists.
/// See also [`ComponentMapper`].
pub struct ComponentWorldMapper<'a> {
    world: &'a mut World,
    entity_map: &'a mut ServerEntityMap,
}

impl<'a> ComponentWorldMapper<'a> {
    pub fn new(world: &'a mut World, entity_map: &'a mut ServerEntityMap) -> Self {
        Self { world, entity_map }
    }
}

impl EntityMapper for ComponentWorldMapper<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        self.entity_map
            .get_by_server_or_insert(entity, || self.world.spawn(Replicated).id())
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::CommandQueue;

    use super::*;

    #[test]
    fn component_mapper_spawn() {
        let mut world = World::default();
        let mut entity_map = ServerEntityMap::default();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut mapper = ComponentMapper::new(&mut commands, &mut entity_map);

        let client_entity = mapper.map_entity(Entity::PLACEHOLDER);
        queue.apply(&mut world);

        assert!(world.get_entity(client_entity).is_some());
        assert!(entity_map.to_server().contains_key(&client_entity));
    }

    #[test]
    fn component_mapper_existing() {
        let mut world = World::default();
        let server_entity = world.spawn_empty().id();
        let client_entity = world.spawn_empty().id();
        let mut entity_map = ServerEntityMap::default();
        entity_map.insert(server_entity, client_entity);

        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut mapper = ComponentMapper::new(&mut commands, &mut entity_map);

        assert_eq!(mapper.map_entity(server_entity), client_entity);
        queue.apply(&mut world);

        assert!(entity_map.to_server().contains_key(&client_entity));
        assert!(entity_map.to_client().contains_key(&server_entity));
        assert_eq!(world.entities().len(), 2);
    }

    #[test]
    fn component_world_mapper_spawn() {
        let mut world = World::default();
        let mut entity_map = ServerEntityMap::default();
        let mut mapper = ComponentWorldMapper::new(&mut world, &mut entity_map);

        let client_entity = mapper.map_entity(Entity::PLACEHOLDER);
        assert!(world.get_entity(client_entity).is_some());
        assert!(entity_map.to_server().contains_key(&client_entity));
    }

    #[test]
    fn component_world_mapper_existing() {
        let mut world = World::default();
        let server_entity = world.spawn_empty().id();
        let client_entity = world.spawn_empty().id();
        let mut entity_map = ServerEntityMap::default();
        entity_map.insert(server_entity, client_entity);
        let mut mapper = ComponentWorldMapper::new(&mut world, &mut entity_map);

        assert_eq!(mapper.map_entity(server_entity), client_entity);
        assert!(entity_map.to_server().contains_key(&client_entity));
        assert!(entity_map.to_client().contains_key(&server_entity));
        assert_eq!(world.entities().len(), 2);
    }
}
