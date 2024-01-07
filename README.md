# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![codecov](https://codecov.io/gh/lifescapegame/bevy_replicon/branch/master/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/lifescapegame/bevy_replicon)

ECS-focused high-level networking crate for the [Bevy game engine](https://bevyengine.org) using the [Renet](https://github.com/lucaspoffo/renet) library.

The crate provides component-oriented world state replication and exposes an events-based messaging API.

Prediction and interpolation are not implemented in this crate, but the crate API is designed to be extensible so if your game needs something, you can implement it on top. Also check out [related crates](#Related-crates).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon/latest/bevy_replicon).

See also [examples](https://github.com/lifescapegame/bevy_replicon/tree/master/examples).

## Related Crates

- [bevy_timewarp](https://github.com/RJ/bevy_timewarp) - a rollback library that buffers component state. See [this](https://github.com/RJ/bevy_timewarp/blob/main/REPLICON_INTEGRATION.md) instruction about how to integrate.
- [bevy_replicon_snap](https://github.com/Bendzae/bevy_replicon_snap) - a snapshot interpolation plugin.

## Bevy compatibility

| bevy   | bevy_replicon |
|--------|---------------|
| 0.11.1 | 0.18-0.19     |
| 0.11.0 | 0.6-0.17      |
| 0.10.1 | 0.2-0.6       |
| 0.10.0 | 0.1           |
