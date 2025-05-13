use bevy::{
    ecs::entity::{EntityHash, hash_map::EntityHashMap},
    platform::collections::hash_map::{self, Entry},
    prelude::*,
};
use log::{error, warn};

/// Maps server entities to client entities and vice versa.
///
/// If [`ClientSet::Reset`](crate::client::ClientSet) is disabled, then this needs to be cleaned up manually
/// by removing entries via [`EntityEntry::remove`] or [`Self::clear`].
///
/// Inserted as resource by [`ClientPlugin`](crate::client::ClientPlugin).
#[derive(Default, Resource)]
pub struct ServerEntityMap {
    server_to_client: EntityHashMap<Entity>,
    client_to_server: EntityHashMap<Entity>,
}

impl ServerEntityMap {
    /// Inserts a server-client pair into the map.
    #[inline]
    pub fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        self.server_entry(server_entity)
            .or_insert_with(|| client_entity);
    }

    /// Returns server to client mappings.
    #[inline]
    pub fn to_client(&self) -> &EntityHashMap<Entity> {
        &self.server_to_client
    }

    /// Returns client to server mappings.
    #[inline]
    pub fn to_server(&self) -> &EntityHashMap<Entity> {
        &self.client_to_server
    }

    /// Gets a client entry using the server entity.
    pub fn client_entry(&mut self, client_entity: Entity) -> EntityEntry {
        EntityEntry::new(
            self.client_to_server.entry(client_entity),
            &mut self.server_to_client,
        )
    }

    /// Gets a server entry using the client entity.
    pub fn server_entry(&mut self, server_entity: Entity) -> EntityEntry {
        EntityEntry::new(
            self.server_to_client.entry(server_entity),
            &mut self.client_to_server,
        )
    }

    /// Clears the map.
    pub fn clear(&mut self) {
        self.client_to_server.clear();
        self.server_to_client.clear();
    }
}

/// A view into an entry in [`ServerEntityMap`].
#[must_use]
pub enum EntityEntry<'a> {
    Occupied(OccupiedEntityEntry<'a>),
    Vacant(VacantEntityEntry<'a>),
}

impl<'a> EntityEntry<'a> {
    fn new(
        main_entry: Entry<'a, Entity, Entity, EntityHash>,
        reverse_map: &'a mut EntityHashMap<Entity>,
    ) -> Self {
        match main_entry {
            Entry::Occupied(main_entry) => Self::Occupied(OccupiedEntityEntry {
                main_entry,
                reverse_map,
            }),
            Entry::Vacant(main_entry) => Self::Vacant(VacantEntityEntry {
                main_entry,
                reverse_map,
            }),
        }
    }

    /// Returns the mappend entity for the entry.
    pub fn get(&self) -> Option<Entity> {
        match self {
            EntityEntry::Occupied(entry) => Some(entry.get()),
            EntityEntry::Vacant(_) => None,
        }
    }

    /// Removes the entry and returns the mapped entity.
    pub fn remove(self) -> Option<Entity> {
        match self {
            EntityEntry::Occupied(entry) => Some(entry.remove()),
            EntityEntry::Vacant(_) => None,
        }
    }

    /// Inserts the mapped entity from the function and returns it.
    pub fn or_insert_with<F: FnOnce() -> Entity>(self, f: F) -> Entity {
        match self {
            EntityEntry::Occupied(entry) => entry.get(),
            EntityEntry::Vacant(entry) => entry.insert(f()),
        }
    }
}

/// A view into an occupied entry in [`ServerEntityMap`].
///
/// It's part of [`EntityEntry`] enum.
pub struct OccupiedEntityEntry<'a> {
    main_entry: hash_map::OccupiedEntry<'a, Entity, Entity, EntityHash>,
    reverse_map: &'a mut EntityHashMap<Entity>,
}

impl OccupiedEntityEntry<'_> {
    /// Returns the mappend entity for the entry.
    pub fn get(&self) -> Entity {
        *self.main_entry.get()
    }

    /// Sets the mapped entity for the entry and returns the old mapping.
    pub fn insert(&mut self, value: Entity) -> Entity {
        let key = *self.main_entry.key();
        let old_value = self.main_entry.insert(value);
        if value != old_value {
            error!("mapping {key:?} to {value:?}, but it's already mapped to {old_value:?}");
        } else {
            warn!("ignoring duplicate mapping from {key:?} to {value:?}");
            return value;
        }

        self.reverse_map.remove(&old_value);
        self.reverse_map.insert(value, *self.main_entry.key());
        old_value
    }

    /// Removes the entry and returns the mapped entity.
    pub fn remove(self) -> Entity {
        let (_, value) = self.main_entry.remove_entry();
        self.reverse_map.remove(&value);
        value
    }
}

/// A view into a vacant entry in [`ServerEntityMap`].
///
/// It's part of [`EntityEntry`] enum.
pub struct VacantEntityEntry<'a> {
    main_entry: hash_map::VacantEntry<'a, Entity, Entity, EntityHash>,
    reverse_map: &'a mut EntityHashMap<Entity>,
}

impl VacantEntityEntry<'_> {
    /// Sets the mapped entity for the entry and returns it.
    pub fn insert(self, value: Entity) -> Entity {
        let key = *self.main_entry.key();
        self.main_entry.insert(value);
        self.reverse_map.insert(value, key);
        value
    }
}
