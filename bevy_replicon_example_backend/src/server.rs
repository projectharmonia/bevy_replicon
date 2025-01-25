use std::{
    io, mem,
    net::{Ipv4Addr, UdpSocket},
};

use anyhow::Result;
use bevy::prelude::*;
use bevy_replicon::prelude::*;

/// Adds a server messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleServerPlugin;

impl Plugin for RepliconExampleServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                set_stopped.run_if(resource_removed::<ExampleServerSocket>),
                set_running.run_if(resource_added::<ExampleServerSocket>),
                receive_packets.never_param_warn(),
            )
                .chain()
                .in_set(ServerSet::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .never_param_warn()
                .in_set(ServerSet::SendPackets),
        );
    }
}

/// The socket used by the server.
#[derive(Resource, Deref)]
pub struct ExampleServerSocket(UdpSocket);

impl ExampleServerSocket {
    /// Opens an example server socket on the specified port.
    pub fn new(port: u16) -> Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, port))?;
        socket.set_nonblocking(true)?;
        Ok(Self(socket))
    }
}

fn set_stopped(mut server: ResMut<RepliconServer>) {
    server.set_running(false);
}

fn set_running(mut server: ResMut<RepliconServer>) {
    server.set_running(true);
}

fn receive_packets(
    socket: Res<ExampleServerSocket>,
    mut replicon_server: ResMut<RepliconServer>,
    clients: Res<ConnectedClients>,
    mut server_events: EventWriter<ServerEvent>,
) {
    let mut buf = [0u8; 1502];
    loop {
        let (size, addr) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock {
                    return;
                }
                error!("failed to receive packet: {e}");
                continue;
            }
        };

        let client_id = ClientId::new(addr.port() as u64);
        if size < 1 {
            error!("received empty packet from `{client_id:?}`");
            continue;
        }
        let channel_id = buf[0];
        if !clients.iter().any(|c| c.id() == client_id) {
            server_events.send(ServerEvent::ClientConnected { client_id });
        }
        replicon_server.insert_received(client_id, channel_id, Vec::from(&buf[1..size]));
    }
}

fn send_packets(socket: Res<ExampleServerSocket>, mut replicon_server: ResMut<RepliconServer>) {
    for (client_id, channel_id, message) in replicon_server.drain_sent() {
        let mut data = Vec::with_capacity(message.len() + mem::size_of_val(&channel_id));
        data.push(channel_id);
        data.extend(message);
        let port = client_id.get() as u16;
        socket
            .send_to(&data, (socket.local_addr().unwrap().ip(), port))
            .unwrap();
    }
}
