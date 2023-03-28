use bevy::{
    ecs::{component::ComponentId, system::SystemChangeTick},
    prelude::*,
    utils::HashMap,
};

use super::AckedTicks;
use crate::{
    replication_core::{Replication, ReplicationRules},
    server::ServerState,
    tick::NetworkTick,
};

/// Stores component removals in [`RemovalTracker`] component to make them persistent across ticks.
///
/// Used only on server and tracks only entities with [`Replication`] component.
pub(super) struct RemovalTrackerPlugin;

impl Plugin for RemovalTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            (Self::insertion_system, Self::cleanup_system).in_set(OnUpdate(ServerState::Hosting)),
        )
        .add_system(
            Self::detection_system
                .in_base_set(CoreSet::PostUpdate)
                .run_if(in_state(ServerState::Hosting)),
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
                client_acks.values().any(|last_tick| {
                    tick.is_newer_than(*last_tick, Tick::new(change_tick.change_tick()))
                })
            });
        }
    }

    fn detection_system(
        mut set: ParamSet<(&World, Query<&mut RemovalTracker>)>,
        replication_rules: Res<ReplicationRules>,
    ) {
        let current_tick = set.p0().read_change_tick();
        for &component_id in &replication_rules.replicated {
            let entities: Vec<_> = set.p0().removed_with_id(component_id).collect();
            for entity in entities {
                if let Ok(mut removal_tracker) = set.p1().get_mut(entity) {
                    removal_tracker.insert(component_id, Tick::new(current_tick));
                }
            }
        }
    }
}

#[derive(Component, Default, Deref, DerefMut)]
pub(crate) struct RemovalTracker(pub(crate) HashMap<ComponentId, NetworkTick>);

#[cfg(test)]
mod tests {
    use crate::replication_core::AppReplicationExt;

    use super::*;

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugin(RemovalTrackerPlugin)
            .add_state::<ServerState>()
            .init_resource::<AckedTicks>()
            .init_resource::<ReplicationRules>()
            .replicate::<Transform>();

        app.world
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Hosting);

        app.update();

        // To avoid cleanup.
        const DUMMY_CLIENT_ID: u64 = 0;
        app.world
            .resource_mut::<AckedTicks>()
            .insert(DUMMY_CLIENT_ID, Default::default());

        let replicated_entity = app.world.spawn((Transform::default(), Replication)).id();

        app.world
            .entity_mut(replicated_entity)
            .remove::<Transform>();

        app.update();

        let transform_id = app.world.component_id::<Transform>().unwrap();
        let removal_tracker = app.world.get::<RemovalTracker>(replicated_entity).unwrap();
        assert!(removal_tracker.contains_key(&transform_id));
    }
}
