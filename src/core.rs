pub mod channels;
pub mod command_markers;
pub mod common_conditions;
pub mod connected_clients;
pub mod ctx;
pub mod deferred_entity;
pub mod event_registry;
pub mod replicated_clients;
pub mod replication_registry;
pub mod replication_rules;
pub mod replicon_client;
pub mod replicon_server;
pub mod replicon_tick;
pub mod server_entity_map;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use channels::RepliconChannels;
use command_markers::CommandMarkers;
use event_registry::EventRegistry;
use replication_registry::ReplicationRegistry;
use replication_rules::ReplicationRules;

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replicated>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<CommandMarkers>()
            .init_resource::<EventRegistry>();
    }
}

#[deprecated(note = "use `Replicated` instead")]
pub type Replication = Replicated;

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replicated;

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
