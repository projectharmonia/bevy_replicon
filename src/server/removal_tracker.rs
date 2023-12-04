use bevy::{
    ecs::{component::Tick, removal_detection::RemovedComponentEvents, system::SystemChangeTick},
    prelude::*,
    utils::HashMap,
};
use bevy_renet::renet::RenetServer;

use super::ServerSet;
use crate::replicon_core::replication_rules::{Replication, ReplicationId, ReplicationRules};

/// Stores component removals in [`RemovalTracker`] component to make them persistent across ticks.
///
/// Used only on server and tracks only entities with [`Replication`] component.
pub(super) struct RemovalTrackerPlugin;

impl Plugin for RemovalTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (
                Self::insertion_system,
                Self::detection_system.run_if(resource_exists::<RenetServer>()),
            )
                .before(ServerSet::Send)
                .run_if(resource_exists::<RenetServer>()),
        );
    }
}

impl RemovalTrackerPlugin {
    fn insertion_system(
        mut commands: Commands,
        new_replicated_entities: Query<Entity, (Added<Replication>, Without<RemovalTracker>)>,
    ) {
        for entity in &new_replicated_entities {
            commands.entity(entity).insert(RemovalTracker::default());
        }
    }

    fn detection_system(
        change_tick: SystemChangeTick,
        remove_events: &RemovedComponentEvents,
        replication_rules: Res<ReplicationRules>,
        mut removal_trackers: Query<&mut RemovalTracker>,
    ) {
        for (&component_id, &replication_id) in replication_rules.get_ids() {
            for entity in remove_events
                .get(component_id)
                .map(|removed| removed.iter_current_update_events().cloned())
                .into_iter()
                .flatten()
                .map(Into::into)
            {
                if let Ok(mut removal_tracker) = removal_trackers.get_mut(entity) {
                    removal_tracker.insert(replication_id, change_tick.this_run());
                }
            }
        }
    }
}

#[derive(Component, Default, Deref, DerefMut)]
pub(crate) struct RemovalTracker(pub(super) HashMap<ReplicationId, Tick>);

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::replicon_core::replication_rules::AppReplicationExt;

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugins(RemovalTrackerPlugin)
            .insert_resource(RenetServer::new(Default::default()))
            .init_resource::<ReplicationRules>()
            .replicate::<DummyComponent>();

        app.update();

        let replicated_entity = app.world.spawn((DummyComponent, Replication)).id();

        app.update();

        app.world
            .entity_mut(replicated_entity)
            .remove::<DummyComponent>();

        app.update();

        let component_id = app.world.init_component::<DummyComponent>();
        let replcation_rules = app.world.resource::<ReplicationRules>();
        let (replication_id, _) = replcation_rules.get(component_id).unwrap();
        let removal_tracker = app.world.get::<RemovalTracker>(replicated_entity).unwrap();
        assert!(removal_tracker.contains_key(&replication_id));
    }

    #[derive(Serialize, Deserialize, Component)]
    struct DummyComponent;
}
