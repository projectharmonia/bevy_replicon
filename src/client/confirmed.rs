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
    pub(super) fn new(last_tick: RepliconTick) -> Self {
        Self { mask: 1, last_tick }
    }

    /// Returns the last received tick for an entity.
    pub fn last_tick(&self) -> RepliconTick {
        self.last_tick
    }

    /// Returns `true` if this tick is confirmed for an entity.
    ///
    /// All ticks older then 64 ticks since [`Self::last_tick`] are considered received.
    pub fn get(&self, tick: RepliconTick) -> bool {
        if tick > self.last_tick {
            return false;
        }

        let ago = self.last_tick - tick;
        ago >= u64::BITS || (self.mask >> ago & 1) == 1
    }

    /// Marks specific tick as received.
    pub(super) fn set(&mut self, ago: u32) {
        debug_assert!(ago < u64::BITS);
        self.mask |= 1 << ago;
    }

    /// Sets the last received tick and shifts the mask.
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
    fn get() {
        let confirmed = Confirmed::new(RepliconTick(1));

        assert!(!confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(2)));
        assert!(!confirmed.get(RepliconTick(u32::MAX)));
    }

    #[test]
    fn set() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.set(1);

        assert!(confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(2)));
    }

    #[test]
    fn resize() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.resize_to(RepliconTick(2));

        assert!(!confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(confirmed.get(RepliconTick(2)));
    }

    #[test]
    fn resize_with_wrapping() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.resize_to(RepliconTick(65));

        assert!(confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(2)));
        assert!(!confirmed.get(RepliconTick(64)));
        assert!(confirmed.get(RepliconTick(65)));
        assert!(!confirmed.get(RepliconTick(66)));
    }

    #[test]
    fn resize_with_overflow() {
        let mut confirmed = Confirmed::new(RepliconTick(u32::MAX));

        confirmed.resize_to(RepliconTick(1));

        assert!(!confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(3)));
        assert!(confirmed.get(RepliconTick(u32::MAX)));
    }
}
