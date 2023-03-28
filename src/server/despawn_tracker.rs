use bevy::{ecs::system::SystemChangeTick, prelude::*, utils::HashSet};

use super::AckedTicks;
use crate::{replication_core::Replication, server::ServerState, tick::NetworkTick};

/// Tracks entity despawns of entities with [`Replication`] component in [`DespawnTracker`] resource.
///
/// Used only on server. Despawns will be cleaned after all clients acknowledge them.
pub(super) struct DespawnTrackerPlugin;

impl Plugin for DespawnTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnTracker>().add_systems(
            (
                Self::entity_tracking_system,
                Self::cleanup_system,
                Self::detection_system,
            )
                .in_set(OnUpdate(ServerState::Hosting)),
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
            client_acks.values().any(|last_tick| {
                tick.is_newer_than(*last_tick, Tick::new(change_tick.change_tick()))
            })
        });
    }

    fn detection_system(
        network_tick: Res<NetworkTick>,
        mut tracker: ResMut<DespawnTracker>,
        entities: Query<Entity>,
    ) {
        let DespawnTracker {
            ref mut tracked_entities,
            ref mut despawns,
        } = *tracker;

        tracked_entities.retain(|&entity| {
            if entities.get(entity).is_err() {
                despawns.push((entity, *network_tick));
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
    /// Entities and network ticks when they were despawned.
    pub(crate) despawns: Vec<(Entity, NetworkTick)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection() {
        let mut app = App::new();
        app.add_plugin(DespawnTrackerPlugin)
            .add_state::<ServerState>()
            .init_resource::<AckedTicks>();

        app.world
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Hosting);

        app.update();

        // To avoid cleanup.
        const DUMMY_CLIENT_ID: u64 = 0;
        app.world
            .resource_mut::<AckedTicks>()
            .insert(DUMMY_CLIENT_ID, Default::default());

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
