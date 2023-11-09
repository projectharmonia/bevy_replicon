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
use serde::{Deserialize, Serialize};

pub(super) fn connect(server_app: &mut App, client_app: &mut App) {
    let network_channels = server_app.world.resource::<NetworkChannels>();

    let server_channels_config = network_channels.get_server_configs();
    let client_channels_config = network_channels.get_client_configs();

    let server = RenetServer::new(ConnectionConfig {
        server_channels_config: server_channels_config.clone(),
        client_channels_config: client_channels_config.clone(),
        ..Default::default()
    });
    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let server_transport = create_server_transport();
    let client_transport =
        create_client_transport(server_transport.addresses().first().unwrap().port());

    server_app
        .insert_resource(server)
        .insert_resource(server_transport);

    client_app
        .insert_resource(client)
        .insert_resource(client_transport);

    loop {
        client_app.update();
        server_app.update();
        if client_app.world.resource::<RenetClient>().is_connected() {
            break;
        }
    }
}

const PROTOCOL_ID: u64 = 0;

fn create_server_transport() -> NetcodeServerTransport {
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
        max_clients: 1,
        protocol_id: PROTOCOL_ID,
        public_addresses: vec![public_addr],
        authentication: ServerAuthentication::Unsecure,
    };

    NetcodeServerTransport::new(server_config, socket).unwrap()
}

fn create_client_transport(port: u16) -> NetcodeClientTransport {
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

    NetcodeClientTransport::new(current_time, authentication, socket).unwrap()
}

#[derive(Deserialize, Event, Serialize)]
pub(super) struct DummyEvent(pub(super) Entity);

impl MapNetworkEntities for DummyEvent {
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T) {
        self.0 = mapper.map(self.0);
    }
}
