use bevy::{
    ecs::component::Tick,
    prelude::*,
    utils::{EntityHashMap, HashMap},
};
use bevy_renet::renet::ClientId;

/// Stores meta-information about connected clients.
#[derive(Default, Resource)]
pub(crate) struct ClientsInfo {
    pub(super) info: Vec<ClientInfo>,

    /// [`Vec`]'s from acknowledged update indexes from [`ClientInfo`].
    ///
    /// All data is cleared before the insertion, used just to reuse allocated capacity.
    pub(super) entity_buffer: Vec<Vec<Entity>>,
}

impl ClientsInfo {
    /// Removes info for the client.
    ///
    /// Keeps memory from update entities for reuse.
    pub(super) fn remove(&mut self, client_id: ClientId) {
        let index = self
            .info
            .iter()
            .position(|info| info.id == client_id)
            .expect("clients info should contain all connected clients");
        let mut client_info = self.info.remove(index);
        let old_entities = client_info
            .update_entities
            .drain()
            .map(|(_, (_, mut entities))| {
                entities.clear();
                entities
            });
        self.entity_buffer.extend(old_entities);
    }

    /// Clears information for all clients.
    ///
    /// Keeps memory from update entities for reuse.
    pub(super) fn clear(&mut self) {
        let old_entities = self
            .info
            .drain(..)
            .flat_map(|client_info| client_info.update_entities)
            .map(|(_, (_, mut entities))| {
                entities.clear();
                entities
            });
        self.entity_buffer.extend(old_entities);
    }
}

pub(super) struct ClientInfo {
    pub(super) id: ClientId,
    pub(super) just_connected: bool,
    pub(super) ticks: EntityHashMap<Entity, Tick>,
    pub(super) update_entities: HashMap<u16, (Tick, Vec<Entity>)>,
    next_update_index: u16,
}

impl ClientInfo {
    pub(super) fn new(id: ClientId) -> Self {
        Self {
            id,
            just_connected: true,
            ticks: Default::default(),
            update_entities: Default::default(),
            next_update_index: Default::default(),
        }
    }

    /// Remembers `entities` and `tick` of an update message and returns its index.
    ///
    /// Used later to acknowledge updated entities.
    #[must_use]
    pub(super) fn register_update(&mut self, tick: Tick, entities: Vec<Entity>) -> u16 {
        let update_index = self.next_update_index;
        self.update_entities.insert(update_index, (tick, entities));

        self.next_update_index = self.next_update_index.overflowing_add(1).0;

        update_index
    }
}
