pub mod client_event;
pub mod server_event;

use std::{marker::PhantomData, time::Duration};

use bevy::{ecs::entity::EntityHashMap, prelude::*};
use bevy_renet::renet::SendType;

#[allow(deprecated)]
use crate::replicon_core::replication_rules::Mapper;

/// Holds a server's channel ID for `T`.
#[derive(Resource)]
pub struct ServerEventChannel<T> {
    id: u8,
    marker: PhantomData<T>,
}

impl<T> ServerEventChannel<T> {
    fn new(id: u8) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
}

impl<T> Clone for ServerEventChannel<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ServerEventChannel<T> {}

impl<T> From<ServerEventChannel<T>> for u8 {
    fn from(value: ServerEventChannel<T>) -> Self {
        value.id
    }
}

/// Holds a client's channel ID for `T`.
#[derive(Resource)]
pub struct ClientEventChannel<T> {
    id: u8,
    marker: PhantomData<T>,
}

impl<T> ClientEventChannel<T> {
    fn new(id: u8) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
}

impl<T> Clone for ClientEventChannel<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ClientEventChannel<T> {}

impl<T> From<ClientEventChannel<T>> for u8 {
    fn from(value: ClientEventChannel<T>) -> Self {
        value.id
    }
}

/// Event delivery guarantee.
///
/// Mirrors [`SendType`] and can be converted into it with `resend_time` set to 300ms for reliable types.
/// Provided for convenient defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventType {
    /// Unreliable and unordered.
    Unreliable,
    /// Reliable and unordered.
    Unordered,
    /// Reliable and ordered.
    Ordered,
}

impl From<EventType> for SendType {
    fn from(event_type: EventType) -> Self {
        const RESEND_TIME: Duration = Duration::from_millis(300);
        match event_type {
            EventType::Unreliable => SendType::Unreliable,
            EventType::Unordered => SendType::ReliableUnordered {
                resend_time: RESEND_TIME,
            },
            EventType::Ordered => SendType::ReliableOrdered {
                resend_time: RESEND_TIME,
            },
        }
    }
}

/// Maps server entities into client entities inside events.
///
/// Panics if a mapping doesn't exists.
pub struct EventMapper<'a>(pub &'a EntityHashMap<Entity>);

#[allow(deprecated)]
impl Mapper for EventMapper<'_> {
    fn map(&mut self, entity: Entity) -> Entity {
        *self
            .0
            .get(&entity)
            .unwrap_or_else(|| panic!("entity {entity:?} should be mappable"))
    }
}

impl EntityMapper for EventMapper<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self
            .0
            .get(&entity)
            .unwrap_or_else(|| panic!("entity {entity:?} should be mappable"))
    }
}
