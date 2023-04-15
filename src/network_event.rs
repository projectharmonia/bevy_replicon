pub mod client_event;
pub mod server_event;

use std::marker::PhantomData;

use bevy::prelude::*;

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
