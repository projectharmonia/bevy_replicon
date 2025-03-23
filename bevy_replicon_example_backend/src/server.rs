use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::Instant,
};

use bevy::prelude::*;
use bevy_replicon::{prelude::*, shared::backend::connected_client::NetworkId};

use super::{
    link_conditioner::{ConditionerConfig, LinkConditioner},
    tcp,
};

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
    mut clients: Query<(Entity, &mut ExampleConnection, Option<&ConditionerConfig>)>,
    global_config: Option<Res<ConditionerConfig>>,
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
                let network_id = NetworkId::new(addr.port().into());
                let client_entity = commands
                    .spawn((
                        ConnectedClient { max_size: 1200 },
                        network_id,
                        ExampleConnection {
                            stream,
                            conditioner: Default::default(),
                        },
                    ))
                    .id();
                debug!("connecting `{client_entity}` with `{network_id:?}`");
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

    let now = Instant::now();
    for (client_entity, mut connection, config) in &mut clients {
        let config = config.or(global_config.as_deref());
        loop {
            match tcp::read_message(&mut connection.stream) {
                Ok((channel_id, message)) => connection
                    .conditioner
                    .insert(config, now, channel_id, message),
                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::WouldBlock => (),
                        io::ErrorKind::UnexpectedEof => {
                            commands.entity(client_entity).despawn();
                            debug!("`client {client_entity}` closed the connection");
                        }
                        _ => {
                            commands.entity(client_entity).despawn();
                            error!(
                                "disconnecting due to message read error from client `{client_entity}`: {e}"
                            );
                        }
                    }
                    break;
                }
            }
        }

        while let Some((channel_id, message)) = connection.conditioner.pop(now) {
            replicon_server.insert_received(client_entity, channel_id, message)
        }
    }
}

fn send_packets(
    mut commands: Commands,
    mut replicon_server: ResMut<RepliconServer>,
    mut clients: Query<&mut ExampleConnection>,
) {
    for (client_entity, channel_id, message) in replicon_server.drain_sent() {
        let mut connection = clients
            .get_mut(client_entity)
            .expect("all connected clients should have streams");
        if let Err(e) = tcp::send_message(&mut connection.stream, channel_id, &message) {
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

/// A connected for a client.
#[derive(Component)]
struct ExampleConnection {
    stream: TcpStream,
    conditioner: LinkConditioner,
}
