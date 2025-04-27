//! System conditions for [`RepliconClient`] and [`RepliconServer`] resources.

use bevy::prelude::*;

use super::backend::{replicon_client::RepliconClient, replicon_server::RepliconServer};

/// Returns `true` if the server is running.
pub fn server_running(server: Option<Res<RepliconServer>>) -> bool {
    server.is_some_and(|server| server.is_running())
}

/// Returns `true` if there is no client or if the existing client is disconnected.
///
/// Can be used instead of the regular [`server_running`] to seamlessly support
/// singleplayer or listen-server mode (where server is also a player).
pub fn server_or_singleplayer(client: Option<Res<RepliconClient>>) -> bool {
    client.is_none_or(|client| client.is_disconnected())
}

/// Returns `true` when the client is connecting.
pub fn client_connecting(client: Option<Res<RepliconClient>>) -> bool {
    client.is_some_and(|client| client.is_connecting())
}

/// Returns `true` when the client is connected.
pub fn client_connected(client: Option<Res<RepliconClient>>) -> bool {
    client.is_some_and(|client| client.is_connected())
}

/// Returns `true` if the server stopped on this tick.
pub fn server_just_stopped(
    mut last_running: Local<bool>,
    server: Option<Res<RepliconServer>>,
) -> bool {
    let running = server.is_some_and(|server| server.is_running());

    let just_stopped = *last_running && !running;
    *last_running = running;
    just_stopped
}

/// Returns `true` if the server started on this tick.
pub fn server_just_started(
    mut last_running: Local<bool>,
    server: Option<Res<RepliconServer>>,
) -> bool {
    let running = server.is_some_and(|server| server.is_running());

    let just_started = !*last_running && running;
    *last_running = running;
    just_started
}

/// Returns `true` when the client just started connecting on this tick.
pub fn client_started_connecting(
    mut last_connecting: Local<bool>,
    client: Option<Res<RepliconClient>>,
) -> bool {
    let connecting = client.is_some_and(|client| client.is_connecting());

    let started_connecting = !*last_connecting && connecting;
    *last_connecting = connecting;
    started_connecting
}

/// Returns `true` when the client is connected on this tick.
pub fn client_just_connected(
    mut last_connected: Local<bool>,
    client: Option<Res<RepliconClient>>,
) -> bool {
    let connected = client.is_some_and(|client| client.is_connected());

    let just_connected = !*last_connected && connected;
    *last_connected = connected;
    just_connected
}

/// Returns `true` when the client is disconnected on this tick.
pub fn client_just_disconnected(
    mut last_not_disconnected: Local<bool>,
    client: Option<Res<RepliconClient>>,
) -> bool {
    let disconnected = client.is_some_and(|client| client.is_disconnected());

    let just_disconnected = *last_not_disconnected && disconnected;
    *last_not_disconnected = !disconnected;
    just_disconnected
}
