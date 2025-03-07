use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
};

use bevy::{
    platform_support::collections::{hash_map::Entry, HashMap},
    prelude::*,
};
use bevy_replicon::prelude::*;

use super::tcp;

/// Adds a server messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleServerPlugin;

impl Plugin for RepliconExampleServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                set_stopped.run_if(resource_removed::<ExampleServer>),
                set_running.run_if(resource_added::<ExampleServer>),
                receive_packets.ignore_param_missing(),
            )
                .chain()
                .in_set(ServerSet::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .ignore_param_missing()
                .in_set(ServerSet::SendPackets),
        );
    }
}

fn set_stopped(mut server: ResMut<RepliconServer>) {
    server.set_running(false);
}

fn set_running(mut server: ResMut<RepliconServer>) {
    server.set_running(true);
}

fn receive_packets(
    mut commands: Commands,
    mut server: ResMut<ExampleServer>,
    mut replicon_server: ResMut<RepliconServer>,
) {
    loop {
        match server.listener.accept() {
            Ok((stream, addr)) => {
                let client_id = ClientId::new(addr.port().into());
                match server.add_connected(client_id, stream) {
                    Ok(()) => commands.trigger(ClientConnected { client_id }),
                    Err(e) => error!("unable to accept connection from `{client_id:?}`: {e}"),
                }
            }
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    error!("stopping server due to network error: {e}");
                    commands.remove_resource::<ExampleServer>();
                }
                break;
            }
        }
    }

    server.streams.retain(|client_id, stream| loop {
        match tcp::read_message(stream) {
            Ok((channel_id, message)) => {
                replicon_server.insert_received(*client_id, channel_id, message)
            }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => return true,
                io::ErrorKind::UnexpectedEof => {
                    commands.trigger(ClientDisconnected {
                        client_id: *client_id,
                        reason: DisconnectReason::DisconnectedByClient,
                    });
                    return false;
                }
                _ => {
                    commands.trigger(ClientDisconnected {
                        client_id: *client_id,
                        reason: Box::<BackendError>::from(e).into(),
                    });
                    return false;
                }
            },
        }
    });
}

fn send_packets(
    mut commands: Commands,
    mut server: ResMut<ExampleServer>,
    mut replicon_server: ResMut<RepliconServer>,
) {
    for (client_id, channel_id, message) in replicon_server.drain_sent() {
        match server.streams.entry(client_id) {
            Entry::Occupied(mut entry) => {
                if let Err(e) = tcp::send_message(entry.get_mut(), channel_id, &message) {
                    commands.trigger(ClientDisconnected {
                        client_id,
                        reason: e.into(),
                    });
                    entry.remove();
                }
            }
            Entry::Vacant(_) => error!(
                "unable to send message over channel {channel_id} for non-existing `{client_id:?}`"
            ),
        }
    }
}

/// The socket used by the server.
#[derive(Resource)]
pub struct ExampleServer {
    listener: TcpListener,
    streams: HashMap<ClientId, TcpStream>,
}

impl ExampleServer {
    /// Opens an example server socket on the specified port.
    pub fn new(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            streams: Default::default(),
        })
    }

    /// Returns the number of connected clients.
    pub fn connected_clients(&self) -> usize {
        self.streams.len()
    }

    /// Returns local address if the server is running.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Associates a stream with a client and properly configures it.
    fn add_connected(&mut self, client_id: ClientId, stream: TcpStream) -> io::Result<()> {
        stream.set_nodelay(true)?;
        stream.set_nonblocking(true)?;
        self.streams.insert(client_id, stream);

        Ok(())
    }
}
