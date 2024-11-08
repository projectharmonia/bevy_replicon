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
    common_conditions::server_running, replication_registry::FnsId,
    replication_rules::ReplicationRules, Replicated,
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
    replicated: Query<'w, 's, (), With<Replicated>>,
}

impl RemovalReader<'_, '_> {
    /// Returns iterator over all components removed since the last call.
    ///
    /// Only replicated entities taken into account.
    fn read(&mut self) -> impl Iterator<Item = (&Entity, &HashSet<ComponentId>)> {
        self.clear();

        for (&component_id, component_events) in self
            .remove_events
            .iter()
            .filter(|(component_id, _)| self.components.contains(*component_id))
        {
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

#[derive(Deref)]
struct ReplicatedComponents(HashSet<ComponentId>);

impl FromWorld for ReplicatedComponents {
    fn from_world(world: &mut World) -> Self {
        let rules = world.resource::<ReplicationRules>();
        let component_ids = rules
            .iter()
            .flat_map(|rule| &rule.components)
            .map(|&(component_id, _)| component_id)
            .collect();

        Self(component_ids)
    }
}

/// Buffer with removed components.
#[derive(Default, Resource, Deref)]
pub(crate) struct RemovalBuffer {
    /// Component removals grouped by entity.
    #[deref]
    removals: EntityHashMap<Vec<(ComponentId, FnsId)>>,

    /// [`Vec`]s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Vec<Vec<(ComponentId, FnsId)>>,
}

impl RemovalBuffer {
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
            for &(component_id, fns_id) in &rule.components {
                // Since rules are sorted by priority,
                // we are inserting only new components that aren't present.
                if removed_ids
                    .iter()
                    .all(|&(removed_id, _)| removed_id != component_id)
                    && !archetype.contains(component_id)
                {
                    removed_ids.push((component_id, fns_id));
                }
            }
        }

        if removed_ids.is_empty() {
            self.ids_buffer.push(removed_ids);
        } else {
            self.removals.insert(entity, removed_ids);
        }
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

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::core::{
        replication_registry::ReplicationRegistry, replication_rules::AppRuleExt,
        replicon_server::RepliconServer, Replicated,
    };

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        app.world_mut()
            .spawn((Replicated, ComponentA))
            .remove::<ComponentA>();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert!(removal_buffer.removals.is_empty());
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, ComponentA))
            .remove::<ComponentA>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removals_id = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removals_id.len(), 1);
    }

    #[test]
    fn group() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, ComponentA, ComponentB))
            .remove::<(ComponentA, ComponentB)>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removals_id = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removals_id.len(), 2);
    }

    #[test]
    fn part_of_group() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, ComponentA, ComponentB))
            .remove::<ComponentA>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removals_id = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removals_id.len(), 1);
    }

    #[test]
    fn group_with_subset() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, ComponentA, ComponentB))
            .remove::<(ComponentA, ComponentB)>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removals_id = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removals_id.len(), 2);
    }

    #[test]
    fn part_of_group_with_subset() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, ComponentA, ComponentB))
            .remove::<ComponentA>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removals_id = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removals_id.len(), 1);
    }

    #[test]
    fn despawn() {
        let mut app = App::new();
        app.add_plugins(RemovalBufferPlugin)
            .init_resource::<RepliconServer>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .replicate::<ComponentA>();

        app.world_mut()
            .resource_mut::<RepliconServer>()
            .set_running(true);

        app.update();

        app.world_mut().spawn((ComponentA, Replicated)).despawn();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
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
