use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_renet::renet::{
    transport::{
        ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
        ServerConfig,
    },
    ChannelConfig, ConnectionConfig, RenetClient, RenetServer,
};

use super::replication_core::NetworkChannels;

pub(super) fn setup(app: &mut App) {
    let network_channels = app.world.resource_mut::<NetworkChannels>();
    let server_channels = network_channels.server_channels();
    let client_channels = network_channels.client_channels();
    let (server, server_transport) =
        create_server(server_channels.clone(), client_channels.clone());
    let (client, client_transport) = create_client(
        server_transport.addr().port(),
        server_channels,
        client_channels,
    );

    app.insert_resource(server)
        .insert_resource(server_transport)
        .insert_resource(client)
        .insert_resource(client_transport);

    loop {
        app.update();
        if app
            .world
            .resource::<NetcodeClientTransport>()
            .is_connected()
        {
            break;
        }
    }
}

const PROTOCOL_ID: u64 = 0;

fn create_server(
    server_channels_config: Vec<ChannelConfig>,
    client_channels_config: Vec<ChannelConfig>,
) -> (RenetServer, NetcodeServerTransport) {
    let server = RenetServer::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let server_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let socket = UdpSocket::bind(server_addr).expect("localhost should be bindable");
    let public_addr = socket
        .local_addr()
        .expect("socket should autodetect local address");
    let server_config = ServerConfig {
        max_clients: 1,
        protocol_id: PROTOCOL_ID,
        public_addr,
        authentication: ServerAuthentication::Unsecure,
    };

    let transport = NetcodeServerTransport::new(current_time, server_config, socket).unwrap();

    (server, transport)
}

fn create_client(
    port: u16,
    server_channels_config: Vec<ChannelConfig>,
    client_channels_config: Vec<ChannelConfig>,
) -> (RenetClient, NetcodeClientTransport) {
    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

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
    let transport = NetcodeClientTransport::new(current_time, authentication, socket).unwrap();

    (client, transport)
}
