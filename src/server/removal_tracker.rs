use bevy::{
    ecs::{component::Tick, removal_detection::RemovedComponentEvents, system::SystemChangeTick},
    prelude::*,
    utils::HashMap,
};
use bevy_renet::renet::RenetServer;

use super::{AckedTicks, ServerSet, TicksMap};
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
        acked_ticks: Res<AckedTicks>,
        ticks_map: Res<TicksMap>,
        mut removal_trackers: Query<&mut RemovalTracker>,
    ) {
        for mut removal_tracker in &mut removal_trackers {
            removal_tracker.retain(|_, tick| {
                acked_ticks.values().any(|acked_tick| {
                    let system_tick = *ticks_map.get(acked_tick).unwrap_or(&Tick::new(0));
                    tick.is_newer_than(system_tick, change_tick.this_run())
                })
            });
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
    use bevy_renet::renet::ClientId;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{replicon_core::replication_rules::AppReplicationExt, server::RepliconTick};

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugins(RemovalTrackerPlugin)
            .insert_resource(RenetServer::new(Default::default()))
            .init_resource::<AckedTicks>()
            .init_resource::<TicksMap>()
            .init_resource::<ReplicationRules>()
            .replicate::<DummyComponent>();

        app.update();

        // To avoid cleanup.
        const DUMMY_CLIENT_ID: ClientId = ClientId::from_raw(0);
        app.world
            .resource_mut::<AckedTicks>()
            .0
            .insert(DUMMY_CLIENT_ID, RepliconTick(0));

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
