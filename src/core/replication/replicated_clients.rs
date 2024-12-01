pub mod client_visibility;

use std::mem;

use bevy::{
    ecs::{component::Tick, entity::EntityHashMap},
    prelude::*,
    utils::{Duration, HashMap},
};

use crate::core::{replicon_tick::RepliconTick, ClientId};

use client_visibility::ClientVisibility;

/// Stores information about connected clients which are enabled for replication.
///
/// Inserted as resource by [`ServerPlugin`](crate::server::ServerPlugin).
///
/// See also [ConnectedClients](crate::core::connected_clients::ConnectedClients).
#[derive(Resource, Default)]
pub struct ReplicatedClients {
    clients: Vec<ReplicatedClient>,
    policy: VisibilityPolicy,
    replicate_after_connect: bool,
}

impl ReplicatedClients {
    /// Makes a new replicated clients struct.
    ///
    /// Generally you should not need this except in testing contexts.
    pub fn new(policy: VisibilityPolicy, replicate_after_connect: bool) -> Self {
        Self {
            clients: Default::default(),
            policy,
            replicate_after_connect,
        }
    }

    /// Returns the configured [`VisibilityPolicy`].
    pub fn visibility_policy(&self) -> VisibilityPolicy {
        self.policy
    }

    /// Returns if clients will automatically have replication enabled for them after they connect.
    pub fn replicate_after_connect(&self) -> bool {
        self.replicate_after_connect
    }

    /// Returns a reference to a connected client.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::get_client`] for the fallible version.
    ///
    /// # Panics
    ///
    /// Panics if the passed client ID is not connected.
    pub fn client(&self, client_id: ClientId) -> &ReplicatedClient {
        self.get_client(client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be connected"))
    }

    /// Returns a mutable reference to a connected client.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::get_client_mut`] for the fallible version.
    ///
    /// # Panics
    ///
    /// Panics if the passed client ID is not connected.
    pub fn client_mut(&mut self, client_id: ClientId) -> &mut ReplicatedClient {
        self.get_client_mut(client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be connected"))
    }

    /// Returns a reference to a connected client.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::client`] for the panicking version.
    pub fn get_client(&self, client_id: ClientId) -> Option<&ReplicatedClient> {
        self.clients.iter().find(|client| client.id == client_id)
    }

    /// Returns a mutable reference to a connected client.
    ///
    /// This operation is *O*(*n*).
    /// See also [`Self::client`] for the panicking version.
    pub fn get_client_mut(&mut self, client_id: ClientId) -> Option<&mut ReplicatedClient> {
        self.clients
            .iter_mut()
            .find(|client| client.id == client_id)
    }

    /// Returns an iterator over client IDs.
    pub fn iter_client_ids(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients.iter().map(|client| client.id())
    }

    /// Returns an iterator over connected clients.
    pub fn iter(&self) -> impl Iterator<Item = &ReplicatedClient> {
        self.clients.iter()
    }

    /// Returns a mutable iterator over connected clients.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ReplicatedClient> {
        self.clients.iter_mut()
    }

    /// Returns the number of connected clients.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Returns `true` if no clients are connected.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Initializes a new [`ReplicatedClient`] for this client.
    ///
    /// Reuses the memory from the buffers if available.
    pub(crate) fn add(&mut self, client_buffers: &mut ClientBuffers, client_id: ClientId) {
        if self.clients.iter().any(|client| client.id == client_id) {
            warn!("ignoring attempt to start replication for `{client_id:?}` that already has replication enabled");
            return;
        }

        debug!("starting replication for `{client_id:?}`");

        let client = if let Some(mut client) = client_buffers.clients.pop() {
            client.reset(client_id);
            client
        } else {
            ReplicatedClient::new(client_id, self.policy)
        };

        self.clients.push(client);
    }

    /// Removes a replicated client if replication has already been enabled for it.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(crate) fn remove(&mut self, client_buffers: &mut ClientBuffers, client_id: ClientId) {
        let Some(index) = self
            .clients
            .iter()
            .position(|client| client.id == client_id)
        else {
            // It's valid to remove a client which is connected but not replicating yet,
            // which is just a no-op.
            return;
        };

        debug!("stopping replication for `{client_id:?}`");
        let mut client = self.clients.remove(index);
        client_buffers.entities.extend(client.drain_entities());
        client_buffers.clients.push(client);
    }

    /// Clears all clients.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(crate) fn clear(&mut self, client_buffers: &mut ClientBuffers) {
        for mut client in self.clients.drain(..) {
            client_buffers.entities.extend(client.drain_entities());
            client_buffers.clients.push(client);
        }
    }
}

pub struct ReplicatedClient {
    /// Client's ID.
    id: ClientId,

    /// Lowest tick for use in change detection for each entity.
    mutation_ticks: EntityHashMap<Tick>,

    /// Entity visibility settings.
    visibility: ClientVisibility,

    /// The last tick in which a replicated entity had an insertion, removal, or gained/lost a component from the
    /// perspective of the client.
    ///
    /// It should be included in mutate messages and server events to avoid needless waiting for the next update
    /// message to arrive.
    update_tick: RepliconTick,

    /// Mutate message indices mapped to their info.
    mutations: HashMap<u16, MutateInfo>,

    /// Index for the next mutate message to be sent to this client.
    ///
    /// See also [`Self::register_mutate_message`].
    next_mutate_index: u16,
}

