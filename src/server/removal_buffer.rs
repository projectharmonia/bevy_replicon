use bevy::{
    ecs::{
        archetype::{Archetype, Archetypes},
        component::ComponentId,
        entity::{Entities, EntityHashMap},
        event::ManualEventReader,
        removal_detection::{RemovedComponentEntity, RemovedComponentEvents},
        system::SystemParam,
    },
    prelude::*,
    utils::{HashMap, HashSet},
};

use super::{ServerPlugin, ServerSet};
use crate::core::{
    common_conditions::server_running, replication_fns::FnsInfo,
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
        entities: &Entities,
        archetypes: &Archetypes,
        mut removal_reader: RemovalReader,
        mut removal_buffer: ResMut<RemovalBuffer>,
        rules: Res<ReplicationRules>,
    ) {
        for (&entity, components) in removal_reader.read() {
            let location = entities
                .get(entity)
                .expect("removals count only existing entities");
            let archetype = archetypes.get(location.archetype_id).unwrap();

            removal_buffer.update(&rules, archetype, entity, components);
        }
    }
}

/// Reader for removed components.
///
/// Like [`RemovedComponentEvents`], but reads them in per-entity format.
#[derive(SystemParam)]
struct RemovalReader<'w, 's> {
    /// Cached components list from [`ReplicationRules`].
    components: Local<'s, ReplicatedComponents>,

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

    /// Filter for replicated and valid entities.
    replicated: Query<'w, 's, (), With<Replication>>,
}

impl RemovalReader<'_, '_> {
    /// Returns iterator over all components removed since the last call.
    ///
    /// Only replicated entities taken into account.
    fn read(&mut self) -> impl Iterator<Item = (&Entity, &HashSet<ComponentId>)> {
        self.clear();

        // TODO: Ask Bevy to provide an iterator over `RemovedComponentEvents`.
        for &component_id in &self.components.0 {
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

struct ReplicatedComponents(HashSet<ComponentId>);

impl FromWorld for ReplicatedComponents {
    fn from_world(world: &mut World) -> Self {
        let rules = world.resource::<ReplicationRules>();
        let component_ids = rules
            .iter()
            .flat_map(|rule| rule.components())
            .map(|fns_info| fns_info.component_id())
            .collect();

        Self(component_ids)
    }
}

/// Buffer with removed components.
#[derive(Default, Resource)]
pub(crate) struct RemovalBuffer {
    /// Component removals grouped by entity.
    removals: Vec<(Entity, Vec<FnsInfo>)>,

    /// [`Vec`]s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Vec<Vec<FnsInfo>>,
}

impl RemovalBuffer {
    /// Returns an iterator over entities and their removed components.
    pub(super) fn iter(&self) -> impl Iterator<Item = (Entity, &[FnsInfo])> {
        self.removals
            .iter()
            .map(|(entity, remove_ids)| (*entity, &**remove_ids))
    }

    /// Registers component removals that match replication rules for an entity.
    fn update(
        &mut self,
        rules: &ReplicationRules,
        archetype: &Archetype,
        entity: Entity,
        components: &HashSet<ComponentId>,
    ) {
        let mut removed_ids = self.ids_buffer.pop().unwrap_or_default();
        for rule in rules
            .iter()
            .filter(|rule| rule.matches_removals(archetype, components))
        {
            for &fns_info in rule.components() {
                // Since rules are sorted by priority,
                // we are inserting only new components that aren't present.
                if removed_ids
                    .iter()
                    .all(|removed_info| removed_info.component_id() != fns_info.component_id())
                    && !archetype.contains(fns_info.component_id())
                {
                    removed_ids.push(fns_info);
                }
            }
        }
        self.removals.push((entity, removed_ids));
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
    fn not_replicated() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((Replication, ComponentA))
            .remove::<ComponentA>();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert!(removal_buffer.removals.is_empty());
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((Replication, ComponentA))
            .remove::<ComponentA>();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let (_, removals_id) = removal_buffer.removals.first().unwrap();
        assert_eq!(removals_id.len(), 1);
    }

    #[test]
    fn group() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((Replication, ComponentA, ComponentB))
            .remove::<(ComponentA, ComponentB)>();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let (_, removals_id) = removal_buffer.removals.first().unwrap();
        assert_eq!(removals_id.len(), 2);
    }

    #[test]
    fn part_of_group() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((Replication, ComponentA, ComponentB))
            .remove::<ComponentA>();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let (_, removals_id) = removal_buffer.removals.first().unwrap();
        assert_eq!(removals_id.len(), 1);
    }

    #[test]
    fn group_with_subset() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((Replication, ComponentA, ComponentB))
            .remove::<(ComponentA, ComponentB)>();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let (_, removals_id) = removal_buffer.removals.first().unwrap();
        assert_eq!(removals_id.len(), 2);
    }

    #[test]
    fn part_of_group_with_subset() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world
            .spawn((Replication, ComponentA, ComponentB))
            .remove::<ComponentA>();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let (_, removals_id) = removal_buffer.removals.first().unwrap();
        assert_eq!(removals_id.len(), 1);
    }

    #[test]
    fn despawn() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationFns>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>();

        app.world.resource_mut::<RepliconServer>().set_running(true);

        app.update();

        app.world.spawn((ComponentA, Replication)).despawn();

        app.update();

        let removal_buffer = app.world.resource::<RemovalBuffer>();
        assert!(
            removal_buffer.removals.is_empty(),
            "despawns shouldn't be counted as removals"
        );
    }

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentA;

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentB;
}
