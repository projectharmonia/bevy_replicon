use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use clap::Parser;
use serde::{Deserialize, Serialize};

use bevy_replicon::{
    prelude::*,
    renet::{
        transport::{
            ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport,
            ServerAuthentication, ServerConfig,
        },
        ConnectionConfig, ServerEvent,
    },
};

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins((DefaultPlugins, ReplicationPlugins, SimpleBoxPlugin))
        .run();
}

struct SimpleBoxPlugin;

impl Plugin for SimpleBoxPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<PlayerPosition>()
            .replicate::<PlayerColor>()
            .add_client_event::<MoveCommandEvent>(SendPolicy::Ordered)
            .add_systems(
                Startup,
                (
                    Self::cli_system.pipe(system_adapter::unwrap),
                    Self::init_system,
                ),
            )
            .add_systems(
                Update,
                (
                    Self::movement_system.run_if(has_authority()), // Runs only on the server or a single player.
                    Self::server_event_system.run_if(resource_exists::<RenetServer>()), // Runs only on the server.
                    (Self::draw_box_system, Self::input_system),
                ),
            );
    }
}

impl SimpleBoxPlugin {
    fn cli_system(
        mut commands: Commands,
        cli: Res<Cli>,
        network_channels: Res<NetworkChannels>,
    ) -> Result<(), Box<dyn Error>> {
        match *cli {
            Cli::SinglePlayer => {
                commands.spawn(PlayerBundle::new(SERVER_ID, Vec2::ZERO, Color::GREEN));
            }
            Cli::Server { port } => {
                let server_channels_config = network_channels.server_channels();
                let client_channels_config = network_channels.client_channels();

                let server = RenetServer::new(ConnectionConfig {
                    server_channels_config,
                    client_channels_config,
                    ..Default::default()
                });

                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let public_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
                let socket = UdpSocket::bind(public_addr)?;
                let server_config = ServerConfig {
                    max_clients: 10,
                    protocol_id: PROTOCOL_ID,
                    public_addr,
                    authentication: ServerAuthentication::Unsecure,
                };
                let transport = NetcodeServerTransport::new(current_time, server_config, socket)?;

                commands.insert_resource(server);
                commands.insert_resource(transport);

                commands.spawn(TextBundle::from_section(
                    "Server",
                    TextStyle {
                        font_size: 30.0,
                        color: Color::WHITE,
                        ..default()
                    },
                ));
                commands.spawn(PlayerBundle::new(SERVER_ID, Vec2::ZERO, Color::GREEN));
            }
            Cli::Client { port, ip } => {
                let server_channels_config = network_channels.server_channels();
                let client_channels_config = network_channels.client_channels();

                let client = RenetClient::new(ConnectionConfig {
                    server_channels_config,
                    client_channels_config,
                    ..Default::default()
                });

                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let client_id = current_time.as_millis() as u64;
                let server_addr = SocketAddr::new(ip, port);
                let socket = UdpSocket::bind((ip, 0))?;
                let authentication = ClientAuthentication::Unsecure {
                    client_id,
                    protocol_id: PROTOCOL_ID,
                    server_addr,
                    user_data: None,
                };
                let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

                commands.insert_resource(client);
                commands.insert_resource(transport);

                commands.spawn(TextBundle::from_section(
                    format!("Client: {client_id:?}"),
                    TextStyle {
                        font_size: 30.0,
                        color: Color::WHITE,
                        ..default()
                    },
                ));
            }
        }

        Ok(())
    }

    fn init_system(mut commands: Commands) {
        commands.spawn(Camera2dBundle::default());
    }

    /// Logs server events and spawns a new player whenever a client connects.
    fn server_event_system(mut commands: Commands, mut server_event: EventReader<ServerEvent>) {
        for event in &mut server_event {
            match event {
                ServerEvent::ClientConnected { client_id } => {
                    info!("Player: {client_id} Connected");
                    // Generate pseudo random color from client id.
                    let r = ((client_id % 23) as f32) / 23.0;
                    let g = ((client_id % 27) as f32) / 27.0;
                    let b = ((client_id % 39) as f32) / 39.0;
                    commands.spawn(PlayerBundle::new(
                        *client_id,
                        Vec2::ZERO,
                        Color::rgb(r, g, b),
                    ));
                }
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    info!("Client {client_id} disconnected: {reason}");
                }
            }
        }
    }

    fn draw_box_system(players: Query<(&PlayerPosition, &PlayerColor)>, mut gizmos: Gizmos) {
        for (position, color) in players.iter() {
            gizmos.rect(
                Vec3::new(position.x, position.y, 0.0),
                Quat::IDENTITY,
                Vec2::ONE * 50.0,
                **color,
            );
        }
    }

    /// Reads player inputs and sends [`MoveCommandEvents`]
    fn input_system(input: Res<Input<KeyCode>>, mut move_event: EventWriter<MoveCommandEvent>) {
        let mut direction = Vec2::ZERO;
        if input.pressed(KeyCode::Right) {
            direction.x += 1.0;
        }
        if input.pressed(KeyCode::Left) {
            direction.x -= 1.0;
        }
        if input.pressed(KeyCode::Up) {
            direction.y += 1.0;
        }
        if input.pressed(KeyCode::Down) {
            direction.y -= 1.0;
        }
        if direction != Vec2::ZERO {
            move_event.send(MoveCommandEvent {
                direction: direction.normalize_or_zero(),
            })
        }
    }

    /// Mutate [`PlayerPosition`] based on [`MoveCommandEvents`].
    fn movement_system(
        mut events: EventReader<FromClient<MoveCommandEvent>>,
        mut players: Query<(&Player, &mut PlayerPosition)>,
        time: Res<Time>,
    ) {
        let move_speed = 300.0;
        for FromClient { client_id, event } in &mut events {
            info!("received event {event:?} from client {client_id}");
            for (player, mut position) in players.iter_mut() {
                if *client_id == **player {
                    **position += event.direction * time.delta_seconds() * move_speed;
                }
            }
        }
    }
}

const PORT: u16 = 5000;
const PROTOCOL_ID: u64 = 0;

#[derive(Debug, Parser, PartialEq, Resource)]
enum Cli {
    SinglePlayer,
    Server {
        #[arg(short, long, default_value_t = PORT)]
        port: u16,
    },
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,

        #[arg(short, long, default_value_t = PORT)]
        port: u16,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    position: PlayerPosition,
    color: PlayerColor,
    replication: Replication,
}

impl PlayerBundle {
    fn new(id: u64, position: Vec2, color: Color) -> Self {
        Self {
            player: Player(id),
            position: PlayerPosition(position),
            color: PlayerColor(color),
            replication: Replication,
        }
    }
}

/// Contains the client ID of the player.
#[derive(Component, Serialize, Deserialize, Deref)]
struct Player(u64);

#[derive(Component, Deserialize, Serialize, Deref, DerefMut)]
struct PlayerPosition(Vec2);

#[derive(Component, Deserialize, Serialize, Deref)]
struct PlayerColor(Color);

#[derive(Debug, Default, Deserialize, Event, Serialize)]
struct MoveCommandEvent {
    pub direction: Vec2,
}
