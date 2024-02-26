use bevy::prelude::*;

use crate::{client::replicon_client::RepliconClient, server::replicon_server::RepliconServer};

/// Returns `true` if the server is active.
pub fn server_active(server: Option<Res<RepliconServer>>) -> bool {
    server.filter(|server| server.is_active()).is_some()
}

/// Returns `true` if the client doesn't have a connection.
///
/// Can be used for systems that runs both on server and in singleplayer mode.
pub fn no_connection(client: Option<Res<RepliconClient>>) -> bool {
    client.filter(|client| !client.is_no_connection()).is_none()
}

/// Returns `true` when the client is connecting.
pub fn connecting(client: Option<Res<RepliconClient>>) -> bool {
    client.filter(|client| client.is_connecting()).is_some()
}

/// Returns `true` when the client is connected.
pub fn connected(client: Option<Res<RepliconClient>>) -> bool {
    client.filter(|client| client.is_connected()).is_some()
}

/// Returns `true` if the server became inactive on this tick.
pub fn server_just_deactivated(
    mut last_active: Local<bool>,
    server: Option<Res<RepliconServer>>,
) -> bool {
    let active = server.filter(|server| server.is_active()).is_some();

    let just_deactivated = *last_active && !active;
    *last_active = active;
    just_deactivated
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
    let disconnected = client.filter(|client| client.is_no_connection()).is_some();

    let just_disconnected = *last_connected && disconnected;
    *last_connected = !disconnected;
    just_disconnected
}
