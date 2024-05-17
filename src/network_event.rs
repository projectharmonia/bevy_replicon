pub mod client_event;
pub mod server_event;

use std::any::Any;

use bevy::{
    ecs::entity::EntityHashMap,
    prelude::*,
    reflect::{erased_serde::Serialize, TypeRegistry},
};
use bytes::Bytes;

use crate::core::replicon_tick::RepliconTick;

/// Maps server entities into client entities inside events.
///
/// Panics if a mapping doesn't exists.
pub struct EventMapper<'a>(pub &'a EntityHashMap<Entity>);

impl EntityMapper for EventMapper<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self
            .0
            .get(&entity)
            .unwrap_or_else(|| panic!("{entity:?} should be mappable"))
    }
}

pub type SendFn = fn(&mut World, u8, UntypedSerializeFn);
pub type ReceiveFn = fn(&mut World, u8);
pub type SerializeFn<T> = fn(&T, &EventContext) -> Bytes;
type UntypedSerializeFn = fn(&dyn Any, &EventContext) -> Bytes;

struct NetworkEventFns {
    channel_id: u8,
    send: SendFn,
    resend_locally: fn(&mut World),
    receive: ReceiveFn,
    reset: fn(&mut World),
    serialize_fn: UntypedSerializeFn,
}

pub struct EventContext<'a> {
    pub type_registry: &'a AppTypeRegistry,
    // pub current_tick: RepliconTick,
}
