use bevy::{
    ecs::{
        component::ComponentId,
        entity::EntityHashMap,
        event::ManualEventReader,
        removal_detection::{RemovedComponentEntity, RemovedComponentEvents},
    },
    prelude::*,
    utils::{HashMap, HashSet},
};

use super::{ServerPlugin, ServerSet};
use crate::core::{
    common_conditions::server_running, replication_fns::RemoveFnId,
    replication_rules::ReplicationRules, Replication,
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
                .before(ServerPlugin::send_replication)
                .in_set(ServerSet::Send)
                .run_if(server_running),
        );
    }
}

impl RemovalBufferPlugin {
    fn buffer_removals(
        mut entity_removals: Local<EntityRemovals>,
        mut readers: Local<HashMap<ComponentId, ManualEventReader<RemovedComponentEntity>>>,
        remove_events: &RemovedComponentEvents,
        mut removal_buffer: ResMut<RemovalBuffer>,
        replication_rules: Res<ReplicationRules>,
        replicatred: Query<(), With<Replication>>,
    ) {
        // TODO: Ask Bevy to provide an iterator over `RemovedComponentEvents`.
        for &(component_id, _) in replication_rules
            .iter()
            .flat_map(|replication_rule| replication_rule.components.iter())
        {
            let Some(component_events) = remove_events.get(component_id) else {
                continue;
            };

            // Removed components are grouped by type, not by entity, so we need an intermediate container.
            let reader = readers.entry(component_id).or_default();
            for entity in reader
                .read(component_events)
                .cloned()
                .map(Into::into)
                .filter(|&entity| replicatred.get(entity).is_ok())
            {
                entity_removals.insert(entity, component_id);
            }
        }

        removal_buffer.read_removals(&entity_removals, &replication_rules);
        entity_removals.clear();
    }
}

/// An intermediate container to group removals by entity.
#[derive(Default)]
struct EntityRemovals {
    /// Component removals grouped by entity.
    removals: EntityHashMap<HashSet<ComponentId>>,

    /// [`HashSet`]'s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Vec<HashSet<ComponentId>>,
}

impl EntityRemovals {
    /// Registers component removal for the specified entity.
    fn insert(&mut self, entity: Entity, component_id: ComponentId) {
        self.removals
            .entry(entity)
            .or_insert_with(|| self.ids_buffer.pop().unwrap_or_default())
            .insert(component_id);
    }

    /// Clears all removals.
    ///
    /// Keeps the allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.ids_buffer
            .extend(self.removals.drain().map(|(_, mut components)| {
                components.clear();
                components
            }));
    }
}

/// Buffer with replication rule removals.
#[derive(Default, Resource)]
pub(crate) struct RemovalBuffer {
    /// Replication rule removals for entities.
    removals: Vec<(Entity, Vec<RemoveFnId>)>,

    /// [`Vec`]'s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Vec<Vec<RemoveFnId>>,
}

impl RemovalBuffer {
    /// Returns an iterator over entities and their removed replication rules.
    pub(super) fn iter(&self) -> impl Iterator<Item = (Entity, &[RemoveFnId])> {
        self.removals
            .iter()
            .map(|(entity, remove_ids)| (*entity, &**remove_ids))
    }

    /// Converts component removals into replication rule removals.
    fn read_removals(
        &mut self,
        entity_removals: &EntityRemovals,
        replication_rules: &ReplicationRules,
    ) {
        for (entity, components) in &entity_removals.removals {
            let mut removed_ids = self.ids_buffer.pop().unwrap_or_default();
            for replication_rule in replication_rules.iter() {
                if replication_rule.matches(components) {
                    removed_ids.push(replication_rule.remove_id);
                }
            }
            self.removals.push((*entity, removed_ids));
        }
    }

    /// Clears all removals.
    ///
    /// Keeps the allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.ids_buffer
            .extend(self.removals.drain(..).map(|(_, mut components)| {
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
            replication_fns::ReplicationFns, replication_rules::AppReplicationExt, Replication,
        },
        server::replicon_server::RepliconServer,
    };

    #[test]
    fn removals() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
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
        assert_eq!(removal_buffer.ids_buffer.len(), 1);
    }

    #[test]
    fn despawn_ignore() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
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
