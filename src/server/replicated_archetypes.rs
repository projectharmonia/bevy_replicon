use bevy::{
    ecs::{
        archetype::ArchetypeId,
        component::{ComponentId, StorageType},
    },
    prelude::*,
};

use crate::core::replication_fns::ComponentFnsIndex;

/// Stores cached information about all replicated archetypes.
#[derive(Resource, Default)]
pub struct ReplicatedArchetypes(Vec<ReplicatedArchetype>);

impl ReplicatedArchetypes {
    /// Marks an archetype as replicated and returns a mutable reference to its data.
    ///
    /// # Safety
    ///
    /// ID of [`ReplicatedArchetype`] should exist in [`Archetypes`](bevy::ecs::archetype::Archetypes).
    pub unsafe fn add_archetype(&mut self, replicated_archetype: ReplicatedArchetype) {
        self.0.push(replicated_archetype);
    }

    /// Returns an iterator over replicated archetypes.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &ReplicatedArchetype> {
        self.0.iter()
    }
}

pub struct ReplicatedArchetype {
    id: ArchetypeId,
    components: Vec<ReplicatedComponent>,
}

impl ReplicatedArchetype {
    /// Creates a replicated archetype with no components.
    pub fn new(id: ArchetypeId) -> Self {
        Self {
            id,
            components: Default::default(),
        }
    }

    /// Adds replicated component to the archetype.
    ///
    /// # Safety
    ///
    /// - Component should be present in the archetype.
    /// - Functions index and storage type should correspond to this component.
    pub unsafe fn add_component(&mut self, replicated_component: ReplicatedComponent) {
        self.components.push(replicated_component);
    }

    /// Returns associated archetype ID.
    #[must_use]
    pub(crate) fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Returns component marked as replicated.
    #[must_use]
    pub(crate) fn components(&self) -> &[ReplicatedComponent] {
        &self.components
    }
}

/// Stores information about replicated component.
pub struct ReplicatedComponent {
    pub component_id: ComponentId,
    pub storage_type: StorageType,
    pub fns_index: ComponentFnsIndex,
}
