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
    ConnectionConfig, RenetClient, RenetServer,
};
use bevy_replicon::prelude::*;

pub(super) fn single_client(server_app: &mut App, client_app: &mut App) {
    const CLIENT_ID: u64 = 1;
    let port = setup_server(server_app, 1);
    setup_client(client_app, CLIENT_ID, port);
    wait_for_connection(server_app, client_app);
}

#[allow(dead_code)]
pub(super) fn multiple_clients(server_app: &mut App, client_apps: &mut [App]) {
    const BASE_ID: u64 = 1;
    let port = setup_server(server_app, client_apps.len());
    for (index, client_app) in client_apps.iter_mut().enumerate() {
        setup_client(client_app, BASE_ID + index as u64, port);
        wait_for_connection(server_app, client_app);
    }
}

pub(super) fn setup_client(app: &mut App, client_id: u64, port: u16) {
    let channels = app.world.resource::<RepliconChannels>();

    let server_channels_config = channels.get_server_configs();
    let client_channels_config = channels.get_client_configs();

    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });
    let transport = create_client_transport(client_id, port);

    app.insert_resource(client).insert_resource(transport);
}

pub(super) fn setup_server(app: &mut App, max_clients: usize) -> u16 {
    let channels = app.world.resource::<RepliconChannels>();

    let server_channels_config = channels.get_server_configs();
    let client_channels_config = channels.get_client_configs();

    let server = RenetServer::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });
    let transport = create_server_transport(max_clients);
    let port = transport.addresses().first().unwrap().port();

    app.insert_resource(server).insert_resource(transport);

    port
}

pub(super) fn wait_for_connection(server_app: &mut App, client_app: &mut App) {
    loop {
        client_app.update();
        server_app.update();
        if client_app.world.resource::<RenetClient>().is_connected() {
            break;
        }
    }
}

const PROTOCOL_ID: u64 = 0;

fn create_server_transport(max_clients: usize) -> NetcodeServerTransport {
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let server_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let socket = UdpSocket::bind(server_addr).expect("localhost should be bindable");
    let public_addr = socket
        .local_addr()
        .expect("socket should autodetect local address");
    let server_config = ServerConfig {
        current_time,
        max_clients,
        protocol_id: PROTOCOL_ID,
        public_addresses: vec![public_addr],
        authentication: ServerAuthentication::Unsecure,
    };

    NetcodeServerTransport::new(server_config, socket).unwrap()
}

fn create_client_transport(client_id: u64, port: u16) -> NetcodeClientTransport {
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let ip = Ipv4Addr::LOCALHOST.into();
    let server_addr = SocketAddr::new(ip, port);
    let socket = UdpSocket::bind((ip, 0)).expect("localhost should be bindable");
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: None,
    };

    NetcodeClientTransport::new(current_time, authentication, socket).unwrap()
}
