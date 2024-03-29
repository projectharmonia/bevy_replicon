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
#[derive(Deref)]
pub(crate) struct ReplicatedArchetypes {
    /// ID of [`Replication`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    generation: ArchetypeGeneration,

    /// Archetypes marked as replicated.
    #[deref]
    archetypes: Vec<ReplicatedArchetype>,

    /// Temporary container for storing indices of rule subsets for currently processing archetype.
    ///
    /// Cleaned after each archetype processing.
    current_subsets: Vec<usize>,
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
            for (index, rule) in rules.iter().enumerate() {
                if self.current_subsets.contains(&index) || !rule.matches_archetype(archetype) {
                    continue;
                }

                for &(component_id, serde_id) in &rule.components {
                    // SAFETY: component ID obtained from this archetype.
                    let storage_type =
                        unsafe { archetype.get_storage_type(component_id).unwrap_unchecked() };

                    replicated_archetype.components.push(ReplicatedComponent {
                        component_id,
                        storage_type,
                        serde_id,
                    });
                }
                self.current_subsets.extend_from_slice(&rule.subsets);
            }
            self.archetypes.push(replicated_archetype);
            self.current_subsets.clear();
        }
    }
}

impl FromWorld for ReplicatedArchetypes {
    fn from_world(world: &mut World) -> Self {
        Self {
            marker_id: world.init_component::<Replication>(),
            generation: ArchetypeGeneration::initial(),
            archetypes: Default::default(),
            current_subsets: Default::default(),
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
    pub(super) serde_id: SerdeFnsId,
}
