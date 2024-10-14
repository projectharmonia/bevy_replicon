use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::replicon_tick::RepliconTick;

/// Stores current [`RepliconTick`].
///
/// Used only on the server. The [`ServerPlugin`](super::ServerPlugin) sends replication data
/// in [`PostUpdate`] any time this resource changes.
/// By default, its incremented in [`PostUpdate`] per the [`TickPolicy`](super::TickPolicy).
///
/// If you set [`TickPolicy::Manual`](super::TickPolicy::Manual), you can increment this resource
/// at the start of your game loop (e.g. inside [`FixedMain`](bevy::app::FixedMain)).
/// This value can be used to represent your simulation step, and is made available to the client in
/// the custom deserialization, despawn, and component removal functions.
///
/// See [`ServerInitTick`](crate::client::ServerInitTick) for tracking the last received
/// tick on clients.
#[derive(Clone, Copy, Deref, Debug, Default, Deserialize, Resource, Serialize)]
pub struct ServerTick(RepliconTick);

impl ServerTick {
    /// Increments current tick by the specified `value` and takes wrapping into account.
    #[inline]
    pub fn increment_by(&mut self, value: u32) {
        self.0 += value;
    }

    /// Same as [`Self::increment_by`], but increments only by 1.
    #[inline]
    pub fn increment(&mut self) {
        self.increment_by(1)
    }
}
