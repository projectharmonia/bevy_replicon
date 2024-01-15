# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![codecov](https://codecov.io/gh/lifescapegame/bevy_replicon/branch/master/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/lifescapegame/bevy_replicon)

ECS-focused high-level networking crate for the [Bevy game engine](https://bevyengine.org) using the [Renet](https://github.com/lucaspoffo/renet) library.

## Features

- Authentication and encryption, using [`renetcode`](https://github.com/lucaspoffo/renet/tree/master/renetcode).
- Packet fragmentation and reassembly.
- Support for client and server both in one `App` and in separate.
- Customizable transport layer. Right now only Netcode is supported, but Steam, WebTransport, and memory channels are on the way (new Renet release is needed).
- Component-oriented world state replication.
- Events-based messaging API with different guarantees (reliable, reliable unordered, and unreliable).
- Control over client visibility of entities and events.
- Replication into scene to save server state.
- API focused on writing logic once that automatically works for singleplayer, client, server, and listen server (when server is also a player).

Prediction and interpolation are not implemented in this crate and are considered out of scope. But the idea of the crate is to provide an extensible core, so if your game needs something, you can implement it on top. Also check out [related crates](#Related-crates).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon/latest/bevy_replicon).

See also [examples](https://github.com/lifescapegame/bevy_replicon/tree/master/examples).

## Related Crates

- [bevy_timewarp](https://github.com/RJ/bevy_timewarp) - a rollback library that buffers component state. See [this](https://github.com/RJ/bevy_timewarp/blob/main/REPLICON_INTEGRATION.md) instruction about how to integrate.
- [bevy_replicon_snap](https://github.com/Bendzae/bevy_replicon_snap) - a snapshot interpolation plugin.
- [bevy_replicon_repair](https://github.com/UkoeHB/bevy_replicon_repair) - preserves replicated client state across reconnects.

## Bevy compatibility

| bevy   | bevy_replicon |
|--------|---------------|
| 0.12.1 | 0.18-0.20     |
| 0.11.0 | 0.6-0.17      |
| 0.10.1 | 0.2-0.6       |
| 0.10.0 | 0.1           |
