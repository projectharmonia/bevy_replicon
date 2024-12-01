use std::ops::Range;

use bevy::{prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use integer_encoding::VarIntWriter;

use crate::{
    core::{
        replication::replication_registry::{
            component_fns::ComponentFns, ctx::SerializeCtx, rule_fns::UntypedRuleFns, FnsId,
        },
        replicon_tick::RepliconTick,
    },
    server::client_entity_map::ClientMapping,
};

/// Single continious buffer that stores serialized data for messages.
///
/// See [`UpdateMessage`](super::update_message::UpdateMessage) and
/// [`MutateMessage`](super::mutate_message::MutateMessage).
#[derive(Default, Deref, DerefMut)]
pub(crate) struct SerializedData(Vec<u8>);

impl SerializedData {
    pub(crate) fn write_mappings(
        &mut self,
        mappings: impl Iterator<Item = ClientMapping>,
    ) -> bincode::Result<Range<usize>> {
        let start = self.len();

        for mapping in mappings {
            self.write_entity(mapping.server_entity)?;
            self.write_entity(mapping.client_entity)?;
        }

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_fn_ids(
        &mut self,
        fn_ids: impl Iterator<Item = FnsId>,
    ) -> bincode::Result<Range<usize>> {
        let start = self.len();

        for fns_id in fn_ids {
            DefaultOptions::new().serialize_into(&mut self.0, &fns_id)?;
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
    ) -> bincode::Result<Range<usize>> {
        let start = self.len();

        DefaultOptions::new().serialize_into(&mut self.0, &fns_id)?;
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
    pub(crate) fn write_entity(&mut self, entity: Entity) -> bincode::Result<Range<usize>> {
        let start = self.len();

        let mut flagged_index = (entity.index() as u64) << 1;
        let flag = entity.generation() > 1;
        flagged_index |= flag as u64;

        self.0.write_varint(flagged_index)?;
        if flag {
            self.0.write_varint(entity.generation() - 1)?;
        }

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_tick(&mut self, tick: RepliconTick) -> bincode::Result<Range<usize>> {
        let start = self.len();

        // Use fixedint encoding as serializing ticks as varints increases the average message size.
        // A tick >= 2^16 will be 5 bytes: https://docs.rs/bincode/1.3.3/bincode/config/struct.VarintEncoding.html.
        // At 60 ticks/sec, that will happen after 18 minutes.
        // So any session over 36 minutes would transmit more total bytes with varint encoding.
        // TODO: consider dynamically switching from varint to fixint encoding using one of the `UpdateMessageFlags` when tick sizes get large enough.
        bincode::serialize_into(&mut self.0, &tick)?;
        let end = self.len();

        Ok(start..end)
    }
}