impl ReplicatedClient {
    fn new(id: ClientId, policy: VisibilityPolicy) -> Self {
        Self {
            id,
            mutation_ticks: Default::default(),
            visibility: ClientVisibility::new(policy),
            update_tick: Default::default(),
            mutations: Default::default(),
            next_mutate_index: Default::default(),
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

    /// Sets the client's update tick.
    pub(crate) fn set_update_tick(&mut self, tick: RepliconTick) {
        self.update_tick = tick;
    }

    /// Returns the last tick in which a replicated entity had an insertion, removal, or gained/lost a component from the
    /// perspective of the client.
    pub fn update_tick(&self) -> RepliconTick {
        self.update_tick
    }

    /// Clears all entities for unacknowledged mutate messages, returning them as an iterator.
    ///
    /// Keeps the allocated memory for reuse.
    fn drain_entities(&mut self) -> impl Iterator<Item = Vec<Entity>> + '_ {
        self.mutations
            .drain()
            .map(|(_, mutate_info)| mutate_info.entities)
    }

    /// Resets all data.
    ///
    /// Keeps the allocated memory for reuse.
    fn reset(&mut self, id: ClientId) {
        self.id = id;
        self.visibility.clear();
        self.mutation_ticks.clear();
        self.mutations.clear();
        self.next_mutate_index = 0;
    }

    /// Registers mutate message at specified `tick` and `timestamp` and returns its index with entities to fill.
    ///
    /// Used later to acknowledge updated entities.
    #[must_use]
    pub(crate) fn register_mutate_message(
        &mut self,
        client_buffers: &mut ClientBuffers,
        tick: Tick,
        timestamp: Duration,
    ) -> (u16, &mut Vec<Entity>) {
        let mutate_index = self.next_mutate_index;
        self.next_mutate_index = self.next_mutate_index.overflowing_add(1).0;

        let mut entities = client_buffers.entities.pop().unwrap_or_default();
        entities.clear();
        let mutate_info = MutateInfo {
            tick,
            timestamp,
            entities,
        };
        let mutate_info = self
            .mutations
            .entry(mutate_index)
            .insert(mutate_info)
            .into_mut();

        (mutate_index, &mut mutate_info.entities)
    }

    /// Sets the mutation tick for an entity that is replicated to this client.
    ///
    /// The mutation tick is the reference point for determining if components on an entity have mutated and
    /// need to be replicated. Component mutations older than the update tick are assumed to be acked by the client.
    pub(crate) fn set_mutation_tick(&mut self, entity: Entity, tick: Tick) {
        self.mutation_ticks.insert(entity, tick);
    }

    /// Gets the mutation tick for an entity that is replicated to this client.
    pub fn mutation_tick(&self, entity: Entity) -> Option<Tick> {
        self.mutation_ticks.get(&entity).copied()
    }

    /// Marks mutate message as acknowledged by its index.
    ///
    /// Mutation tick for all entities from this mutate message will be set to the message tick if it's higher.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(crate) fn ack_mutate_message(
        &mut self,
        client_buffers: &mut ClientBuffers,
        tick: Tick,
        mutate_index: u16,
    ) {
        let Some(mutate_info) = self.mutations.remove(&mutate_index) else {
            debug!(
                "received unknown mutate index {mutate_index} from {:?}",
                self.id
            );
            return;
        };

        for entity in &mutate_info.entities {
            let Some(last_tick) = self.mutation_ticks.get_mut(entity) else {
                // We ignore missing entities, since they were probably despawned.
                continue;
            };

            // Received tick could be outdated because we bump it
            // if we detect any insertion on the entity in `collect_changes`.
            if !last_tick.is_newer_than(mutate_info.tick, tick) {
                *last_tick = mutate_info.tick;
            }
        }
        client_buffers.entities.push(mutate_info.entities);

        trace!(
            "{:?} acknowledged mutate message with {:?}",
            self.id,
            mutate_info.tick,
        );
    }

    /// Removes a despawned entity tracked by this client.
    pub fn remove_despawned(&mut self, entity: Entity) {
        self.mutation_ticks.remove(&entity);
        self.visibility.remove_despawned(entity);
        // We don't clean up `self.mutations` for efficiency reasons.
        // `Self::acknowledge()` will properly ignore despawned entities.
    }

    /// Drains all entities for which visibility was lost during this tick.
    ///
    /// Internal cleanup happens lazily during the iteration.
    pub(crate) fn drain_lost_visibility(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.visibility.drain_lost().inspect(|entity| {
            self.mutation_ticks.remove(entity);
        })
    }

    /// Removes all mutate messages older then `min_timestamp`.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(crate) fn cleanup_older_mutations(
        &mut self,
        client_buffers: &mut ClientBuffers,
        min_timestamp: Duration,
    ) {
        self.mutations.retain(|_, mutate_info| {
            if mutate_info.timestamp < min_timestamp {
                client_buffers
                    .entities
                    .push(mem::take(&mut mutate_info.entities));
                false
            } else {
                true
            }
        });
    }
}

/// Reusable buffers for [`ReplicatedClients`] and [`ReplicatedClient`].
#[derive(Default, Resource)]
pub(crate) struct ClientBuffers {
    /// [`ReplicatedClient`]'s of previously disconnected clients.
    ///
    /// Stored to reuse allocated memory.
    clients: Vec<ReplicatedClient>,

    /// [`Vec`]'s from acknowledged [`MutateInfo`]'s.
    ///
    /// Stored to reuse allocated capacity.
    entities: Vec<Vec<Entity>>,
}

struct MutateInfo {
    tick: Tick,
    timestamp: Duration,
    entities: Vec<Entity>,
}

/// Controls how visibility will be managed via [`ClientVisibility`].
#[derive(Default, Debug, Clone, Copy)]
pub enum VisibilityPolicy {
    /// All entities are visible by default and visibility can't be changed.
    #[default]
    All,
    /// All entities are visible by default and should be explicitly registered to be hidden.
    Blacklist,
    /// All entities are hidden by default and should be explicitly registered to be visible.
    Whitelist,
}
