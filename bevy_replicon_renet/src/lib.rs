/*!
Provides integration for [`bevy_replicon`](https://docs.rs/bevy_replicon) for [`bevy_renet`](https://docs.rs/bevy_renet).

# Getting started

This guide assumes that you have already read [quick start guide](https://docs.rs/bevy_replicon#quick-start) from `bevy_replicon`.

All Renet API is re-exported from this plugin, you don't need to include `bevy_renet` or `renet` to your `Cargo.toml`.

Renet by default uses the netcode transport which is re-exported by the `renet_transport` feature. If you want to use other transports, you can disable it.

## Initialization

Add [`RepliconRenetPlugins`] along with `RepliconPlugins`:

```
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::RepliconRenetPlugins;

let mut app = App::new();
app.add_plugins((MinimalPlugins, RepliconPlugins, RepliconRenetPlugins));
```

Plugins in [`RepliconRenetPlugins`] automatically add `renet` plugins, you don't need to add them.

If the `renet_transport` feature is enabled, netcode plugins will also be automatically added.

## Server and client creation

To connect to the server or create it, you need to initialize the
[`RenetClient`] and [`NetcodeClientTransport`] **or**
[`RenetServer`] and [`NetcodeServerTransport`](renet::transport::NetcodeServerTransport) resources from Renet.

Never insert client and server resources in the same app for single-player, it will cause a replication loop.

This crate provides the [`RenetChannelsExt`] extension trait to conveniently convert channels
from the [`RepliconChannels`] resource into renet channels.
When creating a server or client you need to use a [`ConnectionConfig`](renet::ConnectionConfig)
from [`renet`], which can be initialized like this:

```
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{renet::ConnectionConfig, RenetChannelsExt, RepliconRenetPlugins};

# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
let channels = app.world.resource::<RepliconChannels>();
let connection_config = ConnectionConfig {
    server_channels_config: channels.get_server_configs(),
    client_channels_config: channels.get_client_configs(),
    ..Default::default()
};
```

For a full example of how to initialize a server or client see the example in the
repository.
*/

use bevy::{app::PluginGroupBuilder, prelude::*};
pub use bevy_renet::*;
use bevy_replicon::prelude::*;
use renet::{ChannelConfig, ClientId, RenetClient, RenetServer, SendType, ServerEvent};
#[cfg(feature = "renet_transport")]
use {
    renet::transport::NetcodeClientTransport,
    transport::{NetcodeClientPlugin, NetcodeServerPlugin},
};

pub struct RepliconRenetServerPlugin;

impl Plugin for RepliconRenetServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RenetServerPlugin)
            .configure_sets(PreUpdate, ServerSet::ReceivePackets.after(RenetReceive))
            .configure_sets(PostUpdate, ServerSet::SendPackets.before(RenetSend))
            .add_systems(
                PreUpdate,
                (
                    (
                        Self::starting_system.run_if(resource_added::<RenetServer>),
                        Self::stopping_system.run_if(resource_removed::<RenetServer>()),
                        Self::receiving_system.run_if(resource_exists::<RenetServer>),
                    )
                        .chain()
                        .in_set(ServerSet::ReceivePackets),
                    Self::peer_events_system.in_set(ServerSet::PeerEvents),
                ),
            )
            .add_systems(
                PostUpdate,
                Self::sending_system
                    .in_set(ServerSet::SendPackets)
                    .run_if(resource_exists::<RenetServer>),
            );

        #[cfg(feature = "renet_transport")]
        app.add_plugins(NetcodeServerPlugin);
    }
}

impl RepliconRenetServerPlugin {
    fn starting_system(mut server: ResMut<RepliconServer>) {
        server.set_running(true);
    }

    fn stopping_system(mut server: ResMut<RepliconServer>) {
        server.set_running(false);
    }

    fn peer_events_system(
        mut server_events: EventReader<ServerEvent>,
        mut peer_events: EventWriter<PeerEvent>,
    ) {
        for event in server_events.read() {
            let replicon_event = match event {
                ServerEvent::ClientConnected { client_id } => PeerEvent::PeerConnected {
                    peer_id: PeerId::new(client_id.raw()),
                },
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    PeerEvent::PeerDisconnected {
                        peer_id: PeerId::new(client_id.raw()),
                        reason: reason.to_string(),
                    }
                }
            };

            peer_events.send(replicon_event);
        }
    }

    fn receiving_system(
        connected_clients: Res<ConnectedClients>,
        channels: Res<RepliconChannels>,
        mut renet_server: ResMut<RenetServer>,
        mut replicon_server: ResMut<RepliconServer>,
    ) {
        for peer_id in connected_clients.iter_peer_ids() {
            for channel_id in 0..channels.client_channels().len() as u8 {
                while let Some(message) =
                    renet_server.receive_message(ClientId::from_raw(peer_id.get()), channel_id)
                {
                    replicon_server.insert_received(peer_id, message, channel_id);
                }
            }
        }
    }

    fn sending_system(
        mut renet_server: ResMut<RenetServer>,
        mut replicon_server: ResMut<RepliconServer>,
    ) {
        for (channel_id, messages) in replicon_server.iter_sent_mut() {
            for (peer_id, message) in messages.drain(..) {
                renet_server.send_message(ClientId::from_raw(peer_id.get()), channel_id, message)
            }
        }
    }
}

