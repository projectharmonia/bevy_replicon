# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![codecov](https://codecov.io/gh/lifescapegame/bevy_replicon/branch/master/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/lifescapegame/bevy_replicon)

Write the same logic that works for both multiplayer and single-player. The crate provides synchronization of components and network events between the server and clients using the [Renet](https://github.com/lucaspoffo/renet) library for the [Bevy game engine](https://bevyengine.org).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon/latest/bevy_replicon).

See also [examples](https://github.com/lifescapegame/bevy_replicon/tree/master/examples).

## Related Crates

- [bevy_timewarp](https://github.com/RJ/bevy_timewarp) - a rollback library that buffers component state. See [this](https://github.com/RJ/bevy_timewarp/blob/main/REPLICON_INTEGRATION.md) instruction about how to integrate.
- [bevy_replicon_snap](https://github.com/Bendzae/bevy_replicon_snap) - a snapshot interpolation plugin.

## Bevy compatibility

| bevy   | bevy_replicon |
|--------|---------------|
| 0.11.0 | 0.6-0.17      |
| 0.10.1 | 0.2-0.6       |
| 0.10.0 | 0.1           |
