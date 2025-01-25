use std::{
    mem,
    net::{Ipv4Addr, UdpSocket},
};

use anyhow::Result;
use bevy::prelude::*;
use bevy_replicon::prelude::*;

/// Adds a client messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleClientPlugin;

impl Plugin for RepliconExampleClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                Self::set_disconnected.run_if(resource_removed::<ExampleClientSocket>),
                Self::set_connected.run_if(resource_added::<ExampleClientSocket>),
                Self::receive_packets.never_param_warn(),
            )
                .chain()
                .in_set(ClientSet::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            Self::send_packets
                .never_param_warn()
                .in_set(ClientSet::SendPackets),
        );
    }
}

impl RepliconExampleClientPlugin {
    fn set_disconnected(mut client: ResMut<RepliconClient>) {
        client.set_status(RepliconClientStatus::Disconnected);
    }

    fn set_connected(mut client: ResMut<RepliconClient>) {
        client.set_status(RepliconClientStatus::Connected { client_id: None });
    }

    fn receive_packets(
        mut commands: Commands,
        socket: Res<ExampleClientSocket>,
        mut replicon_client: ResMut<RepliconClient>,
    ) {
        let mut buf = [0u8; 1502];
        loop {
            let size = match socket.recv(&mut buf) {
                Ok(s) => s,
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::WouldBlock {
                        commands.remove_resource::<ExampleClientSocket>();
                        error!("disconnecting due to network error: {e}");
                    }
                    return;
                }
            };
            if size < 1 {
                commands.remove_resource::<ExampleClientSocket>();
                debug!("got empty packet, disconnecting");
                return;
            }
            let channel_id = buf[0];
            replicon_client.insert_received(channel_id, Vec::from(&buf[1..size]));
        }
    }

    fn send_packets(socket: Res<ExampleClientSocket>, mut replicon_client: ResMut<RepliconClient>) {
        for (channel_id, message) in replicon_client.drain_sent() {
            let mut data = Vec::with_capacity(message.len() + mem::size_of_val(&channel_id));
            data.push(channel_id);
            data.extend(message);
            socket.send(&data).unwrap();
        }
    }
}

/// The socket used by the client.
#[derive(Resource, Deref)]
pub struct ExampleClientSocket(UdpSocket);

impl ExampleClientSocket {
    /// Opens an example client socket connected to a server on the specified port.
    pub fn new(port: u16) -> Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        socket.set_nonblocking(true)?;
        socket.connect((Ipv4Addr::LOCALHOST, port))?;
        Ok(Self(socket))
    }
}
