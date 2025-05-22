//! API for messaging backends.
//!
//! We don't provide any traits to avoid Rust's orphan rule. Instead, backends are expected to:
//!
//! - Create channels defined in the [`RepliconChannels`](replicon_channels::RepliconChannels) resource.
//!   This can be done via an extension trait that provides a conversion which the user needs to call manually to get channels for the backend.
//! - Update the [`RepliconServer`](replicon_server::RepliconServer) and [`RepliconClient`](replicon_client::RepliconClient) resources.
//! - Spawn and despawn entities with [`ConnectedClient`](connected_client::ConnectedClient) component.
//! - React on [`DisconnectRequest`] event.
//!
//! This way, integrations can be provided as separate crates without requiring us or crate authors to maintain them under a feature.
//! See the documentation on types in this module for details.
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

use bevy::prelude::*;

/// An event for the messaging backend to queue a disconnection
/// for a specific client on the server.
///
/// The disconnection should occur **after** all pending messages
/// for this client have been sent. The actual delivery of these
/// messages is not guaranteed.
#[derive(Event, Clone, Copy, Debug)]
pub struct DisconnectRequest {
    pub client_entity: Entity,
}
