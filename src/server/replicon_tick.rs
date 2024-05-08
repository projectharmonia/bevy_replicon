use std::cmp::Ordering;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Like to [`Tick`](bevy::ecs::component::Tick), but for replication.
///
/// Used only on server. The [`ServerPlugin`](super::ServerPlugin) sends replication data
/// in [`PostUpdate`] any time this resource changes.
/// By default, its incremented in [`PostUpdate`] per the [`TickPolicy`](super::TickPolicy).
///
/// If you set [`TickPolicy::Manual`](super::TickPolicy::Manual), you can increment [`RepliconTick`]
/// at the start of your game loop inside [`FixedMain`](bevy::app::FixedMain).
/// This value can represent your simulation step, and is made available to the client in the custom
/// deserialization, despawn and component removal functions.
///
/// One use for this is rollback networking: you may want to rollback time and apply the update
/// for the tick frame, which is in the past, then resimulate.
///
/// See [`ServerInitTick`](crate::client::ServerInitTick) for tick tracking the last received
/// tick on the client.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Resource, Serialize)]
pub struct RepliconTick(pub(crate) u32);

impl RepliconTick {
    /// Gets the value of this tick.
    #[inline]
    pub fn get(self) -> u32 {
        self.0
    }

    /// Increments current tick by the specified `value` and takes wrapping into account.
    #[inline]
    pub fn increment_by(&mut self, value: u32) {
        self.0 = self.0.wrapping_add(value);
    }

    /// Same as [`Self::increment_by`], but increments only by 1.
    #[inline]
    pub fn increment(&mut self) {
        self.increment_by(1)
    }
}

impl PartialOrd for RepliconTick {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let difference = self.0.wrapping_sub(other.0);
        if difference == 0 {
            Some(Ordering::Equal)
        } else if difference > u32::MAX / 2 {
            Some(Ordering::Less)
        } else {
            Some(Ordering::Greater)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_comparsion() {
        assert_eq!(RepliconTick(0), RepliconTick(0));
        assert!(RepliconTick(0) < RepliconTick(1));
        assert!(RepliconTick(0) > RepliconTick(u32::MAX));
    }
}
