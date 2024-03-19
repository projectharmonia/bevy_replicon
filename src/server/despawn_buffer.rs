use bevy::prelude::*;

use super::{ServerPlugin, ServerSet};
use crate::core::{common_conditions::server_running, component_rules::Replication};

/// Treats removals of [`Replication`] component as despawns and stores them into [`DespawnBuffer`] resource.
///
/// Used to avoid missing events in case the server's tick policy is not [`TickPolicy::EveryFrame`].
pub(super) struct DespawnBufferPlugin;

impl Plugin for DespawnBufferPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnBuffer>().add_systems(
            PostUpdate,
            Self::buffer_despawns
                .before(ServerPlugin::send_replication)
                .in_set(ServerSet::Send)
                .run_if(server_running),
        );
    }
}

impl DespawnBufferPlugin {
    pub(super) fn buffer_despawns(
        mut removed_replications: RemovedComponents<Replication>,
        mut despawn_buffer: ResMut<DespawnBuffer>,
    ) {
        for entity in removed_replications.read() {
            despawn_buffer.push(entity);
        }
    }
}

/// Buffer with all despawned entities.
#[derive(Default, Resource)]
pub struct DespawnBuffer(Vec<Entity>);

impl DespawnBuffer {
    /// Adds an entity to the end of the buffer.
    pub fn push(&mut self, entity: Entity) {
        self.0.push(entity);
    }

    /// Returns `true` if the buffer contains an entity.
    pub fn contains(&self, entity: Entity) -> bool {
        self.0.contains(&entity)
    }

    /// Removes all entities, returning them as an iterator.
    pub(crate) fn drain(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.0.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::replicon_server::RepliconServer;

    #[test]
    fn despawns() {
        let mut app = App::new();
        app.add_plugins(DespawnBufferPlugin)
            .init_resource::<RepliconServer>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world.spawn(Replication).despawn();

        app.update();

        let despawn_buffer = app.world.resource::<DespawnBuffer>();
        assert_eq!(despawn_buffer.0.len(), 1);
    }
}
