pub mod client_event;
pub mod server_event;

use std::{marker::PhantomData, time::Duration};

use bevy::{prelude::*, utils::HashMap};
use bevy_renet::renet::SendType;

use crate::replicon_core::replication_rules::Mapper;

/// Holds a channel ID for `T`.
#[derive(Resource)]
pub struct EventChannel<T> {
    id: u8,
    marker: PhantomData<T>,
}

impl<T> EventChannel<T> {
    fn new(id: u8) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
}

impl<T> Clone for EventChannel<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for EventChannel<T> {}

impl<T> From<EventChannel<T>> for u8 {
    fn from(value: EventChannel<T>) -> Self {
        value.id
    }
}

/// Event delivery guarantee.
#[derive(Clone, Copy, Debug)]
pub enum SendPolicy {
    /// Unreliable and unordered
    Unreliable,
    /// Reliable and unordered
    Unordered,
    /// Reliable and ordered
    Ordered,
}

impl From<SendPolicy> for SendType {
    fn from(policy: SendPolicy) -> Self {
        const RESEND_TIME: Duration = Duration::from_millis(300);
        match policy {
            SendPolicy::Unreliable => SendType::Unreliable,
            SendPolicy::Unordered => SendType::ReliableUnordered {
                resend_time: RESEND_TIME,
            },
            SendPolicy::Ordered => SendType::ReliableOrdered {
                resend_time: RESEND_TIME,
            },
        }
    }
}

/// Maps server entities into client entities inside events.
///
/// Panics if a mapping doesn't exists.
pub struct EventMapper<'a>(pub &'a HashMap<Entity, Entity>);

impl Mapper for EventMapper<'_> {
    fn map(&mut self, entity: Entity) -> Entity {
        *self
            .0
            .get(&entity)
            .unwrap_or_else(|| panic!("entity {entity:?} should be mappable"))
    }
}
