use bevy::{
    ecs::{component::Tick, system::SystemChangeTick},
    prelude::*,
};
use bevy_renet::renet::RenetServer;

use super::ServerSet;
use crate::replicon_core::replication_rules::Replication;

/// Tracks entity despawns of entities with [`Replication`] component in [`DespawnTracker`] resource.
///
/// Used only on server. Despawns will be cleaned after all clients acknowledge them.
pub(super) struct DespawnTrackerPlugin;

impl Plugin for DespawnTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnTracker>().add_systems(
            PostUpdate,
            Self::detection_system
                .before(ServerSet::Send)
                .run_if(resource_exists::<RenetServer>()),
        );
    }
}

impl DespawnTrackerPlugin {
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

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugins(DespawnTrackerPlugin)
            .insert_resource(RenetServer::new(Default::default()));

        app.update();

        let replicated_entity = app.world.spawn(Replication).id();

        app.update();

        app.world.entity_mut(replicated_entity).despawn();

        app.update();

        let despawn_tracker = app.world.resource::<DespawnTracker>();
        assert_eq!(despawn_tracker.len(), 1);
        assert_eq!(despawn_tracker.first().unwrap().0, replicated_entity);
    }
}
