pub mod common_conditions;
pub mod replication_fns;
pub mod replication_rules;
pub mod replicon_channels;
pub mod replicon_tick;

use bevy::prelude::*;

use replication_fns::ReplicationFns;
use replication_rules::{ReplicationRule, ReplicationRules};
use replicon_channels::RepliconChannels;
use replicon_tick::RepliconTick;
use serde::{Deserialize, Serialize};

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replication>()
            .init_resource::<RepliconTick>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>();
    }

    fn finish(&self, app: &mut App) {
        if cfg!(debug_assertions) {
            let rules = app.world.resource::<ReplicationRules>();
            for (index, rule_a) in rules.iter().enumerate() {
                for rule_b in &rules[index + 1..] {
                    if rule_a.is_subset(rule_b) {
                        subset_panic(app, rule_a, rule_b);
                    } else if rule_b.is_subset(rule_a) {
                        subset_panic(app, rule_b, rule_a);
                    }
                }
            }
        }
    }
}

fn subset_panic(app: &App, subset_rule: &ReplicationRule, rule: &ReplicationRule) {
    let components: Vec<_> = rule
        .components
        .iter()
        .filter_map(|&(component_id, _)| app.world.components().get_name(component_id))
        .collect();
    let subset_components: Vec<_> = subset_rule
        .components
        .iter()
        .filter_map(|&(component_id, _)| app.world.components().get_name(component_id))
        .collect();

    panic!("rule with components {subset_components:?} is a subset of {components:?}, try splitting it");
}

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replication;

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
