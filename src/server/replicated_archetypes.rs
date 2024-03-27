use std::mem;

use bevy::{
    ecs::{
        archetype::{ArchetypeGeneration, ArchetypeId, Archetypes},
        component::{ComponentId, StorageType},
    },
    prelude::*,
};

use crate::core::{replication_fns::SerdeFnsId, replication_rules::ReplicationRules, Replication};

/// Cached information about all replicated archetypes.
pub(crate) struct ReplicatedArchetypes {
    /// ID of [`Replication`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    generation: ArchetypeGeneration,

    /// Archetypes marked as replicated.
    archetypes: Vec<ReplicatedArchetype>,
}

impl ReplicatedArchetypes {
    /// ID of [`Replication`] component.
    pub(crate) fn marker_id(&self) -> ComponentId {
        self.marker_id
    }

    /// Returns an iterator over the archetypes.
    pub(super) fn iter(&self) -> impl Iterator<Item = &ReplicatedArchetype> {
        self.archetypes.iter()
    }

    /// Updates internal view of the [`World`]'s replicated archetypes.
    ///
    /// If this is not called before querying data, the results may not accurately reflect what is in the world.
    pub(super) fn update(&mut self, archetypes: &Archetypes, replication_rules: &ReplicationRules) {
        let old_generation = mem::replace(&mut self.generation, archetypes.generation());

        // Archetypes are never removed, iterate over newly added since the last update.
        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(self.marker_id))
        {
            let mut replicated_archetype = ReplicatedArchetype::new(archetype.id());
            for replication_rule in replication_rules
                .iter()
                .filter(|rule| rule.matches_archetype(archetype))
            {
                for &(component_id, serde_id) in &replication_rule.components {
                    // SAFETY: component ID obtained from this archetype.
                    let storage_type =
                        unsafe { archetype.get_storage_type(component_id).unwrap_unchecked() };

                    replicated_archetype.components.push(ReplicatedComponent {
                        component_id,
                        storage_type,
                        serde_id,
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
pub(super) struct ReplicatedArchetype {
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
    pub(super) serde_id: SerdeFnsId,
}
