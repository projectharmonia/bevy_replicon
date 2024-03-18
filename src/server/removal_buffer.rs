use bevy::{
    ecs::{
        component::ComponentId,
        entity::EntityHashMap,
        event::ManualEventReader,
        removal_detection::{RemovedComponentEntity, RemovedComponentEvents},
    },
    prelude::*,
    utils::HashMap,
};

use super::{
    despawn_buffer::{DespawnBuffer, DespawnBufferPlugin},
    ServerPlugin, ServerSet,
};
use crate::core::{
    common_conditions::server_running, component_rules::ComponentRules,
    replication_fns::ComponentFnsIndex,
};

/// Buffers all replicated component removals in [`RemovalBuffer`] resource.
///
/// Used to avoid missing events.
pub(super) struct RemovalBufferPlugin;

impl Plugin for RemovalBufferPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RemovalBuffer>().add_systems(
            PostUpdate,
            Self::buffer_removals
                .after(DespawnBufferPlugin::buffer_despawns)
                .before(ServerPlugin::send_replication)
                .in_set(ServerSet::Send)
                .run_if(server_running),
        );
    }
}

impl RemovalBufferPlugin {
    fn buffer_removals(
        mut readers: Local<HashMap<ComponentId, ManualEventReader<RemovedComponentEntity>>>,
        remove_events: &RemovedComponentEvents,
        mut removal_buffer: ResMut<RemovalBuffer>,
        component_rules: Res<ComponentRules>,
        despawn_buffer: Res<DespawnBuffer>,
    ) {
        for (&component_id, &fns_index) in component_rules.ids() {
            for removals in remove_events.get(component_id).into_iter() {
                let reader = readers.entry(component_id).or_default();
                for entity in reader
                    .read(removals)
                    .cloned()
                    .map(Into::into)
                    .filter(|entity| !despawn_buffer.contains(entity))
                {
                    removal_buffer.insert(entity, fns_index);
                }
            }
        }
    }
}

/// Buffer with removed components.
#[derive(Default, Resource)]
pub(crate) struct RemovalBuffer {
    /// Component removals grouped by entity.
    removals: EntityHashMap<Vec<ComponentFnsIndex>>,

    /// [`Vec`]'s from entity removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    component_buffer: Vec<Vec<ComponentFnsIndex>>,
}

impl RemovalBuffer {
    /// Returns an iterator over entities and their removed components.
    pub(super) fn iter(&self) -> impl Iterator<Item = (Entity, &[ComponentFnsIndex])> {
        self.removals
            .iter()
            .map(|(&entity, components)| (entity, &**components))
    }

    /// Registers component removal for the specified entity.
    fn insert(&mut self, entity: Entity, fns_index: ComponentFnsIndex) {
        self.removals
            .entry(entity)
            .or_insert_with(|| self.component_buffer.pop().unwrap_or_default())
            .push(fns_index);
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
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{
        core::{
            component_rules::{AppReplicationExt, Replication},
            replication_fns::ReplicationFns,
        },
        server::replicon_server::RepliconServer,
    };

    #[test]
    fn removals() {
        let mut app = App::new();
        app.add_plugins((DespawnBufferPlugin, RemovalBufferPlugin))
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ComponentRules>()
            .replicate::<DummyComponent>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((DummyComponent, Replication))
            .remove::<DummyComponent>();

        app.update();

        let mut removal_buffer = app.world.resource_mut::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        removal_buffer.clear();
        assert!(removal_buffer.removals.is_empty());
        assert_eq!(removal_buffer.component_buffer.len(), 1);
    }

    #[test]
    fn despawn_ignore() {
        let mut app = App::new();
        app.add_plugins((DespawnBufferPlugin, RemovalBufferPlugin))
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ComponentRules>()
            .replicate::<DummyComponent>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world.spawn((DummyComponent, Replication)).despawn();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert!(
            removal_buffer.removals.is_empty(),
            "despawns shouldn't be counted as removals"
        );
    }

    #[derive(Serialize, Deserialize, Component)]
    struct DummyComponent;
}
