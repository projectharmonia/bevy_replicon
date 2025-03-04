pub mod channels;
pub mod common_conditions;
pub mod entity_serde;
pub mod event;
pub mod postcard_utils;
pub mod replication;
pub mod replicon_client;
pub mod replicon_server;
pub mod replicon_tick;
pub mod server_entity_map;

use bevy::prelude::*;

use channels::RepliconChannels;
use event::event_registry::EventRegistry;
use replication::{
    command_markers::CommandMarkers, replication_registry::ReplicationRegistry,
    replication_rules::ReplicationRules, track_mutate_messages::TrackMutateMessages, Replicated,
};

/// Initializes types and resources needed for both client and server.
pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replicated>()
            .register_type::<NetworkStats>()
            .init_resource::<TrackMutateMessages>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<CommandMarkers>()
            .init_resource::<EventRegistry>();
    }
}

/// Statistic associated with [`RepliconClient`](replicon_client::RepliconClient) or
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

/// Marker for a connected client.
///
/// Backends should spawn and despawn entities with this component on connect and disconnect.
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
#[derive(Component)]
#[require(Name(|| Name::new("Connected client")), NetworkStats)]
pub struct ConnectedClient;

/// A placeholder entity for a connected client that refers to the listen server (when the server is also a client).
///
/// Equal to [`Entity::PLACEHOLDER`].
///
/// See also [`ToClients`](event::server_event::ToClients) and [`FromClient`](event::client_event::FromClient) events.
pub const SERVER: Entity = Entity::PLACEHOLDER;
