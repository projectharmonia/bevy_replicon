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
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    pub(super) entity_buffer: Vec<Vec<Entity>>,

    /// Disconnected client's [`ClientsInfo`].
    ///
    /// [`ClientInfo::clear`] is used before the insertion.
    /// Stored to reuse allocated memory.
    info_buffer: Vec<ClientInfo>,
}

impl ClientsInfo {
    /// Initializes a new [`ClientInfo`] for this client.
    pub(super) fn init(&mut self, client_id: ClientId) {
        let client_info = if let Some(mut client_info) = self.info_buffer.pop() {
            client_info.id = client_id;
            client_info
        } else {
            ClientInfo::new(client_id)
        };

        self.info.push(client_info);
    }

    /// Removes info for the client.
    ///
    /// Keeps allocated memory.
    pub(super) fn remove(&mut self, client_id: ClientId) {
        let index = self
            .info
            .iter()
            .position(|info| info.id == client_id)
            .expect("clients info should contain all connected clients");
        let mut client_info = self.info.remove(index);
        self.entity_buffer.extend(client_info.reset());
        self.info_buffer.push(client_info);
    }

    /// Clears information for all clients.
    ///
    /// Keeps allocated memory.
    pub(super) fn clear(&mut self) {
        for mut client_info in self.info.drain(..) {
            self.entity_buffer.extend(client_info.reset());
            self.info_buffer.push(client_info);
        }
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
    fn new(id: ClientId) -> Self {
        Self {
            id,
            just_connected: true,
            ticks: Default::default(),
            update_entities: Default::default(),
            next_update_index: Default::default(),
        }
    }

    /// Resets all data except `id` and drains all [`Vec`]s from update entities mapping.
    ///
    /// Drained data will be cleared.
    /// Keeps allocated memory.
    fn reset(&mut self) -> impl Iterator<Item = Vec<Entity>> + '_ {
        self.just_connected = true;
        self.ticks.clear();
        self.next_update_index = 0;
        self.update_entities.drain().map(|(_, (_, mut entities))| {
            entities.clear();
            entities
        })
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
