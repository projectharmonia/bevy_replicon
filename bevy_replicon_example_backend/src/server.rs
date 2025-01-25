use std::{
    io,
    net::{Ipv4Addr, UdpSocket},
};

use anyhow::Result;
use bevy::prelude::*;
use bevy_replicon::prelude::*;

/// Adds a server messaging backend made for examples to bevy_replicon.
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

/// The socket used by the server
#[derive(Resource, Deref)]
pub struct ExampleServerSocket(UdpSocket);

impl ExampleServerSocket {
    /// Open an example server socket on the specified port
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
                eprintln!("Failed to receive packet");
                continue;
            }
        };
        if size < 1 {
            eprintln!("Packet is of size 0");
            continue;
        }
        let channel_id = buf[0];
        let client_id = ClientId::new(addr.port() as u64);
        if !clients.iter().any(|c| c.id() == client_id) {
            eprintln!("New client! {}", client_id.get());
            server_events.send(ServerEvent::ClientConnected { client_id });
        }
        replicon_server.insert_received(client_id, channel_id, Vec::from(&buf[1..size]));
    }
}

fn send_packets(socket: Res<ExampleServerSocket>, mut replicon_server: ResMut<RepliconServer>) {
    eprintln!("Trying to send packets");
    for (client_id, channel_id, message) in replicon_server.drain_sent() {
        eprintln!(
            "Sending to {} on {}: {:x}",
            client_id.get(),
            channel_id,
            message
        );
        let mut data = Vec::with_capacity(message.len() + 1);
        data.push(channel_id);
        data.extend(message);
        let port = client_id.get() as u16;
        socket
            .send_to(&data, (socket.local_addr().unwrap().ip(), port))
            .unwrap();
    }
}
