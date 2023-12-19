use bevy::{
    ecs::{
        component::ComponentId,
        event::ManualEventReader,
        removal_detection::{RemovedComponentEntity, RemovedComponentEvents},
        system::SystemParam,
    },
    prelude::*,
    utils::{EntityHashMap, EntityHashSet, HashMap},
};

use crate::replicon_core::replication_rules::{ReplicationId, ReplicationRules};

/// Like [`RemovedComponents<T>`], but iterates over all removals and their [`ComponentId`]'s.
///
/// An abstraction over [`RemovedComponentEvents`] that groups removals by entity and tracks which events have already been read.
#[derive(SystemParam)]
pub(crate) struct RemovedComponentIds<'w, 's> {
    /// Removal events grouped by [`ComponentId`].
    events: &'w RemovedComponentEvents,

    /// Readers for each [`Events`] from `events` to track read events.
    readers: Local<'s, HashMap<ComponentId, ManualEventReader<RemovedComponentEntity>>>,

    /// Despawned entities that will be excluded from iteration.
    despawns: Local<'s, EntityHashSet<Entity>>,

    /// Intermediate buffer to group removals by entity.
    entity_buffer: Local<'s, EntityHashMap<Entity, Vec<ReplicationId>>>,

    /// Removed components from previous reading.
    ///
    /// Stored to reuse allocated capacity.
    component_buffer: Local<'s, Vec<Vec<ReplicationId>>>,
}

impl RemovedComponentIds<'_, '_> {
    /// Registers a despawn for the next [`Seld::read`].
    pub(super) fn register_despawn(&mut self, entity: Entity) {
        self.despawns.insert(entity);
    }

    /// Iterates over the removals this [`RemovedComponentIds`] has not seen yet.
    ///
    /// Despawned entities registered via [`Self::register_despawn`] since the last call will be skipped.
    pub(super) fn read(
        &mut self,
        replication_rules: &ReplicationRules,
    ) -> impl Iterator<Item = (Entity, &[ReplicationId])> {
        self.component_buffer
            .extend(self.entity_buffer.drain().map(|(_, components)| components));

        // Removed components are grouped by type, not by entity, so we need an intermediate container.
        for (&component_id, &replication_id) in replication_rules.get_ids() {
            for removals in self.events.get(component_id).into_iter() {
                let reader = self.readers.entry(component_id).or_default();
                for entity in reader
                    .read(removals)
                    .cloned()
                    .map(Into::into)
                    .filter(|entity| !self.despawns.contains(entity))
                {
                    self.entity_buffer
                        .entry(entity)
                        .or_insert_with(|| {
                            let mut components = self.component_buffer.pop().unwrap_or_default();
                            components.clear();
                            components
                        })
                        .push(replication_id);
                }
            }
        }

        self.despawns.clear();
        self.entity_buffer
            .iter()
            .map(|(&entity, entities)| (entity, &**entities))
    }
}
