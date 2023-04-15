pub mod client_event;
pub mod server_event;
#[cfg(test)]
mod test_events;

use std::marker::PhantomData;

use bevy::{prelude::*, reflect::TypeRegistryInternal};

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

    fn new<'a>(registry: &'a TypeRegistryInternal, event: &'a T) -> Self::EventSerializer<'a>;
}

/// Creates a struct implements deserialization for the event using [`TypeRegistryInternal`].
pub trait BuildEventDeserializer {
    type EventDeserializer<'a>;

    fn new(registry: &TypeRegistryInternal) -> Self::EventDeserializer<'_>;
}
