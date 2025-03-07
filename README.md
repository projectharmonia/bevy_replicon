# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![license](https://img.shields.io/crates/l/bevy_replicon)](#license)
[![codecov](https://codecov.io/gh/projectharmonia/bevy_replicon/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/projectharmonia/bevy_replicon)

Server-authoritative networking crate for the [Bevy game engine](https://bevyengine.org).

## Features

- Automatic world replication.
- Remote events and triggers.
- Control over client visibility of entities and events.
- Abstracts game logic to support singleplayer, client, dedicated server, and listen server configurations simultaneously.
- No builtin I/O, can be used with any messaging library. See [messaging backends](#messaging-backends) for already available integrations.
- Replication into scene to save server state.
- Customizable serialization and deserialization even for types that don't implement `serde` traits (like `Box<dyn Reflect>`).
- Extensible architecture. See [related crates](#related-crates).

If you are new to networking, see [glossary](https://gist.github.com/maniwani/f92cc5d827b00163f5846ea7dcb90d44).

## Goals

The purpose of the crate is to provide a minimal and fast core that can be extended with the necessary features to ensure smooth gameplay. Consider the following examples:

- A slow paced centrally hosted game wants ECS-level replication, and maybe some interpolation on top.
- A slightly faster paced game might care more about order and need a lockstep system.
- A shooter needs client prediction for the player and interpolation for everything else.
- A sports game, or an online game featuring mechanics more complex than most shooters, needs ECS-level replication with full rollback on the entire world.
- A fighting game only needs to replicate some input events and needs rollback on top.

All of these examples also have drastically different optimization requirements. This is why modularity is essential. It also allows for more developers to be involved and for each to maintain what they use.

Check out [related crates](#related-crates) to extend the core functionality.

See also [What kind of networking should X game use?](https://github.com/bevyengine/bevy/discussions/8675).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon).

For examples navigate to the [`bevy_replicon_example_backend`](bevy_replicon_example_backend) (because you need I/O in order to run them).

Have any questions? Feel free to ask in the dedicated [`bevy_replicon` channel](https://discord.com/channels/691052431525675048/1090432346907492443) in Bevy's Discord server.

## Related Crates

> [!WARNING]
> Ensure that your `bevy_replicon` version is compatible with the used crate according to semantic versioning.

#### Messaging backends

- [`bevy_replicon_renet`](https://github.com/projectharmonia/bevy_replicon_renet) - integration for [`bevy_renet`](https://github.com/lucaspoffo/renet/tree/master/bevy_renet). Maintained by the authors of this crate.
- [`bevy_replicon_renet2`](https://github.com/UkoeHB/renet2/tree/main/bevy_replicon_renet2) - integration for [`bevy_renet2`](https://github.com/UkoeHB/renet2/tree/main/bevy_renet2). Includes a WebTransport backend for browsers, and enables servers that can manage multi-platform clients simultaneously.
- [`bevy_replicon_quinnet`](https://github.com/Henauxg/bevy_replicon_quinnet) - integration for [`bevy_quinnet`](https://github.com/Henauxg/bevy_quinnet).
- [`aeronet_replicon`](https://github.com/aecsocket/aeronet/tree/main/crates/aeronet_replicon) - integration for [`aeronet`](https://github.com/aecsocket/aeronet). Works on any IO layer supported by `aeronet_io`, but requires `aeronet_transport`.

#### Helpers

- [`bevy_bundlication`](https://github.com/NiseVoid/bevy_bundlication) - adds registration of replication groups using a bundle-like api.
- [`bevy_replicon_attributes`](https://github.com/UkoeHB/bevy_replicon_attributes) - adds ergonomic visibility control through client attributes and entity/event visibility conditions. An extension of this crate's raw client visibility API.

#### Interpolation and/or rollback

- [`bevy_replicon_snap`](https://github.com/Bendzae/bevy_replicon_snap) - adds snapshot interpolation and client-side prediction.

#### Miscellaneous

- [`bevy_replicon_repair`](https://github.com/UkoeHB/bevy_replicon_repair) - preserves replicated client state across reconnects.

#### Unmaintained

- [`bevy_timewarp`](https://github.com/RJ/bevy_timewarp) - a rollback library that buffers component state. See [this](https://github.com/RJ/bevy_timewarp/blob/main/REPLICON_INTEGRATION.md) instruction about how to integrate.

## Bevy compatibility

| bevy   | bevy_replicon |
| ------ | ------------- |
| 0.15.0 | 0.29-0.30     |
| 0.14.0 | 0.27-0.28     |
| 0.13.0 | 0.23-0.26     |
| 0.12.1 | 0.18-0.22     |
| 0.11.0 | 0.6-0.17      |
| 0.10.1 | 0.2-0.6       |
| 0.10.0 | 0.1           |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
