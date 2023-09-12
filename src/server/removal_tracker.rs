use bevy::{
    ecs::{
        component::{ComponentId, Tick},
        removal_detection::RemovedComponentEvents,
        system::SystemChangeTick,
    },
    prelude::*,
    utils::HashMap,
};
use bevy_renet::renet::RenetServer;

use super::{AckedTicks, ServerSet};
use crate::replication_core::{Replication, ReplicationRules};

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
                Self::cleanup_system,
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

    /// Cleanups all acknowledged despawns.
    fn cleanup_system(
        change_tick: SystemChangeTick,
        client_acks: Res<AckedTicks>,
        mut removal_trackers: Query<&mut RemovalTracker>,
    ) {
        for mut removal_tracker in &mut removal_trackers {
            removal_tracker.retain(|_, tick| {
                client_acks
                    .values()
                    .any(|last_tick| tick.is_newer_than(*last_tick, change_tick.this_run()))
            });
        }
    }

    fn detection_system(
        mut set: ParamSet<(&World, Query<&mut RemovalTracker>)>,
        replication_rules: Res<ReplicationRules>,
        remove_events: &RemovedComponentEvents,
    ) {
        let current_tick = set.p0().read_change_tick();
        for &component_id in &replication_rules.replicated {
            for entity in remove_events
                .get(component_id)
                .map(|removed| removed.iter_current_update_events().cloned())
                .into_iter()
                .flatten()
                .map(|e| e.into())
            {
                if let Ok(mut removal_tracker) = set.p1().get_mut(entity) {
                    removal_tracker.insert(component_id, current_tick);
                }
            }
        }
    }
}

#[derive(Component, Default, Deref, DerefMut)]
pub(crate) struct RemovalTracker(pub(crate) HashMap<ComponentId, Tick>);

#[cfg(test)]
mod tests {
    use crate::replication_core::AppReplicationExt;

    use super::*;

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugins(RemovalTrackerPlugin)
            .insert_resource(RenetServer::new(Default::default()))
            .init_resource::<AckedTicks>()
            .init_resource::<ReplicationRules>()
            .replicate::<Transform>();

        app.update();

        // To avoid cleanup.
        const DUMMY_CLIENT_ID: u64 = 0;
        app.world
            .resource_mut::<AckedTicks>()
            .0
            .insert(DUMMY_CLIENT_ID, Tick::new(0));

        let replicated_entity = app.world.spawn((Transform::default(), Replication)).id();

        app.update();

        app.world
            .entity_mut(replicated_entity)
            .remove::<Transform>();

        app.update();

        let transform_id = app.world.component_id::<Transform>().unwrap();
        let removal_tracker = app.world.get::<RemovalTracker>(replicated_entity).unwrap();
        assert!(removal_tracker.contains_key(&transform_id));
    }
}
