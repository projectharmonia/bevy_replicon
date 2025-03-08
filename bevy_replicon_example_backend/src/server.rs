use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
};

use bevy::prelude::*;
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
    server: Res<ExampleServer>,
    mut replicon_server: ResMut<RepliconServer>,
    mut clients: Query<(Entity, &mut ClientStream)>,
) {
    loop {
        match server.0.accept() {
            Ok((stream, addr)) => {
                if let Err(e) = stream.set_nodelay(true) {
                    error!("unable to disable buffering for `{addr}`: {e}");
                    continue;
                }
                if let Err(e) = stream.set_nonblocking(true) {
                    error!("unable to enable non-blocking for `{addr}`: {e}");
                    continue;
                }
                commands.spawn((
                    ConnectedClient::new(addr.port().into()),
                    ClientStream(stream),
                ));
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

    for (client_entity, mut stream) in &mut clients {
        loop {
            match tcp::read_message(&mut stream) {
                Ok((channel_id, message)) => {
                    replicon_server.insert_received(client_entity, channel_id, message)
                }
                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::WouldBlock => (),
                        io::ErrorKind::UnexpectedEof => {
                            commands.entity(client_entity).despawn();
                            debug!("`client {client_entity}` closed the connection");
                        }
                        _ => {
                            commands.entity(client_entity).despawn();
                            error!("disconnecting due to message read error from client `{client_entity}`: {e}");
                        }
                    }
                    break;
                }
            }
        }
    }
}

fn send_packets(
    mut commands: Commands,
    mut replicon_server: ResMut<RepliconServer>,
    mut clients: Query<&mut ClientStream>,
) {
    for (client_entity, channel_id, message) in replicon_server.drain_sent() {
        let mut stream = clients
            .get_mut(client_entity)
            .expect("all connected clients should have streams");
        if let Err(e) = tcp::send_message(&mut stream, channel_id, &message) {
            commands.entity(client_entity).despawn();
            error!("disconnecting client `{client_entity}` due to error: {e}");
        }
    }
}

/// The socket used by the server.
#[derive(Resource)]
pub struct ExampleServer(TcpListener);

impl ExampleServer {
    /// Opens an example server socket on the specified port.
    pub fn new(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        Ok(Self(listener))
    }

    /// Returns local address if the server is running.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

/// A stream for a connected client.
#[derive(Component, Deref, DerefMut)]
struct ClientStream(TcpStream);
