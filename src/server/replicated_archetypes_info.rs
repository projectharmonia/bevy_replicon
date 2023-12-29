use std::mem;

use bevy::{
    ecs::{
        archetype::{ArchetypeGeneration, ArchetypeId, Archetypes},
        component::{ComponentId, StorageType},
    },
    prelude::*,
};

use crate::replicon_core::replication_rules::{ReplicationId, ReplicationInfo, ReplicationRules};

/// Stores cached information about all replicated archetypes.
pub(crate) struct ReplicatedArchetypesInfo {
    info: Vec<ReplicatedArchetypeInfo>,
    generation: ArchetypeGeneration,
}

impl ReplicatedArchetypesInfo {
    /// Returns an iterator over the archetypes.
    pub(super) fn iter(&self) -> impl Iterator<Item = &ReplicatedArchetypeInfo> {
        self.info.iter()
    }

    /// Updates internal view of the [`World`]'s replicated archetypes.
    ///
    /// If this is not called before querying data, the results may not accurately reflect what is in the world.
    pub(super) fn update(&mut self, archetypes: &Archetypes, replication_rules: &ReplicationRules) {
        let old_generation = mem::replace(&mut self.generation, archetypes.generation());

        // Archetypes are never removed, iterate over newly added since the last update.
        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(replication_rules.get_marker_id()))
        {
            let mut archetype_info = ReplicatedArchetypeInfo::new(archetype.id());
            for component_id in archetype.components() {
                let Some((replication_id, replication_info)) = replication_rules.get(component_id)
                else {
                    continue;
                };
                if archetype.contains(replication_info.not_replicate_id) {
                    continue;
                }

                // SAFETY: component ID obtained from this archetype.
                let storage_type =
                    unsafe { archetype.get_storage_type(component_id).unwrap_unchecked() };

                archetype_info.components.push(ReplicatedComponentInfo {
                    component_id,
                    storage_type,
                    replication_id,
                    replication_info: replication_info.clone(),
                });
            }

            self.info.push(archetype_info);
        }
    }
}

impl Default for ReplicatedArchetypesInfo {
    fn default() -> Self {
        Self {
            info: default(),
            generation: ArchetypeGeneration::initial(),
        }
    }
}

pub(super) struct ReplicatedArchetypeInfo {
    pub(super) id: ArchetypeId,
    pub(super) components: Vec<ReplicatedComponentInfo>,
}

impl ReplicatedArchetypeInfo {
    fn new(id: ArchetypeId) -> Self {
        Self {
            id,
            components: Default::default(),
        }
    }
}

pub(super) struct ReplicatedComponentInfo {
    pub(super) component_id: ComponentId,
    pub(super) storage_type: StorageType,
    pub(super) replication_id: ReplicationId,
    pub(super) replication_info: ReplicationInfo,
}
