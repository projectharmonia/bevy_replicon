pub mod dont_replicate;
pub mod replication_rules;
pub mod replicon_channels;
pub mod replicon_tick;

use bevy::prelude::*;

use replication_rules::{Replication, ReplicationRules};
use replicon_channels::RepliconChannels;
use replicon_tick::RepliconTick;

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replication>()
            .init_resource::<RepliconTick>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRules>();
    }
}
