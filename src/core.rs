pub mod channels;
pub mod common_conditions;
pub mod connected_clients;
pub mod event_registry;
pub mod replication;
pub mod replicon_client;
pub mod replicon_server;
pub mod replicon_tick;
pub mod server_entity_map;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use channels::RepliconChannels;
use event_registry::EventRegistry;
use replication::{
    command_markers::CommandMarkers, replication_registry::ReplicationRegistry,
    replication_rules::ReplicationRules, track_mutate_messages::TrackMutateMessages, Replicated,
};

/// Initializes types and resources needed for both client and server.
pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replicated>()
            .init_resource::<TrackMutateMessages>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<CommandMarkers>()
            .init_resource::<EventRegistry>();
    }
}

/// Unique client ID.
///
/// Could be a client or a dual server-client.
#[derive(
    Debug, Clone, Copy, Hash, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize, Reflect,
)]
pub struct ClientId(u64);

impl ClientId {
    /// The server's client ID when it's a dual server-client.
    pub const SERVER: Self = Self::new(0);

    /// Creates a new ID wrapping the given value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Gets the value of this ID.
    pub fn get(&self) -> u64 {
        self.0
    }
}
