use bevy::prelude::*;

use super::ServerSet;
use crate::core::{common_conditions::server_running, replication::Replicated};

/// Treats removals of [`Replicated`] component as despawns and stores them into [`DespawnBuffer`] resource.
///
/// Used to avoid missing events in case the server's tick policy is not [`TickPolicy::EveryFrame`].
pub(super) struct DespawnBufferPlugin;

impl Plugin for DespawnBufferPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnBuffer>().add_systems(
            PostUpdate,
            buffer_despawns
                .before(super::send_replication)
                .in_set(ServerSet::Send)
                .run_if(server_running),
        );
    }
}

fn buffer_despawns(
    mut removed_replications: RemovedComponents<Replicated>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
) {
    for entity in removed_replications.read() {
        despawn_buffer.push(entity);
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
    use crate::core::backend::replicon_server::RepliconServer;

    #[test]
    fn despawns() {
        let mut app = App::new();
        app.add_plugins(DespawnBufferPlugin)
            .init_resource::<RepliconServer>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        app.world_mut().spawn(Replicated).despawn();

        app.update();

        let despawn_buffer = app.world().resource::<DespawnBuffer>();
        assert_eq!(despawn_buffer.len(), 1);
    }
}