pub struct RepliconRenetClientPlugin;

impl Plugin for RepliconRenetClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RenetClientPlugin)
            .configure_sets(PreUpdate, ClientSet::ReceivePackets.after(RenetReceive))
            .configure_sets(PostUpdate, ClientSet::SendPackets.before(RenetSend))
            .add_systems(
                PreUpdate,
                (
                    Self::connecting_system.run_if(resource_added::<RenetClient>),
                    Self::disconnected_system.run_if(bevy_renet::client_just_disconnected),
                    Self::connected_system.run_if(bevy_renet::client_just_connected),
                    Self::receiving_system.run_if(bevy_renet::client_connected),
                )
                    .chain()
                    .in_set(ClientSet::ReceivePackets),
            )
            .add_systems(
                PostUpdate,
                Self::sending_system
                    .in_set(ClientSet::SendPackets)
                    .run_if(bevy_renet::client_connected),
            );

        #[cfg(feature = "renet_transport")]
        app.add_plugins(NetcodeClientPlugin);
    }
}

impl RepliconRenetClientPlugin {
    fn disconnected_system(mut client: ResMut<RepliconClient>) {
        client.set_status(RepliconClientStatus::Disconnected);
    }

    fn connecting_system(mut client: ResMut<RepliconClient>) {
        client.set_status(RepliconClientStatus::Connecting);
    }

    fn connected_system(
        mut client: ResMut<RepliconClient>,
        #[cfg(feature = "renet_transport")] transport: Res<NetcodeClientTransport>,
    ) {
        // In renet only transport knows the ID.
        // TODO: Pending renet issue https://github.com/lucaspoffo/renet/issues/153
        #[cfg(feature = "renet_transport")]
        let peer_id = Some(PeerId::new(transport.client_id().raw()));
        #[cfg(not(feature = "renet_transport"))]
        let peer_id = None;

        client.set_status(RepliconClientStatus::Connected { peer_id });
    }

    fn receiving_system(
        channels: Res<RepliconChannels>,
        mut renet_client: ResMut<RenetClient>,
        mut replicon_client: ResMut<RepliconClient>,
    ) {
        for channel_id in 0..channels.server_channels().len() as u8 {
            while let Some(message) = renet_client.receive_message(channel_id) {
                replicon_client.insert_received(message, channel_id);
            }
        }
    }

    fn sending_system(
        mut renet_client: ResMut<RenetClient>,
        mut replicon_client: ResMut<RepliconClient>,
    ) {
        for (channel_id, messages) in replicon_client.iter_sent_mut() {
            for message in messages.drain(..) {
                renet_client.send_message(channel_id, message)
            }
        }
    }
}

pub struct RepliconRenetPlugins;

impl PluginGroup for RepliconRenetPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(RepliconRenetServerPlugin)
            .add(RepliconRenetClientPlugin)
    }
}

/// External trait for [`RepliconChannels`] to provide convenient conversion into renet channel configs.
pub trait RenetChannelsExt {
    /// Returns server channel configs that can be used to create [`ConnectionConfig`](renet::ConnectionConfig).
    fn get_server_configs(&self) -> Vec<ChannelConfig>;

    /// Same as [`RenetChannelsExt::get_server_configs`], but for clients.
    fn get_client_configs(&self) -> Vec<ChannelConfig>;
}

impl RenetChannelsExt for RepliconChannels {
    fn get_server_configs(&self) -> Vec<ChannelConfig> {
        create_configs(self.server_channels(), self.default_max_bytes)
    }

    fn get_client_configs(&self) -> Vec<ChannelConfig> {
        create_configs(self.client_channels(), self.default_max_bytes)
    }
}

/// Converts replicon channels into renet channel configs.
fn create_configs(channels: &[RepliconChannel], default_max_bytes: usize) -> Vec<ChannelConfig> {
    let mut channel_configs = Vec::with_capacity(channels.len());
    for (index, channel) in channels.iter().enumerate() {
        let send_type = match channel.kind {
            ChannelKind::Unreliable => SendType::Unreliable,
            ChannelKind::Unordered => SendType::ReliableUnordered {
                resend_time: channel.resend_time,
            },
            ChannelKind::Ordered => SendType::ReliableOrdered {
                resend_time: channel.resend_time,
            },
        };
        channel_configs.push(ChannelConfig {
            channel_id: index as u8,
            max_memory_usage_bytes: channel.max_bytes.unwrap_or(default_max_bytes),
            send_type,
        });
    }
    channel_configs
}
