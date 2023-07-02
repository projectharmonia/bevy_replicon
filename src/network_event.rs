pub mod client_event;
pub mod server_event;
#[cfg(test)]
mod test_events;

use std::{marker::PhantomData, time::Duration};

use bevy::{prelude::*, reflect::TypeRegistryInternal};
use bevy_renet::renet::SendType;

/// Holds a channel ID for `T`.
#[derive(Resource)]
pub struct EventChannel<T> {
    pub id: u8,
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

/// Creates a struct implements serialization for the event using [`TypeRegistryInternal`].
pub trait BuildEventSerializer<T> {
    type EventSerializer<'a>
    where
        T: 'a;

    fn new<'a>(event: &'a T, registry: &'a TypeRegistryInternal) -> Self::EventSerializer<'a>;
}

/// Creates a struct implements deserialization for the event using [`TypeRegistryInternal`].
pub trait BuildEventDeserializer {
    type EventDeserializer<'a>;

    fn new(registry: &TypeRegistryInternal) -> Self::EventDeserializer<'_>;
}

/// Event delivery guarantee.
#[derive(Clone, Copy)]
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
