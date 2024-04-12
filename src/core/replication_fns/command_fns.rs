use std::io::Cursor;

use bevy::{
    ecs::{component::ComponentId, system::EntityCommands},
    prelude::*,
    ptr::Ptr,
};

use super::serde_fns::SerdeFns;
use crate::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::replicon_tick::RepliconTick,
};

pub(crate) struct CommandFns {
    component_id: ComponentId,
    read: ReadFn,
    write: WriteFn,
    remove: RemoveFn,
}

impl CommandFns {
    pub(super) fn new<C: Component>(component_id: ComponentId) -> Self {
        Self {
            component_id,
            read: read::<C>,
            write: write::<C>,
            remove: remove::<C>,
        }
    }

    pub(crate) unsafe fn read(
        &self,
        serde_fns: &SerdeFns,
        ptr: Ptr,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        (self.read)(serde_fns, ptr, cursor)
    }

    pub(crate) unsafe fn write(
        &self,
        rule_fns: &SerdeFns,
        commands: &mut Commands,
        entity: &mut EntityMut,
        cursor: &mut Cursor<&[u8]>,
        entity_map: &mut ServerEntityMap,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        (self.write)(
            rule_fns,
            commands,
            entity,
            cursor,
            entity_map,
            replicon_tick,
        )
    }

    pub(crate) fn remove(&self, entity_commands: EntityCommands, replicon_tick: RepliconTick) {
        (self.remove)(entity_commands, replicon_tick)
    }

    pub(super) fn component_id(&self) -> ComponentId {
        self.component_id
    }
}

/// Signature of component reading function.
pub type ReadFn = unsafe fn(&SerdeFns, Ptr, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component writing function.
pub type WriteFn = unsafe fn(
    &SerdeFns,
    &mut Commands,
    &mut EntityMut,
    &mut Cursor<&[u8]>,
    &mut ServerEntityMap,
    RepliconTick,
) -> bincode::Result<()>;

/// Signature of component removal functions.
pub type RemoveFn = fn(EntityCommands, RepliconTick);

/// Default component serialization function.
///
/// # Safety
///
/// `C` must be the erased pointee type for this [`Ptr`].
unsafe fn read<C>(
    rule_fns: &SerdeFns,
    ptr: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    rule_fns.serialize(ptr.deref::<C>(), cursor)
}

/// Default component writing function.
unsafe fn write<C: Component>(
    rule_fns: &SerdeFns,
    commands: &mut Commands,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
    entity_map: &mut ServerEntityMap,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let mut mapper = ClientMapper {
        commands,
        entity_map,
    };

    if let Some(mut component) = entity.get_mut::<C>() {
        rule_fns.deserialize_in_place(&mut *component, cursor, &mut mapper)?;
    } else {
        let component: C = rule_fns.deserialize(cursor, &mut mapper)?;
        commands.entity(entity.id()).insert(component);
    }

    Ok(())
}

/// Default component removal function.
fn remove<C: Component>(mut entity_commands: EntityCommands, _replicon_tick: RepliconTick) {
    entity_commands.remove::<C>();
}
