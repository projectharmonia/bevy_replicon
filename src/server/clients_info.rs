use std::mem;

use bevy::{
    ecs::component::Tick,
    prelude::*,
    utils::{Duration, EntityHashMap, HashMap},
};
use bevy_renet::renet::ClientId;

use crate::replicon_core::replicon_tick::RepliconTick;

/// Stores meta-information about connected clients.
#[derive(Default, Resource)]
pub struct ClientsInfo(Vec<ClientInfo>);

/// Reusable buffers for [`ClientsInfo`] and [`ClientInfo`].
#[derive(Default, Resource)]
pub(crate) struct ClientBuffers {
    /// [`ClientsInfo`]'s of previously disconnected clients.
    ///
    /// Stored to reuse allocated memory.
    info: Vec<ClientInfo>,

    /// [`Vec`]'s from acknowledged update indexes from [`ClientInfo`].
    ///
    /// Stored to reuse allocated capacity.
    entities: Vec<Vec<Entity>>,
}

impl ClientsInfo {
    /// Returns an iterator over clients information.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &ClientInfo> {
        self.0.iter()
    }

    /// Returns a mutable iterator over clients information.
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut ClientInfo> {
        self.0.iter_mut()
    }

    /// Returns number of connected clients.
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Initializes a new [`ClientInfo`] for this client.
    ///
    /// Reuses the memory from the buffers if available.
    pub(super) fn init(&mut self, client_buffers: &mut ClientBuffers, client_id: ClientId) {
        let client_info = if let Some(mut client_info) = client_buffers.info.pop() {
            client_info.reset(client_id);
            client_info
        } else {
            ClientInfo::new(client_id)
        };

        self.0.push(client_info);
    }

    /// Removes info for the client.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(super) fn remove(&mut self, client_buffers: &mut ClientBuffers, client_id: ClientId) {
        let index = self
            .0
            .iter()
            .position(|info| info.id == client_id)
            .expect("clients info should contain all connected clients");
        let mut client_info = self.0.remove(index);
        client_buffers.entities.extend(client_info.drain_entities());
        client_buffers.info.push(client_info);
    }

    /// Clears information for all clients.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(super) fn clear(&mut self, client_buffers: &mut ClientBuffers) {
        for mut client_info in self.0.drain(..) {
            client_buffers.entities.extend(client_info.drain_entities());
            client_buffers.info.push(client_info);
        }
    }
}

pub(crate) struct ClientInfo {
    /// Client's ID.
    id: ClientId,

    /// Indicates whether the client connected in this tick.
    pub(super) just_connected: bool,

    /// Last acknowledged tick for each entity.
    pub(super) ticks: EntityHashMap<Entity, Tick>,

    /// The last tick in which a replicated entity was spawned, despawned, or gained/lost a component.
    ///
    /// It should be included in update messages and server events to avoid needless waiting for the next init message to arrive.
    pub(crate) change_tick: RepliconTick,

    /// Update message indexes mapped to their info.
    updates: HashMap<u16, UpdateInfo>,

    /// Index for the next update message to be sent to this client.
    ///
    /// See also [`Self::register_update`].
    next_update_index: u16,
}

impl ClientInfo {
    fn new(id: ClientId) -> Self {
        Self {
            id,
            just_connected: true,
            ticks: Default::default(),
            change_tick: Default::default(),
            updates: Default::default(),
            next_update_index: Default::default(),
        }
    }

    // Returns associated client ID.
    pub(crate) fn id(&self) -> ClientId {
        self.id
    }

    /// Clears all entities for unacknowledged updates, returning them as an iterator.
    ///
    /// Keeps the allocated memory for reuse.
    fn drain_entities(&mut self) -> impl Iterator<Item = Vec<Entity>> + '_ {
        self.updates
            .drain()
            .map(|(_, update_info)| update_info.entities)
    }

    /// Resets all data.
    ///
    /// Keeps the allocated memory for reuse.
    fn reset(&mut self, id: ClientId) {
        self.id = id;
        self.just_connected = true;
        self.ticks.clear();
        self.updates.clear();
        self.next_update_index = 0;
    }

    /// Registers update at specified `tick` and `timestamp` and returns its index with entities to fill.
    ///
    /// Used later to acknowledge updated entities.
    #[must_use]
    pub(super) fn register_update(
        &mut self,
        client_buffers: &mut ClientBuffers,
        tick: Tick,
        timestamp: Duration,
    ) -> (u16, &mut Vec<Entity>) {
        let update_index = self.next_update_index;
        self.next_update_index = self.next_update_index.overflowing_add(1).0;

        let mut entities = client_buffers.entities.pop().unwrap_or_default();
        entities.clear();
        let update_info = UpdateInfo {
            tick,
            timestamp,
            entities,
        };
        let update_info = self
            .updates
            .entry(update_index)
            .insert(update_info)
            .into_mut();

        (update_index, &mut update_info.entities)
    }

    /// Marks update with the specified index as acknowledged.
    ///
    /// Ticks for all entities from this update will be set to the update's tick if it's higher.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(super) fn acknowledge(
        &mut self,
        client_buffers: &mut ClientBuffers,
        tick: Tick,
        update_index: u16,
    ) {
        let Some(update_info) = self.updates.remove(&update_index) else {
            debug!(
                "received unknown update index {update_index} from client {}",
                self.id
            );
            return;
        };

        for entity in &update_info.entities {
            let last_tick = self
                .ticks
                .get_mut(entity)
                .expect("tick should be inserted on any component insertion");

            // Received tick could be outdated because we bump it
            // if we detect any insertion on the entity in `collect_changes`.
            if !last_tick.is_newer_than(update_info.tick, tick) {
                *last_tick = update_info.tick;
            }
        }
        client_buffers.entities.push(update_info.entities);

        trace!(
            "client {} acknowledged an update with {:?}",
            self.id,
            update_info.tick,
        );
    }

    /// Removes all updates older then `min_timestamp`.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(super) fn remove_older_updates(
        &mut self,
        client_buffers: &mut ClientBuffers,
        min_timestamp: Duration,
    ) {
        self.updates.retain(|_, update_info| {
            if update_info.timestamp < min_timestamp {
                client_buffers
                    .entities
                    .push(mem::take(&mut update_info.entities));
                false
            } else {
                true
            }
        });
    }
}

struct UpdateInfo {
    tick: Tick,
    timestamp: Duration,
    entities: Vec<Entity>,
}
