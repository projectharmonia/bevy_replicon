pub mod dont_replicate;
pub mod network_channels;
pub mod replication_rules;
pub mod replicon_tick;

use bevy::prelude::*;

use network_channels::NetworkChannels;
use replication_rules::{Replication, ReplicationRules};
use replicon_tick::RepliconTick;

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replication>()
            .init_resource::<RepliconTick>()
            .init_resource::<NetworkChannels>()
            .init_resource::<ReplicationRules>();
    }
}
