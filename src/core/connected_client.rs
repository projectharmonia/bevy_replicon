use bevy::{
    ecs::{component::HookContext, world::DeferredWorld},
    platform_support::collections::HashMap,
    prelude::*,
};

/// Marker for a connected client.
///
/// Backends should spawn and despawn entities with this component on connect and disconnect
/// and optionally update the [`NetworkStats`] component.
///
/// If the MTU of the connected client is dynamic, it's required for the backend to update
/// [`Self::max_size`] to ensure message splitting works properly.
///
/// `Entity` is used an identifier to refer to a client.
///
/// <div class="warning">
///
/// Entities with this components should be spawned and despawned only from the messaging backend.
///
/// </div>
///
/// See also [`ReplicatedClient`](crate::server::ReplicatedClient).
#[derive(Component, Reflect)]
#[component(on_add = on_client_add, on_remove = on_client_remove)]
#[require(Name(|| Name::new("Connected client")), NetworkStats)]
pub struct ConnectedClient {
    id: ClientId,
    /// Maximum size of a message that can be transferred over unreliable channel without
    /// splitting into multiple packets.
    ///
    /// Used to manually split mutations over packet-size messages to allow applying them partially.
    /// For more details on replication see [`ReplicationChannel`](super::channels::ReplicationChannel).
    ///
    /// <div class="warning">
    ///
    /// Should only be modified from the messaging backend.
    ///
    /// </div>
    pub max_size: usize,
}

impl ConnectedClient {
    /// Creates a new instance with a backend-provided ID and maximum message size for unreliable channel.
    pub fn new(id: ClientId, max_size: usize) -> Self {
        Self { id, max_size }
    }

    /// Returns client ID provided by backend.
    ///
    /// Can be used to identify which client belongs to which connection.
    ///
    /// See also [`ClientIdMap`].
    pub fn id(&self) -> ClientId {
        self.id
    }
}

fn on_client_add(mut world: DeferredWorld, ctx: HookContext) {
    let connected_client = world.get::<ConnectedClient>(ctx.entity).unwrap();
    let client_id = connected_client.id;
    let mut client_map = world.resource_mut::<ClientIdMap>();
    if let Some(old_entity) = client_map.0.insert(client_id, ctx.entity) {
        error!("backend-provided `{client_id:?}` that was already mapped to client `{old_entity}`");
    }
}

fn on_client_remove(mut world: DeferredWorld, ctx: HookContext) {
    let connected_client = world.get::<ConnectedClient>(ctx.entity).unwrap();
    let client_id = connected_client.id;
    let mut client_map = world.resource_mut::<ClientIdMap>();
    client_map.0.remove(&client_id);
}

/// Maps [`ConnectedClient::id`] to associated entity.
///
/// Automatically updated on clients spawns and despawns.
#[derive(Resource, Default, Deref)]
pub struct ClientIdMap(HashMap<ClientId, Entity>);

/// Unique client ID provided by a messaging backend.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Ord, PartialOrd, Reflect)]
pub struct ClientId(u64);

impl ClientId {
    /// Creates a new ID wrapping the given value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Gets the value of this ID.
    pub fn get(&self) -> u64 {
        self.0
    }
}

/// Statistic associated with [`RepliconClient`](super::replicon_client::RepliconClient) or
/// [`ConnectedClient`].
///
/// All values can be zero if not provided by the backend.
///
/// <div class="warning">
///
/// Should only be modified from the messaging backend.
///
/// </div>
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
pub struct NetworkStats {
    /// Round-time trip in seconds for the connection.
    pub rtt: f64,

    /// Packet loss % for the connection.
    pub packet_loss: f64,

    /// Bytes sent per second for the connection.
    pub sent_bps: f64,

    /// Bytes received per second for the connection.
    pub received_bps: f64,
}
