use std::ops::Range;

use bevy::{prelude::*, ptr::Ptr};

use crate::core::{
    entity_serde, postcard_utils,
    replication::replication_registry::{
        component_fns::ComponentFns, ctx::SerializeCtx, rule_fns::UntypedRuleFns, FnsId,
    },
    replicon_tick::RepliconTick,
};

/// Single continuous buffer that stores serialized data for messages.
///
/// See [`UpdateMessage`](super::update_message::UpdateMessage) and
/// [`MutateMessage`](super::mutate_message::MutateMessage).
#[derive(Default, Deref, DerefMut)]
pub(crate) struct SerializedData(Vec<u8>);

impl SerializedData {
    pub(crate) fn write_mappings(
        &mut self,
        mappings: impl Iterator<Item = (Entity, Entity)>,
    ) -> postcard::Result<Range<usize>> {
        let start = self.len();

        for (server_entity, client_entity) in mappings {
            self.write_entity(server_entity)?;
            self.write_entity(client_entity)?;
        }

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_fn_ids(
        &mut self,
        fn_ids: impl Iterator<Item = FnsId>,
    ) -> postcard::Result<Range<usize>> {
        let start = self.len();

        for fns_id in fn_ids {
            postcard_utils::to_extend_mut(&fns_id, &mut self.0)?;
        }

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_component(
        &mut self,
        rule_fns: &UntypedRuleFns,
        component_fns: &ComponentFns,
        ctx: &SerializeCtx,
        fns_id: FnsId,
        ptr: Ptr,
    ) -> postcard::Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&fns_id, &mut self.0)?;
        // SAFETY: `component_fns`, `ptr` and `rule_fns` were created for the same component type.
        unsafe { component_fns.serialize(ctx, rule_fns, ptr, &mut self.0)? };

        let end = self.len();

        Ok(start..end)
    }

    /// Serializes `entity` by writing its index and generation as separate varints.
    ///
    /// The index is first prepended with a bit flag to indicate if the generation
    /// is serialized or not. It is not serialized if <= 1; note that generations are [`NonZeroU32`](std::num::NonZeroU32)
    /// and a value of zero is used in [`Option<Entity>`] to signify [`None`], so generation 1 is the first
    /// generation.
    pub(crate) fn write_entity(&mut self, entity: Entity) -> postcard::Result<Range<usize>> {
        let start = self.len();

        entity_serde::serialize_entity(&mut self.0, entity)?;

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_tick(&mut self, tick: RepliconTick) -> postcard::Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&tick, &mut self.0)?;

        let end = self.len();

        Ok(start..end)
    }
}
