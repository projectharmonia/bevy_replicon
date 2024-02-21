use bevy::prelude::*;
use bevy_renet::renet::RenetServer;

use super::{ServerPlugin, ServerSet};
use crate::replicon_core::replication_rules::Replication;

/// Treats removals of [`Replication`] component as despawns and stores them into [`DespawnBuffer`] resource.
///
/// Used to avoid missing events in case the server's tick policy is not [`TickPolicy::EveryFrame`].
pub(super) struct DespawnBufferPlugin;

impl Plugin for DespawnBufferPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnBuffer>().add_systems(
            PostUpdate,
            Self::detection_system
                .before(ServerPlugin::replication_sending_system)
                .in_set(ServerSet::Send)
                .run_if(resource_exists::<RenetServer>),
        );
    }
}

impl DespawnBufferPlugin {
    pub(super) fn detection_system(
        mut removed_replications: RemovedComponents<Replication>,
        mut despawn_buffer: ResMut<DespawnBuffer>,
    ) {
        for entity in removed_replications.read() {
            despawn_buffer.push(entity);
        }
    }
}

/// Buffer with all despawned entities.
///
/// Should be cleaned up manually.
#[derive(Default, Resource, Deref, DerefMut)]
pub(crate) struct DespawnBuffer(Vec<Entity>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn despawns() {
        let mut app = App::new();
        app.add_plugins(DespawnBufferPlugin)
            .insert_resource(RenetServer::new(Default::default()));

        app.update();

        app.world.spawn(Replication).despawn();

        app.update();

        let despawn_buffer = app.world.resource::<DespawnBuffer>();
        assert_eq!(despawn_buffer.len(), 1);
    }
}
