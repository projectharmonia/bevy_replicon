//! Provides integration for [renet](https://github.com/lucaspoffo/renet) with Bevy.
//! If you implement an integration for your own messaging backend and already have
//! an integration with Bevy, you don't have to do the same.
//!
//! We decided to not depend on [`bevy_renet`](https://github.com/lucaspoffo/renet/tree/master/bevy_renet)
//! directly only because the author is not very active and the crate is sometimes behind a Bevy version.

#[cfg(feature = "renet_transport")]
pub mod transport;
pub mod wrappers;

use bevy::prelude::*;
use wrappers::{RenetClient, RenetServer, ServerEvent};

/// This system set is where all transports receive messages
///
/// If you want to ensure data has arrived in the [`RenetClient`] or [`RenetServer`], then schedule your
/// system after this set.
///
/// This system set runs in PreUpdate.
#[derive(Debug, SystemSet, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenetReceive;

/// This system set is where all transports send messages
///
/// If you want to ensure your packets have been registered by the [`RenetClient`] or [`RenetServer`], then
/// schedule your system before this set.
///
/// This system set runs in PostUpdate.
#[derive(Debug, SystemSet, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenetSend;

pub struct RenetServerPlugin;

pub struct RenetClientPlugin;

impl Plugin for RenetServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Events<ServerEvent>>();
        app.add_systems(
            PreUpdate,
            Self::update_system.run_if(resource_exists::<RenetServer>),
        );
        app.add_systems(
            PreUpdate,
            Self::emit_server_events_system
                .in_set(RenetReceive)
                .run_if(resource_exists::<RenetServer>)
                .after(Self::update_system),
        );
    }
}

impl RenetServerPlugin {
    pub fn update_system(mut server: ResMut<RenetServer>, time: Res<Time>) {
        server.update(time.delta());
    }

    pub fn emit_server_events_system(
        mut server: ResMut<RenetServer>,
        mut server_events: EventWriter<ServerEvent>,
    ) {
        while let Some(event) = server.get_event() {
            server_events.send(event.into());
        }
    }
}

impl Plugin for RenetClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            // Note: This runs before `RenetReceive`. See `NetcodeClientPlugin`.
            Self::update_system.run_if(resource_exists::<RenetClient>),
        );
    }
}

impl RenetClientPlugin {
    pub fn update_system(mut client: ResMut<RenetClient>, time: Res<Time>) {
        client.update(time.delta());
    }
}

pub fn client_connected(client: Option<Res<RenetClient>>) -> bool {
    match client {
        Some(client) => client.is_connected(),
        None => false,
    }
}

pub fn client_disconnected(client: Option<Res<RenetClient>>) -> bool {
    match client {
        Some(client) => client.is_disconnected(),
        None => true,
    }
}

pub fn client_connecting(client: Option<Res<RenetClient>>) -> bool {
    match client {
        Some(client) => client.is_connecting(),
        None => false,
    }
}

pub fn client_just_connected(
    mut last_connected: Local<bool>,
    client: Option<Res<RenetClient>>,
) -> bool {
    let connected = client.map(|client| client.is_connected()).unwrap_or(false);

    let just_connected = !*last_connected && connected;
    *last_connected = connected;
    just_connected
}

pub fn client_just_disconnected(
    mut last_connected: Local<bool>,
    client: Option<Res<RenetClient>>,
) -> bool {
    let disconnected = client
        .map(|client| client.is_disconnected())
        .unwrap_or(true);

    let just_disconnected = *last_connected && disconnected;
    *last_connected = !disconnected;
    just_disconnected
}
