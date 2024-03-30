use std::mem;

use bevy::{
    ecs::{
        archetype::{ArchetypeGeneration, ArchetypeId, Archetypes},
        component::{ComponentId, StorageType},
    },
    prelude::*,
};

use crate::core::{
    replication_fns::ComponentFnsId, replication_rules::ReplicationRules, Replication,
};

/// Cached information about all replicated archetypes.
#[derive(Deref)]
pub(crate) struct ReplicatedArchetypes {
    /// ID of [`Replication`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    generation: ArchetypeGeneration,

    /// Archetypes marked as replicated.
    #[deref]
    archetypes: Vec<ReplicatedArchetype>,
}

impl ReplicatedArchetypes {
    /// ID of [`Replication`] component.
    pub(crate) fn marker_id(&self) -> ComponentId {
        self.marker_id
    }

    /// Updates internal view of the [`World`]'s replicated archetypes.
    ///
    /// If this is not called before querying data, the results may not accurately reflect what is in the world.
    pub(super) fn update(&mut self, archetypes: &Archetypes, rules: &ReplicationRules) {
        let old_generation = mem::replace(&mut self.generation, archetypes.generation());

        // Archetypes are never removed, iterate over newly added since the last update.
        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(self.marker_id))
        {
            let mut replicated_archetype = ReplicatedArchetype::new(archetype.id());
            for rule in rules.iter().filter(|rule| rule.matches(archetype)) {
                for &(component_id, fns_id) in &rule.components {
                    // Since rules are sorted by priority,
                    // we are inserting only new components that aren't present.
                    if replicated_archetype
                        .components
                        .iter()
                        .any(|component| component.component_id == component_id)
                    {
                        continue;
                    }

                    // SAFETY: component ID obtained from this archetype.
                    let storage_type =
                        unsafe { archetype.get_storage_type(component_id).unwrap_unchecked() };

                    replicated_archetype.components.push(ReplicatedComponent {
                        component_id,
                        storage_type,
                        fns_id,
                    });
                }
            }
            self.archetypes.push(replicated_archetype);
        }
    }
}

impl FromWorld for ReplicatedArchetypes {
    fn from_world(world: &mut World) -> Self {
        Self {
            marker_id: world.init_component::<Replication>(),
            generation: ArchetypeGeneration::initial(),
            archetypes: Default::default(),
        }
    }
}

/// An archetype that can be stored in [`ReplicatedArchetypes`].
pub(crate) struct ReplicatedArchetype {
    /// Associated archetype ID.
    pub(super) id: ArchetypeId,

    /// Components marked as replicated.
    pub(super) components: Vec<ReplicatedComponent>,
}

impl ReplicatedArchetype {
    fn new(id: ArchetypeId) -> Self {
        Self {
            id,
            components: Default::default(),
        }
    }
}

/// Stores information about a replicated component.
pub(super) struct ReplicatedComponent {
    pub(super) component_id: ComponentId,
    pub(super) storage_type: StorageType,
    pub(super) fns_id: ComponentFnsId,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{core::replication_fns::ReplicationFns, AppReplicationExt};

    #[test]
    fn empty() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>();

        app.world.spawn_empty();

        let replicated_archetypes = match_archetypes(&mut app.world);
        assert!(replicated_archetypes.is_empty());
    }

    #[test]
    fn no_components() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>();

        app.world.spawn(Replication);

        let replicated_archetypes = match_archetypes(&mut app.world);
        assert_eq!(replicated_archetypes.len(), 1);

        let replicated_component = replicated_archetypes.first().unwrap();
        assert!(replicated_component.components.is_empty());
    }

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate::<ComponentA>();

        app.world.spawn((Replication, ComponentB));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert!(replicated_component.components.is_empty());
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate::<ComponentA>();

        app.world.spawn((Replication, ComponentA));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert_eq!(replicated_component.components.len(), 1);
    }

    #[test]
    fn group() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.spawn((Replication, ComponentA, ComponentB));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert_eq!(replicated_component.components.len(), 2);
    }

    #[test]
    fn part_of_group() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.spawn((Replication, ComponentA));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert!(replicated_component.components.is_empty());
    }

    #[test]
    fn grup_with_subset() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate::<ComponentA>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.spawn((Replication, ComponentA, ComponentB));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert_eq!(replicated_component.components.len(), 2);
    }

    #[test]
    fn group_with_multiple_subsets() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate::<ComponentA>()
            .replicate::<ComponentB>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world.spawn((Replication, ComponentA, ComponentB));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert_eq!(replicated_component.components.len(), 2);
    }

    #[test]
    fn groups_with_overlap() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate_group::<(ComponentA, ComponentC)>()
            .replicate_group::<(ComponentA, ComponentB)>();

        app.world
            .spawn((Replication, ComponentA, ComponentB, ComponentC));

        let replicated_archetypes = match_archetypes(&mut app.world);
        let replicated_component = replicated_archetypes.first().unwrap();
        assert_eq!(replicated_component.components.len(), 3);
    }

    fn match_archetypes(world: &mut World) -> ReplicatedArchetypes {
        let mut replicated_archetypes = ReplicatedArchetypes::from_world(world);
        replicated_archetypes.update(world.archetypes(), world.resource::<ReplicationRules>());

        replicated_archetypes
    }

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentA;

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentB;

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentC;
}
