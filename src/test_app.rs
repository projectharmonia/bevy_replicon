use bevy::prelude::*;

use crate::{
    client::replicon_client::{RepliconClient, RepliconClientStatus},
    core::PeerId,
    server::{connected_clients::ConnectedClients, replicon_server::RepliconServer, PeerEvent},
};

/**
Extension for [`App`] to communicate with other instances like it's a server.

# Example

```
use bevy::prelude::*;
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};

let mut server_app = App::new();
let mut client_app = App::new();
for app in [&mut server_app, &mut client_app] {
    app.add_plugins((
        MinimalPlugins,
        // No messaging library plugin required.
        RepliconPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame, // To tick each app update.
            ..Default::default()
        }),
    ));
}

// Simulate connection between two apps:
// - server app will register a connected client,
// - client app will be in connected state.
server_app.connect_client(&mut client_app);

server_app.world.spawn(Replication);

// Run tick for each app and trigger message exchange.
server_app.update();
server_app.exchange_with_client(&mut client_app);
client_app.update();

assert_eq!(
    client_app.world.entities().len(),
    1,
    "client should replicate spawned entity"
);

// You can optionally simulate a disconnect.
server_app.disconnect_client(&mut client_app);
```
**/
pub trait ServerTestAppExt {
    /// Starts server in [`self`] and connects a client app.
    ///
    /// Can be called multiple times on different client apps.
    /// Runs an update for both apps internally.
    ///
    /// # Panics
    ///
    /// Panics if a client app has been connected before.
    fn connect_client(&mut self, client_app: &mut App);

    /// Disconnects a client app from [`self`].
    ///
    /// Can be called multiple times on different client apps.
    /// Runs an update for both apps internally.
    ///
    /// # Panics
    ///
    /// Panics if a client app hasn't been connected before.
    fn disconnect_client(&mut self, client_app: &mut App);

    /// Makes message exchange between client and server.
    ///
    /// Runs an update for [`self`] before sending and after receiving for the client app.
    ///
    /// # Panics
    ///
    /// Panics if a client app hasn't been connected before.
    fn exchange_with_client(&mut self, client_app: &mut App);
}

impl ServerTestAppExt for App {
    fn connect_client(&mut self, client_app: &mut App) {
        let mut client = client_app.world.resource_mut::<RepliconClient>();
        assert!(
            client.is_no_connection(),
            "client can't be connected multiple times"
        );

        // Use client number as ID.
        // Server ID (0) will always be skipped.
        let max_id = self
            .world
            .resource_mut::<ConnectedClients>()
            .iter_peer_ids()
            .max()
            .unwrap_or(PeerId::SERVER);
        let peer_id = PeerId::new(max_id.get() + 1);
        client.set_status(RepliconClientStatus::Connected {
            peer_id: Some(peer_id),
        });

        let mut server = self.world.resource_mut::<RepliconServer>();
        server.set_running(true);

        self.world.send_event(PeerEvent::PeerConnected { peer_id });

        self.update(); // Will update `ConnectedClients`, otherwise next call will assign the same ID.
        client_app.update();
    }

    fn disconnect_client(&mut self, client_app: &mut App) {
        let mut client = client_app.world.resource_mut::<RepliconClient>();
        let peer_id = client
            .peer_id()
            .expect("client should have an assigned ID for disconnect");

        client.set_status(RepliconClientStatus::NoConnection);

        self.world.send_event(PeerEvent::PeerDisconnected {
            peer_id,
            reason: "Disconnected by server".to_string(),
        });

        self.update();
        client_app.update();
    }

    fn exchange_with_client(&mut self, client_app: &mut App) {
        let mut client = client_app.world.resource_mut::<RepliconClient>();
        let peer_id = client
            .peer_id()
            .expect("client should have an assigned ID for exchanging messages");

        let mut server = self.world.resource_mut::<RepliconServer>();
        for (channel_id, messages) in client.iter_sent() {
            for message in messages.drain(..) {
                server.insert_received(peer_id, message, channel_id)
            }
        }

        for (channel_id, messages) in server.iter_sent() {
            messages.retain(|(send_id, message)| {
                if *send_id == peer_id {
                    client.insert_received(message.clone(), channel_id);
                    false
                } else {
                    true
                }
            });
        }
    }
}
