use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
};

use bevy::{ecs::entity::EntityHashMap, prelude::*, utils::Entry};
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
                let client_entity = commands.spawn(ConnectedClient).id();
                if let Err(e) = server.add_connected(client_entity, stream) {
                    error!("unable to accept connection from `{addr}`: {e}");
                    commands.entity(client_entity).despawn();
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

    server.streams.retain(|&client_entity, stream| loop {
        match tcp::read_message(stream) {
            Ok((channel_id, message)) => {
                replicon_server.insert_received(client_entity, channel_id, message)
            }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => return true,
                io::ErrorKind::UnexpectedEof => {
                    commands.entity(client_entity).despawn();
                    debug!("`client {client_entity}` closed the connection");
                    return false;
                }
                _ => {
                    commands.entity(client_entity).despawn();
                    error!("disconnecting due to message read error from client `{client_entity}`: {e}");
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
    for (client_entity, channel_id, message) in replicon_server.drain_sent() {
        match server.streams.entry(client_entity) {
            Entry::Occupied(mut entry) => {
                if let Err(e) = tcp::send_message(entry.get_mut(), channel_id, &message) {
                    commands.entity(client_entity).despawn();
                    error!("disconnecting client `{client_entity}` due to error: {e}");
                    entry.remove();
                }
            }
            Entry::Vacant(_) => error!(
                "unable to send message over channel {channel_id} for non-existing client `{client_entity}`"
            ),
        }
    }
}

/// The socket used by the server.
#[derive(Resource)]
pub struct ExampleServer {
    listener: TcpListener,
    streams: EntityHashMap<TcpStream>,
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
    fn add_connected(&mut self, client_entity: Entity, stream: TcpStream) -> io::Result<()> {
        stream.set_nodelay(true)?;
        stream.set_nonblocking(true)?;
        self.streams.insert(client_entity, stream);

        Ok(())
    }
}
