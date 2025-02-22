use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpStream},
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::tcp;

/// Adds a client messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleClientPlugin;

impl Plugin for RepliconExampleClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                set_disconnected.run_if(resource_removed::<ExampleClient>),
                set_connected.run_if(resource_added::<ExampleClient>),
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

fn set_disconnected(mut replicon_client: ResMut<RepliconClient>) {
    replicon_client.set_status(RepliconClientStatus::Disconnected);
}

fn set_connected(mut replicon_client: ResMut<RepliconClient>) {
    replicon_client.set_status(RepliconClientStatus::Connected);
}

fn receive_packets(
    mut commands: Commands,
    mut client: ResMut<ExampleClient>,
    mut replicon_client: ResMut<RepliconClient>,
) {
    loop {
        match tcp::read_message(&mut client.0) {
            Ok((channel_id, message)) => replicon_client.insert_received(channel_id, message),
            Err(e) => {
                match e.kind() {
                    io::ErrorKind::WouldBlock => (),
                    io::ErrorKind::UnexpectedEof => {
                        debug!("server closed the connection");
                        commands.remove_resource::<ExampleClient>();
                    }
                    _ => {
                        error!("disconnecting due to message read error: {e}");
                        commands.remove_resource::<ExampleClient>();
                    }
                }
                return;
            }
        }
    }
}

fn send_packets(
    mut commands: Commands,
    mut client: ResMut<ExampleClient>,
    mut replicon_client: ResMut<RepliconClient>,
) {
    for (channel_id, message) in replicon_client.drain_sent() {
        if let Err(e) = tcp::send_message(&mut client.0, channel_id, &message) {
            error!("disconnecting due message write error: {e}");
            commands.remove_resource::<ExampleClient>();
            return;
        }
    }
}

/// The socket used by the client.
#[derive(Resource)]
pub struct ExampleClient(TcpStream);

impl ExampleClient {
    /// Opens an example client socket connected to a server on the specified port.
    pub fn new(port: u16) -> io::Result<Self> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self(stream))
    }

    /// Returns local address if connected.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }

    /// Returns true if the client is connected.
    pub fn is_connected(&self) -> bool {
        self.local_addr().is_ok()
    }
}
