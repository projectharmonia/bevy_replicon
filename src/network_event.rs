pub mod client_event;
pub mod server_event;

use std::any::Any;

use bevy::{
    ecs::entity::EntityHashMap,
    prelude::*,
    reflect::{erased_serde::Serialize, TypeRegistry},
};
use bytes::Bytes;
use serde::de::DeserializeOwned;

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

type SendFn = fn(&mut World, &NetworkEventFns);
type ReceiveFn = fn(&mut World, &NetworkEventFns);
pub type SerializeFn<T: Serialize> = fn(&T, &EventContext) -> bincode::Result<Bytes>;
pub type DeserializeFn<T: DeserializeOwned> = fn(Bytes, &EventContext) -> bincode::Result<T>;

struct NetworkEventFns {
    channel_id: u8,
    send: SendFn,
    resend_locally: fn(&mut World),
    receive: ReceiveFn,
    reset: fn(&mut World),
    serialize: fn(),
    deserialize: fn(),
}

impl NetworkEventFns {
    fn new<T: Serialize + DeserializeOwned>(
        channel_id: u8,
        send: SendFn,
        resend_locally: fn(&mut World),
        receive: ReceiveFn,
        reset: fn(&mut World),
        serialize: SerializeFn<T>,
        deserialize: DeserializeFn<T>,
    ) -> Self {
        Self {
            channel_id,
            send,
            resend_locally,
            receive,
            reset,
            serialize: unsafe { std::mem::transmute::<SerializeFn<T>, fn()>(serialize) },
            deserialize: unsafe { std::mem::transmute::<DeserializeFn<T>, fn()>(deserialize) },
        }
    }
    unsafe fn typed_serialize<T: Serialize>(&self) -> SerializeFn<T> {
        unsafe { std::mem::transmute(self.serialize) }
    }

    unsafe fn typed_deserialize<T: DeserializeOwned>(&self) -> DeserializeFn<T> {
        unsafe { std::mem::transmute(self.deserialize) }
    }
}

pub struct EventContext<'a> {
    pub type_registry: &'a AppTypeRegistry,
    // pub current_tick: RepliconTick,
}
