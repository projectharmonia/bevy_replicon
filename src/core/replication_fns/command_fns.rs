use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{ecs::system::EntityCommands, prelude::*};

use super::rule_fns::RuleFns;
use crate::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::replicon_tick::RepliconTick,
};

/// Writing and removal functions for a component, like [`Commands`].
#[derive(Clone, Copy)]
pub(super) struct UntypedCommandFns {
    type_id: TypeId,
    type_name: &'static str,

    write: unsafe fn(),
    remove: RemoveFn,
}

impl UntypedCommandFns {
    /// Creates a new instance with default command functions for `C`.
    pub(super) fn default_fns<C: Component>() -> Self {
        Self::new(default_write::<C>, default_remove::<C>)
    }

    /// Creates a new instance by erasing the function pointer for `write`.
    pub(super) fn new<C: Component>(write: WriteFn<C>, remove: RemoveFn) -> Self {
        Self {
            type_id: TypeId::of::<C>(),
            type_name: any::type_name::<C>(),
            // SAFETY: the function won't be called until the type is restored.
            write: unsafe { mem::transmute(write) },
            remove,
        }
    }

    /// Calls the assigned writing function.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which this instance was created.
    pub(super) unsafe fn write<C: Component>(
        &self,
        rule_fns: &RuleFns<C>,
        commands: &mut Commands,
        entity: &mut EntityMut,
        cursor: &mut Cursor<&[u8]>,
        entity_map: &mut ServerEntityMap,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        debug_assert_eq!(
            self.type_id,
            TypeId::of::<C>(),
            "trying to call a command write function with {}, but it was created with {}",
            any::type_name::<C>(),
            self.type_name,
        );

        let write: WriteFn<C> = unsafe { mem::transmute(self.write) };
        (write)(
            rule_fns,
            commands,
            entity,
            cursor,
            entity_map,
            replicon_tick,
        )
    }

    /// Calls the assigned removal function.
    pub(super) fn remove(&self, commands: EntityCommands, tick: RepliconTick) {
        (self.remove)(commands, tick);
    }
}

/// Signature of component writing function.
pub type WriteFn<C> = fn(
    &RuleFns<C>,
    &mut Commands,
    &mut EntityMut,
    &mut Cursor<&[u8]>,
    &mut ServerEntityMap,
    RepliconTick,
) -> bincode::Result<()>;

/// Signature of component removal functions.
pub type RemoveFn = fn(EntityCommands, RepliconTick);

/// Default component writing function.
///
/// If the component does not exist on the entity, it will be deserialized with [`RuleFns::deserialize`] and inserted via [`Commands`].
/// If the component exists on the entity, [`RuleFns::deserialize_in_place`] will be used directly on the entity's component.
pub fn default_write<C: Component>(
    rule_fns: &RuleFns<C>,
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
pub fn default_remove<C: Component>(
    mut entity_commands: EntityCommands,
    _replicon_tick: RepliconTick,
) {
    entity_commands.remove::<C>();
}
