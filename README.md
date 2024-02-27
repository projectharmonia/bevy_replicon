# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![codecov](https://codecov.io/gh/projectharmonia/bevy_replicon/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/projectharmonia/bevy_replicon)

ECS-focused high-level networking crate for the [Bevy game engine](https://bevyengine.org).

## Features

- Automatic component-oriented world state replication.
- Events-based messaging API with different guarantees (reliable, reliable unordered, and unreliable).
- Control over client visibility of entities and events.
- Replication into scene to save server state.
- Support for client and server both in one `App` and in separate.
- Customizable serialization and deserialization even for types that don't implement `serde` traits (like `Box<dyn Reflect>`).
- No builtin I/O. Use it with any messaging library (or multiple libraries simultaneously). We provide a first-party integration with [`renet`](https://github.com/lucaspoffo/renet) via `bevy_replicon_renet`.
- API focused on writing logic once that automatically works for singleplayer, client, server, and listen server (when server is also a player).

Prediction and interpolation are not implemented in this crate and are considered out of scope. But the idea of the crate is to provide an extensible core, so if your game needs something, you can implement it on top. Also check out [related crates](#Related-crates).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon/latest/bevy_replicon).

See also [examples](https://github.com/projectharmonia/bevy_replicon/tree/master/bevy_replicon_renet/examples).

## Related Crates

- [bevy_timewarp](https://github.com/RJ/bevy_timewarp) - a rollback library that buffers component state. See [this](https://github.com/RJ/bevy_timewarp/blob/main/REPLICON_INTEGRATION.md) instruction about how to integrate.
- [bevy_replicon_snap](https://github.com/Bendzae/bevy_replicon_snap) - a snapshot interpolation plugin.
- [bevy_replicon_attributes](https://github.com/UkoeHB/bevy_replicon_attributes) - adds ergonomic visibility control through client attributes and entity/event visibility conditions. An extension of this crate's raw client visibility API.
- [bevy_replicon_repair](https://github.com/UkoeHB/bevy_replicon_repair) - preserves replicated client state across reconnects.

## Bevy compatibility

| bevy   | bevy_replicon | bevy_replicon_renet |
|--------|---------------| ------------------- |
| 0.13.0 | 0.23          | 0.1                 |
| 0.12.1 | 0.18-0.22     |                     |
| 0.11.0 | 0.6-0.17      |                     |
| 0.10.1 | 0.2-0.6       |                     |
| 0.10.0 | 0.1           |                     |
