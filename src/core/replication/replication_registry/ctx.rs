use bevy::{ecs::component::ComponentId, prelude::*, reflect::TypeRegistry};

use crate::core::{
    replication::Replicated, replicon_tick::RepliconTick, server_entity_map::ServerEntityMap,
};

/// Replication context for serialization function.
#[non_exhaustive]
pub struct SerializeCtx<'a> {
    /// ID of the serializing component.
    pub component_id: ComponentId,

    /// Current tick.
    pub server_tick: RepliconTick,

    /// Registry of reflected types.
    pub type_registry: &'a TypeRegistry,
}

/// Replication context for writing and deserialization.
#[non_exhaustive]
pub struct WriteCtx<'a, 'w, 's> {
    /// A queue to perform structural changes to the [`World`].
    pub commands: &'a mut Commands<'w, 's>,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a mut ServerEntityMap,

    /// Registry of reflected types.
    pub type_registry: &'a TypeRegistry,

    /// ID of the writing component.
    pub component_id: ComponentId,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,

    /// Disables mapping logic to avoid spawning entities for consume functions.
    pub(crate) ignore_mapping: bool,
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

    /// ID of the removing component.
    pub component_id: ComponentId,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

/// Replication context for despawn.
#[non_exhaustive]
pub struct DespawnCtx {
    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}
