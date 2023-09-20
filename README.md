# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![codecov](https://codecov.io/gh/lifescapegame/bevy_replicon/branch/master/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/lifescapegame/bevy_replicon)

Write the same logic that works for both multiplayer and single-player. The crate provides synchronization of components and network events between the server and clients using the [Renet](https://github.com/lucaspoffo/renet) library for the [Bevy game engine](https://bevyengine.org).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon/latest/bevy_replicon).

See also [Tic-Tac-Toe](https://github.com/lifescapegame/bevy_replicon/tree/master/examples/tic_tac_toe.rs) example.


## Bevy compatibility

| bevy   | bevy_replicon |
|--------|---------------|
| 0.11.0 | 0.6-0.10      |
| 0.10.1 | 0.2-0.6       |
| 0.10.0 | 0.1           |
