//! API for messaging backends.
//!
//! We don't provide any traits to avoid Rust's "orphan rule". Instead, backends need to create channels defined in the
//! [`RepliconChannels`](replicon_channels::RepliconChannels) resource and then manage the
//! [`RepliconServer`](replicon_server::RepliconServer) and [`RepliconClient`](replicon_client::RepliconClient) resources,
//! along with the [`ConnectedClient`](connected_client::ConnectedClient) component. This way, integrations can be provided
//! as separate crates without requiring us or crate authors to maintain them under a feature. See the documentation on
//! types in this module for details.
//!
//! It's also recommended to split the crate into client and server plugins, along with `server` and `client` features.
//! This way, plugins can be conveniently disabled at compile time, which is useful for dedicated server or client
//! configurations.
//!
//! You can also use
//! [bevy_replicon_example_backend](https://github.com/projectharmonia/bevy_replicon/tree/master/bevy_replicon_example_backend)
//! as a reference. For a real backend integration, see [bevy_replicon_renet](https://github.com/projectharmonia/bevy_replicon_renet),
//! which we maintain.

pub mod connected_client;
pub mod replicon_channels;
pub mod replicon_client;
pub mod replicon_server;
