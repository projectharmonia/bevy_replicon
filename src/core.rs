pub mod common_conditions;
pub mod component_rules;
pub mod replicated_archetypes;
pub mod replication_fns;
pub mod replicon_channels;
pub mod replicon_tick;

use bevy::prelude::*;

use component_rules::ComponentRules;
use replicated_archetypes::ReplicatedArchetypes;
use replication_fns::ReplicationFns;
use replicon_channels::RepliconChannels;
use replicon_tick::RepliconTick;
use serde::{Deserialize, Serialize};

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replication>()
            .init_resource::<RepliconTick>()
            .init_resource::<ReplicatedArchetypes>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ComponentRules>();
    }
}

/// Unique client ID.
///
/// Could be a client or a dual server-client.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
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

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replication;
