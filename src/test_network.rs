use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_renet::renet::{
    ChannelConfig, ClientAuthentication, RenetClient, RenetConnectionConfig, RenetServer,
    ServerAuthentication, ServerConfig,
};

use super::replication_core::NetworkChannels;

/// Automates server and client creation for unit tests.
pub(super) struct TestNetworkPlugin;

impl Plugin for TestNetworkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MinimalPlugins);

        let network_channels = app.world.resource_mut::<NetworkChannels>();
        let client_channels = network_channels.client_channels();
        let server_channels = network_channels.server_channels();

        app.insert_resource(create_server(
            server_channels.clone(),
            client_channels.clone(),
        ));
        app.insert_resource(create_client(
            app.world.resource::<RenetServer>().addr().port(),
            client_channels,
            server_channels,
        ));

        loop {
            app.update();
            if app.world.resource::<RenetClient>().is_connected() {
                break;
            }
        }
    }
}

const PROTOCOL_ID: u64 = 0;

fn create_server(
    send_channels_config: Vec<ChannelConfig>,
    receive_channels_config: Vec<ChannelConfig>,
) -> RenetServer {
    const MAX_CLIENTS: usize = 1;
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let server_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let socket = UdpSocket::bind(server_addr).expect("localhost should be bindable");
    let local_addr = socket
        .local_addr()
        .expect("socket should autodetect local address");
    let server_config = ServerConfig::new(
        MAX_CLIENTS,
        PROTOCOL_ID,
        local_addr,
        ServerAuthentication::Unsecure,
    );

    let connection_config = RenetConnectionConfig {
        send_channels_config,
        receive_channels_config,
        ..Default::default()
    };

    RenetServer::new(current_time, server_config, connection_config, socket).unwrap()
}

fn create_client(
    port: u16,
    send_channels_config: Vec<ChannelConfig>,
    receive_channels_config: Vec<ChannelConfig>,
) -> RenetClient {
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let client_id = current_time.as_millis() as u64;
    let ip = Ipv4Addr::LOCALHOST.into();
    let server_addr = SocketAddr::new(ip, port);
    let socket = UdpSocket::bind((ip, 0)).expect("localhost should be bindable");
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: None,
    };

    let connection_config = RenetConnectionConfig {
        send_channels_config,
        receive_channels_config,
        ..Default::default()
    };

    RenetClient::new(current_time, socket, connection_config, authentication).unwrap()
}
