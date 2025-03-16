pub mod channels;
pub mod common_conditions;
pub mod connected_client;
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
use connected_client::{ConnectedClient, NetworkIdMap, NetworkStats};
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
            .register_type::<ConnectedClient>()
            .register_type::<NetworkIdMap>()
            .register_type::<NetworkStats>()
            .init_resource::<NetworkIdMap>()
            .init_resource::<TrackMutateMessages>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<CommandMarkers>()
            .init_resource::<EventRegistry>();
    }
}

/// A placeholder entity for a connected client that refers to the listen server (when the server is also a client).
///
/// Equal to [`Entity::PLACEHOLDER`].
///
/// See also [`ToClients`](event::server_event::ToClients) and [`FromClient`](event::client_event::FromClient) events.
pub const SERVER: Entity = Entity::PLACEHOLDER;
