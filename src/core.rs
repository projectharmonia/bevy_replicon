pub mod command_markers;
pub mod common_conditions;
pub mod replication_fns;
pub mod replication_rules;
pub mod replicon_channels;
pub mod replicon_tick;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use command_markers::CommandMarkers;
use replication_fns::ReplicationFns;
use replication_rules::ReplicationRules;
use replicon_channels::RepliconChannels;

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replicated>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .init_resource::<CommandMarkers>();
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
