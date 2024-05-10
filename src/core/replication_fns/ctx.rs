use bevy::prelude::*;

use crate::{
    client::server_entity_map::ServerEntityMap, core::replicon_tick::RepliconTick, Replicated,
};

/// Replication context for serialization function.
#[non_exhaustive]
pub struct SerializeCtx {
    /// Current tick.
    pub server_tick: RepliconTick,
}

/// Replication context for writing and deserialization.
#[non_exhaustive]
pub struct WriteCtx<'a, 'w, 's> {
    /// A queue to perform structural changes to the [`World`].
    pub commands: &'a mut Commands<'w, 's>,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a mut ServerEntityMap,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,

    /// Disables mapping logic to avoid spawning entities for consume functions.
    pub(super) ignore_mapping: bool,
}

impl<'a, 'w, 's> WriteCtx<'a, 'w, 's> {
    pub(crate) fn new(
        commands: &'a mut Commands<'w, 's>,
        entity_map: &'a mut ServerEntityMap,
        message_tick: RepliconTick,
    ) -> Self {
        Self {
            commands,
            entity_map,
            message_tick,
            ignore_mapping: false,
        }
    }
}

impl EntityMapper for WriteCtx<'_, '_, '_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        if self.ignore_mapping {
            return entity;
        }

        self.entity_map
            .get_by_server_or_insert(entity, || self.commands.spawn(Replicated).id())
    }
}

/// Replication context for removal.
#[non_exhaustive]
pub struct RemoveCtx<'a, 'w, 's> {
    /// A queue to perform structural changes to the [`World`].
    pub commands: &'a mut Commands<'w, 's>,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

impl<'a, 'w, 's> RemoveCtx<'a, 'w, 's> {
    pub(crate) fn new(commands: &'a mut Commands<'w, 's>, message_tick: RepliconTick) -> Self {
        Self {
            commands,
            message_tick,
        }
    }
}

/// Replication context for despawn.
#[non_exhaustive]
pub struct DespawnCtx {
    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}
