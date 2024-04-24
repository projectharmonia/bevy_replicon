use bevy::prelude::*;

use crate::{
    client::server_entity_map::ServerEntityMap, core::replicon_tick::RepliconTick, Replicated,
};

/// Replication context for serialization function.
#[non_exhaustive]
pub struct SerializeCtx {
    /// Current tick.
    pub replicon_tick: RepliconTick,
}

/// Replication context for writing and deserialization.
#[non_exhaustive]
pub struct WriteDeserializeCtx<'a, 'w, 's> {
    /// A queue to perform structural changes to the [`World`].
    pub commands: &'a mut Commands<'w, 's>,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a mut ServerEntityMap,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

impl EntityMapper for WriteDeserializeCtx<'_, '_, '_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        self.entity_map
            .get_by_server_or_insert(entity, || self.commands.spawn(Replicated).id())
    }
}

/// Replication context for removal and despawn functions.
#[non_exhaustive]
pub struct RemoveDespawnCtx {
    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}
