use std::fmt::{self, Debug, Formatter};

use bevy::prelude::*;

use crate::core::replicon_tick::RepliconTick;

/// Received ticks from the server for an entity.
///
/// For efficiency we store only the last received tick and
/// a bitmask indicating whether the most recent 64 ticks were received.
#[derive(Component)]
pub struct Confirmed {
    /// Previously confirmed ticks, including the last tick at position 0.
    mask: u64,

    /// The last received server tick for an entity.
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

        let ago = self.last_tick.get().wrapping_sub(tick.get());
        ago >= u64::BITS || (self.mask >> ago & 1) == 1
    }

    /// Returns `true` if any tick in the given range was confirmed for the entity with
    /// this component.
    ///
    /// All ticks older then 64 ticks since [`Self::last_tick`] are considered received.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and
    /// `start_tick` is greater then `end_tick`.
    pub fn contains_any(&self, start_tick: RepliconTick, end_tick: RepliconTick) -> bool {
        debug_assert!(start_tick <= end_tick);

        let end_tick = if end_tick > self.last_tick {
            self.last_tick
        } else {
            end_tick
        };

        if start_tick > end_tick {
            return false;
        }

        let ago = end_tick.get().wrapping_sub(start_tick.get());
        let range = (1 << (ago + 1)) - 1; // Set bit to `ago + 1` and then decrement to get `ago` of 1's.
        let offset = self.last_tick.get().wrapping_sub(end_tick.get());
        let mask = range << offset;

        self.mask & mask != 0
    }

    /// Confirms a tick.
    ///
    /// Useful for unit tests.
    pub fn confirm(&mut self, tick: RepliconTick) {
        if tick > self.last_tick {
            self.set_last_tick(tick);
        }
        let ago = self.last_tick.get().wrapping_sub(tick.get());
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
    pub(super) fn set_last_tick(&mut self, tick: RepliconTick) {
        debug_assert!(tick >= self.last_tick);
        let diff = tick.get().wrapping_sub(self.last_tick.get());
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
        let confirmed = Confirmed::new(RepliconTick::new(1));

        assert!(!confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(!confirmed.contains(RepliconTick::new(2)));
        assert!(!confirmed.contains(RepliconTick::new(u32::MAX)));
    }

    #[test]
    fn contains_any() {
        let confirmed = Confirmed::new(RepliconTick::new(1));

        assert!(confirmed.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(confirmed.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!confirmed.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(!confirmed.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(0)));
        assert!(confirmed.contains_any(RepliconTick::new(0), RepliconTick::new(2)));
        assert!(confirmed.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(3)));
    }

    #[test]
    fn contains_any_with_set() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.set(2);

        assert!(confirmed.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(confirmed.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!confirmed.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(confirmed.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(0)));
        assert!(confirmed.contains_any(RepliconTick::new(0), RepliconTick::new(2)));
        assert!(confirmed.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(3)));
        assert!(
            confirmed.contains_any(RepliconTick::new(u32::MAX - 1), RepliconTick::new(u32::MAX))
        );
        assert!(!confirmed.contains_any(
            RepliconTick::new(u32::MAX - 2),
            RepliconTick::new(u32::MAX - 1)
        ));
    }

    #[test]
    fn set() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.set(1);

        assert!(confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(!confirmed.contains(RepliconTick::new(2)));
    }

    #[test]
    fn resize() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.set_last_tick(RepliconTick::new(2));

        assert!(!confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(confirmed.contains(RepliconTick::new(2)));
    }

    #[test]
    fn resize_to_same() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.set_last_tick(RepliconTick::new(1));

        assert!(!confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(!confirmed.contains(RepliconTick::new(2)));
    }

    #[test]
    fn resize_with_wrapping() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.set_last_tick(RepliconTick::new(65));

        assert!(confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(!confirmed.contains(RepliconTick::new(2)));
        assert!(!confirmed.contains(RepliconTick::new(64)));
        assert!(confirmed.contains(RepliconTick::new(65)));
        assert!(!confirmed.contains(RepliconTick::new(66)));
    }

    #[test]
    fn resize_with_overflow() {
        let mut confirmed = Confirmed::new(RepliconTick::new(u32::MAX));

        confirmed.set_last_tick(RepliconTick::new(1));

        assert!(!confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(!confirmed.contains(RepliconTick::new(3)));
        assert!(confirmed.contains(RepliconTick::new(u32::MAX)));
    }

    #[test]
    fn confirm_with_resize() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.confirm(RepliconTick::new(2));

        assert!(!confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(confirmed.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_with_set() {
        let mut confirmed = Confirmed::new(RepliconTick::new(1));

        confirmed.confirm(RepliconTick::new(0));

        assert!(confirmed.contains(RepliconTick::new(0)));
        assert!(confirmed.contains(RepliconTick::new(1)));
        assert!(!confirmed.contains(RepliconTick::new(2)));
    }
}
