use std::net::UdpSocket;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

/// Adds a client messaging backend made for examples to bevy_replicon.
pub struct RepliconExampleClientPlugin;

impl Plugin for RepliconExampleClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                set_disconnected.run_if(resource_removed::<ExampleClientSocket>),
                set_connected.run_if(resource_added::<ExampleClientSocket>),
                receive_packets.never_param_warn(),
            )
                .chain()
                .in_set(ClientSet::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .never_param_warn()
                .in_set(ClientSet::SendPackets),
        );
    }
}

/// The socket used by the client
#[derive(Resource, Deref)]
pub struct ExampleClientSocket(UdpSocket);

impl ExampleClientSocket {
    /// Open an example client socket connected to a server on the specified port
    pub fn new(port: u16) -> Option<Self> {
        let socket = match UdpSocket::bind("127.0.0.1:0") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to join: {e}");
                return None;
            }
        };
        socket.set_nonblocking(true).unwrap();
        if let Err(e) = socket.connect(("127.0.0.1", port)) {
            eprintln!("Failed to join: {e}");
            return None;
        };
        Some(ExampleClientSocket(socket))
    }
}

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
                    info!("Got network error, disconnecting: {e}");
                }
                return;
            }
        };
        if size < 1 {
            commands.remove_resource::<ExampleClientSocket>();
            info!("Got empty packet, disconnecting");
            return;
        }
        let channel_id = buf[0];
        replicon_client.insert_received(channel_id, Vec::from(&buf[1..size]));
    }
}

fn send_packets(socket: Res<ExampleClientSocket>, mut replicon_client: ResMut<RepliconClient>) {
    for (channel_id, message) in replicon_client.drain_sent() {
        let mut data = Vec::with_capacity(message.len() + 1);
        data.push(channel_id);
        data.extend(message);
        socket.send(&data).unwrap();
    }
}
