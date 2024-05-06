use std::fmt::{self, Debug, Formatter};

use bevy::prelude::*;

use crate::server::replicon_tick::RepliconTick;

/// Received tick from server for an entity.
#[derive(Component)]
pub struct Confirmed {
    mask: u64,
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

    /// Returns last received tick for an entity.
    pub fn last_tick(&self) -> RepliconTick {
        self.last_tick
    }

    /// Returns `true` if this tick is confirmed for an entity.
    pub fn get(&self, tick: RepliconTick) -> bool {
        if tick > self.last_tick {
            return false;
        }

        let ago = self.last_tick - tick;
        ago >= u64::BITS || (self.mask >> ago & 1) == 1
    }

    pub(super) fn set(&mut self, tick: RepliconTick) -> bool {
        let new = tick > self.last_tick;
        if new {
            self.resize_to(tick);
        }

        let ago = self.last_tick - tick;
        self.mask |= 1 << ago;

        new
    }

    fn resize_to(&mut self, tick: RepliconTick) {
        let diff = tick - self.last_tick;
        self.mask = self.mask.wrapping_shl(diff);
        self.last_tick = tick;
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
    fn set_previous() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.set(RepliconTick(0));

        assert!(confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(2)));
    }

    #[test]
    fn set_next() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.set(RepliconTick(2));

        assert!(!confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(confirmed.get(RepliconTick(2)));
    }

    #[test]
    fn set_with_resize() {
        let mut confirmed = Confirmed::new(RepliconTick(1));

        confirmed.set(RepliconTick(65));

        assert!(confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(2)));
        assert!(!confirmed.get(RepliconTick(64)));
        assert!(confirmed.get(RepliconTick(65)));
        assert!(!confirmed.get(RepliconTick(66)));
    }

    #[test]
    fn set_with_overflow() {
        let mut confirmed = Confirmed::new(RepliconTick(u32::MAX));

        confirmed.set(RepliconTick(1));

        assert!(!confirmed.get(RepliconTick(0)));
        assert!(confirmed.get(RepliconTick(1)));
        assert!(!confirmed.get(RepliconTick(3)));
        assert!(confirmed.get(RepliconTick(u32::MAX)));
    }
}
