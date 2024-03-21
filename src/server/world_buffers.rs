use bevy::{ecs::entity::EntityHashMap, prelude::*};

use super::ServerSet;
use crate::core::{common_conditions::server_running, replication_fns::RemoveFnId, Replication};

/// Registers buffers for despawns and removals.
///
/// Treats removals of [`Replication`] component as despawns and stores them into [`DespawnBuffer`] resource.
///
/// Removals should be tracked by replication rules, see
/// [`ComponentRulesPlugin`](crate::component_rules::ComponentRulesPlugin) for details.
///
/// Used to avoid missing events in case the server's tick policy is not [`TickPolicy::EveryFrame`].
pub(super) struct WorldBuffersPlugin;

impl Plugin for WorldBuffersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnBuffer>()
            .init_resource::<RemovalBuffer>()
            .add_systems(
                PostUpdate,
                Self::buffer_despawns
                    .in_set(ServerSet::BufferDespawns)
                    .run_if(server_running),
            );
    }
}

impl WorldBuffersPlugin {
    fn buffer_despawns(
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

/// Buffer with removed components.
#[derive(Default, Resource)]
pub struct RemovalBuffer {
    /// Component removals grouped by entity.
    removals: EntityHashMap<Vec<RemoveFnId>>,

    /// [`Vec`]s from entity removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    component_buffer: Vec<Vec<RemoveFnId>>,
}

impl RemovalBuffer {
    /// Returns an iterator over entities and their removed components.
    pub(super) fn iter(&self) -> impl Iterator<Item = (Entity, &[RemoveFnId])> {
        self.removals
            .iter()
            .map(|(&entity, components)| (entity, &**components))
    }

    /// Registers component removal for the specified entity.
    pub fn insert(&mut self, entity: Entity, remove_id: RemoveFnId) {
        self.removals
            .entry(entity)
            .or_insert_with(|| self.component_buffer.pop().unwrap_or_default())
            .push(remove_id);
    }

    /// Returns the number of removals in the buffer.
    pub fn len(&self) -> usize {
        self.removals.len()
    }

    /// Returns `true` if the buffer contains no removals.
    pub fn is_empty(&self) -> bool {
        self.removals.is_empty()
    }

    /// Clears all removals.
    ///
    /// Keeps the allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.component_buffer
            .extend(self.removals.drain().map(|(_, mut components)| {
                components.clear();
                components
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::replicon_server::RepliconServer;

    #[test]
    fn despawns() {
        let mut app = App::new();
        app.add_plugins(WorldBuffersPlugin)
            .init_resource::<RepliconServer>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world.spawn(Replication).despawn();

        app.update();

        let despawn_buffer = app.world.resource::<DespawnBuffer>();
        assert_eq!(despawn_buffer.len(), 1);
    }
}
