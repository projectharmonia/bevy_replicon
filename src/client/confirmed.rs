use std::fmt::{self, Debug, Formatter};

use bevy::prelude::*;

use crate::server::replicon_tick::RepliconTick;

/// Received ticks from server for an entity.
///
/// For efficiency reason we store only last received tick and
/// a bitmask indicating whether the last 64 ticks were received.
#[derive(Component)]
pub struct Confirmed {
    /// Previously confirmed ticks, including the last tick at position 0.
    mask: u64,

    /// Last received tick from server for an entity.
    last_tick: RepliconTick,
}

impl Debug for Confirmed {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Confirmed [{:?} {:b}]", self.last_tick, self.mask)
    }
}

impl Confirmed {
    /// Creates a new instance with a single confirmed tick.
    pub fn new(last_tick: RepliconTick) -> Self {
        Self { mask: 1, last_tick }
    }

    /// Returns the last received tick for an entity.
    pub fn last_tick(&self) -> RepliconTick {
        self.last_tick
    }

    /// Returns `true` if this tick is confirmed for an entity.
    ///
    /// All ticks older then 64 ticks since [`Self::last_tick`] are considered received.
    pub fn contains(&self, tick: RepliconTick) -> bool {
        if tick > self.last_tick {
            return false;
        }

        let ago = self.last_tick - tick;
        ago >= u64::BITS || (self.mask >> ago & 1) == 1
    }

    /// Confirms a tick.
    ///
    /// Useful for unit tests.
    pub fn confirm(&mut self, tick: RepliconTick) {
        if tick > self.last_tick {
            self.resize_to(tick);
        }
        let ago = self.last_tick - tick;
        if ago < u64::BITS {
            self.set(ago);
        }
    }

    /// Marks previous tick as received.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and
    /// `ago` is bigger then [`u64::BITS`].
    pub(super) fn set(&mut self, ago: u32) {
        debug_assert!(ago < u64::BITS);
        self.mask |= 1 << ago;
    }

    /// Sets the last received tick and shifts the mask.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and
    /// `tick` is less then the last tick.
    pub(super) fn resize_to(&mut self, tick: RepliconTick) {
        debug_assert!(tick >= self.last_tick);
        let diff = tick - self.last_tick;
        self.mask = self.mask.wrapping_shl(diff);
        self.last_tick = tick;
        self.mask |= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains() {
        let confirmed = Confirmed::new(RepliconTick(1));

        assert!(!confirmed.contains(RepliconTick(0)));
        assert!(confirmed.contains(RepliconTick(1)));
        assert!(!confirmed.contains(RepliconTick(2)));
        assert!(!confirmed.contains(RepliconTick(u32::MAX)));
    }

    #[test]
    fn set() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.set(1);

        assert!(confirmed.contains(RepliconTick(0)));
        assert!(confirmed.contains(RepliconTick(1)));
        assert!(!confirmed.contains(RepliconTick(2)));
    }

    #[test]
    fn resize() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.resize_to(RepliconTick(2));

        assert!(!confirmed.contains(RepliconTick(0)));
        assert!(confirmed.contains(RepliconTick(1)));
        assert!(confirmed.contains(RepliconTick(2)));
    }

    #[test]
    fn resize_to_same() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.resize_to(RepliconTick(1));

        assert!(!confirmed.contains(RepliconTick(0)));
        assert!(confirmed.contains(RepliconTick(1)));
        assert!(!confirmed.contains(RepliconTick(2)));
    }

    #[test]
    fn resize_with_wrapping() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.resize_to(RepliconTick(65));

        assert!(confirmed.contains(RepliconTick(0)));
        assert!(confirmed.contains(RepliconTick(1)));
        assert!(!confirmed.contains(RepliconTick(2)));
        assert!(!confirmed.contains(RepliconTick(64)));
        assert!(confirmed.contains(RepliconTick(65)));
        assert!(!confirmed.contains(RepliconTick(66)));
    }

    #[test]
    fn resize_with_overflow() {
        let mut confirmed = Confirmed::new(RepliconTick(u32::MAX));

        confirmed.resize_to(RepliconTick(1));

        assert!(!confirmed.contains(RepliconTick(0)));
        assert!(confirmed.contains(RepliconTick(1)));
        assert!(!confirmed.contains(RepliconTick(3)));
        assert!(confirmed.contains(RepliconTick(u32::MAX)));
    }
}
