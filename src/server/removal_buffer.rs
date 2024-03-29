use bevy::{
    ecs::{
        component::ComponentId,
        entity::EntityHashMap,
        event::ManualEventReader,
        removal_detection::{RemovedComponentEntity, RemovedComponentEvents},
        system::SystemParam,
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
        mut removal_reader: RemovalReader,
        mut removal_buffer: ResMut<RemovalBuffer>,
        rules: Res<ReplicationRules>,
    ) {
        for (&entity, components) in removal_reader.read() {
            removal_buffer.update(&rules, entity, components);
        }
    }
}

/// Reads removals and returns them in per-entity format, unlike [`RemovedComponentEvents`].
#[derive(SystemParam)]
struct RemovalReader<'w, 's> {
    /// Individual readers for each component.
    readers: Local<'s, HashMap<ComponentId, ManualEventReader<RemovedComponentEntity>>>,

    /// Component removals grouped by entity.
    removals: Local<'s, EntityHashMap<HashSet<ComponentId>>>,

    /// [`HashSet`]'s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Local<'s, Vec<HashSet<ComponentId>>>,

    /// Component removals grouped by [`ComponentId`].
    remove_events: &'w RemovedComponentEvents,

    /// Filter for replicated components
    rules: Res<'w, ReplicationRules>,

    /// Checks is an entity exists and replicated.
    replicated: Query<'w, 's, (), With<Replication>>,
}

impl RemovalReader<'_, '_> {
    /// Returns iterator over all components removed since the last call.
    fn read(&mut self) -> impl Iterator<Item = (&Entity, &HashSet<ComponentId>)> {
        self.clear();

        // TODO: Ask Bevy to provide an iterator over `RemovedComponentEvents`.
        for &(component_id, _) in self.rules.iter().flat_map(|rule| &rule.components) {
            let Some(component_events) = self.remove_events.get(component_id) else {
                continue;
            };

            // Removed components are grouped by type, not by entity, so we need an intermediate container.
            let reader = self.readers.entry(component_id).or_default();
            for entity in reader
                .read(component_events)
                .cloned()
                .map(Into::into)
                .filter(|&entity| self.replicated.get(entity).is_ok())
            {
                self.removals
                    .entry(entity)
                    .or_insert_with(|| self.ids_buffer.pop().unwrap_or_default())
                    .insert(component_id);
            }
        }

        self.removals.iter()
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

    /// Temporary container for storing indices of rule subsets for currently reading entity.
    ///
    /// Cleaned after each entity reading.
    current_subsets: Vec<usize>,
}

impl RemovalBuffer {
    /// Returns an iterator over entities and their removed replication rules.
    pub(super) fn iter(&self) -> impl Iterator<Item = (Entity, &[RemoveFnId])> {
        self.removals
            .iter()
            .map(|(entity, remove_ids)| (*entity, &**remove_ids))
    }

    /// Reads component removals and stores them as replication rule removals.
    fn update(
        &mut self,
        rules: &ReplicationRules,
        entity: Entity,
        components: &HashSet<ComponentId>,
    ) {
        let mut removed_ids = self.ids_buffer.pop().unwrap_or_default();
        for (index, rule) in rules.iter().enumerate() {
            if !self.current_subsets.contains(&index) && rule.matches(components) {
                removed_ids.push(rule.remove_id);
                self.current_subsets.extend_from_slice(&rule.subsets);
            }
        }
        self.removals.push((entity, removed_ids));
        self.current_subsets.clear();
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
