use bevy::{
    ecs::{component::Tick, system::SystemChangeTick},
    prelude::*,
};
use bevy_renet::renet::RenetServer;

use super::{AckedTicks, ServerSet};
use crate::replicon_core::replication_rules::Replication;

/// Tracks entity despawns of entities with [`Replication`] component in [`DespawnTracker`] resource.
///
/// Used only on server. Despawns will be cleaned after all clients acknowledge them.
pub(super) struct DespawnTrackerPlugin;

impl Plugin for DespawnTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnTracker>().add_systems(
            PostUpdate,
            (Self::cleanup_system, Self::detection_system)
                .before(ServerSet::Send)
                .run_if(resource_exists::<RenetServer>()),
        );
    }
}

impl DespawnTrackerPlugin {
    /// Cleanups all acknowledged despawns.
    ///
    /// Cleans all despawns if [`AckedTicks`] is empty.
    fn cleanup_system(
        change_tick: SystemChangeTick,
        mut despawn_tracker: ResMut<DespawnTracker>,
        acked_ticks: Res<AckedTicks>,
    ) {
        despawn_tracker.retain(|(_, tick)| {
            acked_ticks.clients.values().any(|acked_tick| {
                let system_tick = *acked_ticks
                    .system_ticks
                    .get(acked_tick)
                    .unwrap_or(&Tick::new(0));
                tick.is_newer_than(system_tick, change_tick.this_run())
            })
        });
    }

    fn detection_system(
        change_tick: SystemChangeTick,
        mut removed_replications: RemovedComponents<Replication>,
        mut despawn_tracker: ResMut<DespawnTracker>,
    ) {
        for entity in removed_replications.read() {
            despawn_tracker.push((entity, change_tick.this_run()));
        }
    }
}

/// Entities and ticks when they were despawned.
#[derive(Default, Resource, Deref, DerefMut)]
pub(crate) struct DespawnTracker(pub(super) Vec<(Entity, Tick)>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::RepliconTick;

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
            .clients
            .insert(DUMMY_CLIENT_ID, RepliconTick(0));

        let replicated_entity = app.world.spawn(Replication).id();

        app.update();

        app.world.entity_mut(replicated_entity).despawn();

        app.update();

        let despawn_tracker = app.world.resource::<DespawnTracker>();
        assert_eq!(despawn_tracker.len(), 1);
        assert_eq!(despawn_tracker.first().unwrap().0, replicated_entity);
    }
}
