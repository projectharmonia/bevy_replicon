use bevy::{prelude::*, reflect::FromReflect};
use serde::{Deserialize, Serialize};

/// Tick to communicate the game's timeline between client and server.
#[derive(
    Resource,
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Reflect,
    FromReflect,
)]
pub struct NetworkTick(u64);

impl Default for NetworkTick {
    fn default() -> Self {
        Self::new(0)
    }
}

impl NetworkTick {
    pub fn new(tick: u64) -> Self {
        Self(tick)
    }

    pub fn increment(&mut self) {
        self.0 += 1;
    }

    pub fn set_tick(&mut self, tick: u64) {
        self.0 = tick;
    }

    pub fn raw(&self) -> u64 {
        self.0
    }
}

pub struct NetworkTickMap(HashMap<NetworkTick, u32>);
