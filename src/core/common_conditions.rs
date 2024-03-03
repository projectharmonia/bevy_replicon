use bevy::prelude::*;

use crate::{client::replicon_client::RepliconClient, server::replicon_server::RepliconServer};

/// Returns `true` if the server is running.
pub fn server_running(server: Option<Res<RepliconServer>>) -> bool {
    server.filter(|server| server.is_running()).is_some()
}

/// Returns `true` if there is no client or if the existing client is disconnected.
///
/// Can be used for systems that run both on the server and in singleplayer mode.
pub fn has_authority(client: Option<Res<RepliconClient>>) -> bool {
    let Some(client) = client else {
        return true;
    };
    client.is_disconnected()
}

/// Returns `true` when the client is connecting.
pub fn connecting(client: Option<Res<RepliconClient>>) -> bool {
    client.filter(|client| client.is_connecting()).is_some()
}

/// Returns `true` when the client is connected.
pub fn connected(client: Option<Res<RepliconClient>>) -> bool {
    client.filter(|client| client.is_connected()).is_some()
}

/// Returns `true` if the server stopped on this tick.
pub fn server_just_stopped(
    mut last_running: Local<bool>,
    server: Option<Res<RepliconServer>>,
) -> bool {
    let running = server.filter(|server| server.is_running()).is_some();

    let just_stopped = *last_running && !running;
    *last_running = running;
    just_stopped
}

/// Returns `true` when the client just started connecting on this tick.
pub fn started_connecting(
    mut last_connecting: Local<bool>,
    client: Option<Res<RepliconClient>>,
) -> bool {
    let connecting = client.filter(|client| client.is_connecting()).is_some();

    let started_connecting = !*last_connecting && connecting;
    *last_connecting = connecting;
    started_connecting
}

/// Returns `true` when the client is connected on this tick.
pub fn just_connected(
    mut last_connected: Local<bool>,
    client: Option<Res<RepliconClient>>,
) -> bool {
    let connected = client.filter(|client| client.is_connected()).is_some();

    let just_connected = !*last_connected && connected;
    *last_connected = connected;
    just_connected
}

/// Returns `true` when the client is disconnected on this tick.
pub fn just_disconnected(
    mut last_connected: Local<bool>,
    client: Option<Res<RepliconClient>>,
) -> bool {
    let disconnected = client.filter(|client| client.is_disconnected()).is_some();

    let just_disconnected = *last_connected && disconnected;
    *last_connected = !disconnected;
    just_disconnected
}
