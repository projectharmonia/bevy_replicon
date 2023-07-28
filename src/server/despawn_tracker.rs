use bevy::{
    ecs::{component::Tick, system::SystemChangeTick},
    prelude::*,
    utils::HashSet,
};
use bevy_renet::renet::RenetServer;

use super::{AckedTicks, ServerSet};
use crate::replication_core::Replication;

/// Tracks entity despawns of entities with [`Replication`] component in [`DespawnTracker`] resource.
///
/// Used only on server. Despawns will be cleaned after all clients acknowledge them.
pub(super) struct DespawnTrackerPlugin;

impl Plugin for DespawnTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnTracker>().add_systems(
            PostUpdate,
            (
                Self::entity_tracking_system,
                Self::cleanup_system,
                Self::detection_system,
            )
                .before(ServerSet::Send)
                .run_if(resource_exists::<RenetServer>()),
        );
    }
}

impl DespawnTrackerPlugin {
    fn entity_tracking_system(
        mut tracker: ResMut<DespawnTracker>,
        new_replicated_entities: Query<Entity, Added<Replication>>,
    ) {
        for entity in &new_replicated_entities {
            tracker.tracked_entities.insert(entity);
        }
    }

    /// Cleanups all acknowledged despawns.
    ///
    /// Cleans all despawns if [`AckedTicks`] is empty.
    fn cleanup_system(
        change_tick: SystemChangeTick,
        mut despawn_tracker: ResMut<DespawnTracker>,
        client_acks: Res<AckedTicks>,
    ) {
        despawn_tracker.despawns.retain(|(_, tick)| {
            client_acks
                .values()
                .any(|last_tick| tick.is_newer_than(*last_tick, change_tick.this_run()))
        });
    }

    fn detection_system(
        change_tick: SystemChangeTick,
        mut tracker: ResMut<DespawnTracker>,
        entities: Query<Entity>,
    ) {
        let DespawnTracker {
            ref mut tracked_entities,
            ref mut despawns,
        } = *tracker;

        tracked_entities.retain(|&entity| {
            if entities.get(entity).is_err() {
                despawns.push((entity, change_tick.this_run()));
                false
            } else {
                true
            }
        });
    }
}

#[derive(Default, Resource)]
pub(crate) struct DespawnTracker {
    tracked_entities: HashSet<Entity>,
    /// Entities and ticks when they were despawned.
    pub(crate) despawns: Vec<(Entity, Tick)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugins(DespawnTrackerPlugin)
            .insert_resource(RenetServer::new(Default::default()))
            .init_resource::<AckedTicks>();

        app.update();

        // To avoid cleanup.
        const DUMMY_CLIENT_ID: u64 = 0;
        app.world
            .resource_mut::<AckedTicks>()
            .insert(DUMMY_CLIENT_ID, Tick::new(0));

        let replicated_entity = app.world.spawn(Replication).id();

        app.update();

        app.world.entity_mut(replicated_entity).despawn();

        app.update();

        let despawn_tracker = app.world.resource::<DespawnTracker>();
        assert_eq!(despawn_tracker.despawns.len(), 1);
        assert_eq!(
            despawn_tracker.despawns.first().unwrap().0,
            replicated_entity
        );
    }
}
