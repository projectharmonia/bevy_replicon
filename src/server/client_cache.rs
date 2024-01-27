pub mod client_visibility;

use std::mem;

use bevy::{
    ecs::component::Tick,
    prelude::*,
    utils::{Duration, EntityHashMap, HashMap},
};
use bevy_renet::renet::ClientId;

use crate::{replicon_core::replicon_tick::RepliconTick, server::VisibilityPolicy};
use client_visibility::ClientVisibility;

/// Stores information about connected clients.
#[derive(Resource, Default)]
pub struct ClientCache {
    states: Vec<ClientState>,
    policy: VisibilityPolicy,
}

impl ClientCache {
    pub(super) fn new(policy: VisibilityPolicy) -> Self {
        Self {
            states: Default::default(),
            policy,
        }
    }

    /// Returns the configured [`VisibilityPolicy`].
    pub fn visibility_policy(&self) -> VisibilityPolicy {
        self.policy
    }

    /// Returns a reference to a connected client's state.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::get_client`] for the fallible version.
    ///
    /// # Panics
    ///
    /// Panics if the passed client ID is not connected.
    pub fn client(&self, client_id: ClientId) -> &ClientState {
        self.get_client(client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be connected"))
    }

    /// Returns a mutable reference to a connected client's state.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::get_client_mut`] for the fallible version.
    ///
    /// # Panics
    ///
    /// Panics if the passed client ID is not connected.
    pub fn client_mut(&mut self, client_id: ClientId) -> &mut ClientState {
        self.get_client_mut(client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be connected"))
    }

    /// Returns a reference to a connected client's state.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::client`] for the panicking version.
    pub fn get_client(&self, client_id: ClientId) -> Option<&ClientState> {
        self.states
            .iter()
            .find(|client_state| client_state.id == client_id)
    }

    /// Returns a mutable reference to a connected client's state.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::client`] for the panicking version.
    pub fn get_client_mut(&mut self, client_id: ClientId) -> Option<&mut ClientState> {
        self.states
            .iter_mut()
            .find(|client_state| client_state.id == client_id)
    }

    /// Returns an iterator over client states.
    pub fn iter(&self) -> impl Iterator<Item = &ClientState> {
        self.states.iter()
    }

    /// Returns a mutable iterator over client states.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ClientState> {
        self.states.iter_mut()
    }

    /// Returns the number of connected clients.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Returns `true` if no clients are connected.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Initializes a new [`ClientState`] for this client.
    ///
    /// Reuses the memory from the buffers if available.
    pub(super) fn init(&mut self, client_buffers: &mut ClientBuffers, client_id: ClientId) {
        let client_state = if let Some(mut client_state) = client_buffers.states.pop() {
            client_state.reset(client_id);
            client_state
        } else {
            ClientState::new(client_id, self.policy)
        };

        self.states.push(client_state);
    }

    /// Removes state for the client.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(super) fn remove(&mut self, client_buffers: &mut ClientBuffers, client_id: ClientId) {
        let index = self
            .states
            .iter()
            .position(|client_state| client_state.id == client_id)
            .expect("client cache should contain all connected clients");
        let mut client_state = self.states.remove(index);
        client_buffers
            .entities
            .extend(client_state.drain_entities());
        client_buffers.states.push(client_state);
    }

    /// Clears states for all clients.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(super) fn clear(&mut self, client_buffers: &mut ClientBuffers) {
        for mut client_state in self.states.drain(..) {
            client_buffers
                .entities
                .extend(client_state.drain_entities());
            client_buffers.states.push(client_state);
        }
    }
}

pub struct ClientState {
    /// Client's ID.
    id: ClientId,

    /// Lowest tick for use in change detection for each entity.
    ticks: EntityHashMap<Entity, Tick>,

    /// Entity visibility settings.
    visibility: ClientVisibility,

    /// The last tick in which a replicated entity was spawned, despawned, or gained/lost a component from the
    /// perspective of the client.
    ///
    /// It should be included in update messages and server events to avoid needless waiting for the next init
    /// message to arrive.
    change_tick: RepliconTick,

    /// Update message indexes mapped to their info.
    updates: HashMap<u16, UpdateInfo>,

    /// Index for the next update message to be sent to this client.
    ///
    /// See also [`Self::register_update`].
    next_update_index: u16,
}

impl ClientState {
    fn new(id: ClientId, policy: VisibilityPolicy) -> Self {
        Self {
            id,
            ticks: Default::default(),
            visibility: ClientVisibility::new(policy),
            change_tick: Default::default(),
            updates: Default::default(),
            next_update_index: Default::default(),
        }
    }

    // Returns associated client ID.
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Returns a reference to the client's visibility settings.
    pub fn visibility(&self) -> &ClientVisibility {
        &self.visibility
    }

    /// Returns a mutable reference to the client's visibility settings.
    pub fn visibility_mut(&mut self) -> &mut ClientVisibility {
        &mut self.visibility
    }

    /// Sets the client's change tick.
    pub(super) fn set_change_tick(&mut self, new: RepliconTick) {
        self.change_tick = new;
    }

    /// Returns the last tick in which a replicated entity was spawned, despawned, or gained/lost a component from the
    /// perspective of the client.
    pub fn change_tick(&self) -> RepliconTick {
        self.change_tick
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
        self.visibility.clear();
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

    /// Sets the change limit for an entity that is replicated to this client.
    ///
    /// The change limit is the reference point for determining if components on an entity have changed and
    /// need to be replicated. Component changes older than the change limit are assumed to be acked by the client.
    pub(super) fn set_change_limit(&mut self, entity: Entity, tick: Tick) {
        self.ticks.insert(entity, tick);
    }

    /// Gets the change limit for an entity that is replicated to this client.
    pub fn get_change_limit(&mut self, entity: Entity) -> Option<Tick> {
        self.ticks.get(&entity).copied()
    }

    /// Marks update with the specified index as acknowledged.
    ///
    /// Change limits for all entities from this update will be set to the update's tick if it's higher.
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
            let Some(last_tick) = self.ticks.get_mut(entity) else {
                // We ignore missing entities, since they were probably despawned.
                continue;
            };

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

    /// Removes a despawned entity tracked by this client.
    pub fn remove_despawned(&mut self, entity: Entity) {
        self.ticks.remove(&entity);
        self.visibility.remove_despawned(entity);
        // We don't clean up `self.updates` for efficiency reasons.
        // `Self::acknowledge()` will properly ignore despawned entities.
    }

    /// Drains all entities for which visibility was lost during this tick.
    ///
    /// Internal cleanup happens lazily during the iteration.
    pub(super) fn drain_lost_visibility(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.visibility.drain_lost_visibility().inspect(|entity| {
            self.ticks.remove(entity);
        })
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

/// Reusable buffers for [`ClientCache`] and [`ClientState`].
#[derive(Default, Resource)]
pub(crate) struct ClientBuffers {
    /// [`ClientState`]'s of previously disconnected clients.
    ///
    /// Stored to reuse allocated memory.
    states: Vec<ClientState>,

    /// [`Vec`]'s from acknowledged update indexes from [`ClientState`].
    ///
    /// Stored to reuse allocated capacity.
    entities: Vec<Vec<Entity>>,
}

struct UpdateInfo {
    tick: Tick,
    timestamp: Duration,
    entities: Vec<Entity>,
}
