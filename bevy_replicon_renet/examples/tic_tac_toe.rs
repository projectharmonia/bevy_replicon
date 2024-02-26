//! A game to showcase single-player and multiplier game.
//! Run it with `--hotseat` to play locally or with `--client` / `--server`

use std::{
    error::Error,
    fmt::{self, Formatter},
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    renet::{
        transport::{
            ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport,
            ServerAuthentication, ServerConfig,
        },
        ConnectionConfig, RenetClient, RenetServer,
    },
    RenetChannelsExt, RepliconRenetPlugins,
};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins((
            DefaultPlugins.build().set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Tic-Tac-Toe".into(),
                    resolution: (800.0, 600.0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            RepliconPlugins,
            RepliconRenetPlugins,
            TicTacToePlugin,
        ))
        .run();
}

struct TicTacToePlugin;

impl Plugin for TicTacToePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<SymbolFont>()
            .init_resource::<CurrentTurn>()
            .replicate::<Symbol>()
            .replicate::<CellIndex>()
            .replicate::<Player>()
            .add_client_event::<CellPick>(ChannelKind::Ordered)
            .insert_resource(ClearColor(BACKGROUND_COLOR))
            .add_systems(
                Startup,
                (Self::ui_setup_system, Self::cli_system.map(Result::unwrap)),
            )
            .add_systems(
                OnEnter(GameState::InGame),
                (Self::turn_text_system, Self::symbol_turn_text_system),
            )
            .add_systems(
                OnEnter(GameState::Disconnected),
                Self::client_disconnected_text_system,
            )
            .add_systems(OnEnter(GameState::Winner), Self::winner_text_system)
            .add_systems(OnEnter(GameState::Tie), Self::tie_text_system)
            .add_systems(
                Update,
                (
                    Self::connecting_text_system.run_if(resource_added::<RenetClient>),
                    Self::server_waiting_text_system.run_if(resource_added::<RenetServer>),
                    Self::server_event_system.run_if(server_active),
                    Self::start_game_system
                        .run_if(connected)
                        .run_if(any_component_added::<Player>), // Wait until client replicates players before starting the game.
                    (
                        Self::cell_interatction_system.run_if(local_player_turn),
                        Self::picking_system.run_if(no_connection),
                        Self::symbol_init_system,
                        Self::turn_advance_system.run_if(any_component_added::<CellIndex>),
                        Self::symbol_turn_text_system.run_if(resource_changed::<CurrentTurn>),
                    )
                        .run_if(in_state(GameState::InGame)),
                ),
            );
    }
}

const GRID_SIZE: usize = 3;

const BACKGROUND_COLOR: Color = Color::rgb(0.9, 0.9, 0.9);

const PROTOCOL_ID: u64 = 0;

// Bottom text defined in two sections, first for text and second for symbols with different font.
const TEXT_SECTION: usize = 0;
const SYMBOL_SECTION: usize = 1;

impl TicTacToePlugin {
    fn ui_setup_system(mut commands: Commands, symbol_font: Res<SymbolFont>) {
        commands.spawn(Camera2dBundle::default());

        const LINES_COUNT: usize = GRID_SIZE + 1;

        const CELL_SIZE: f32 = 100.0;
        const LINE_THICKNESS: f32 = 10.0;
        const BOARD_SIZE: f32 = CELL_SIZE * GRID_SIZE as f32 + LINES_COUNT as f32 * LINE_THICKNESS;

        const BOARD_COLOR: Color = Color::rgb(0.8, 0.8, 0.8);

        for line in 0..LINES_COUNT {
            let position = -BOARD_SIZE / 2.0
                + line as f32 * (CELL_SIZE + LINE_THICKNESS)
                + LINE_THICKNESS / 2.0;

            // Horizontal
            commands.spawn(SpriteBundle {
                sprite: Sprite {
                    color: BOARD_COLOR,
                    ..Default::default()
                },
                transform: Transform {
                    translation: Vec3::Y * position,
                    scale: Vec3::new(BOARD_SIZE, LINE_THICKNESS, 1.0),
                    ..Default::default()
                },
                ..Default::default()
            });

            // Vertical
            commands.spawn(SpriteBundle {
                sprite: Sprite {
                    color: BOARD_COLOR,
                    ..Default::default()
                },
                transform: Transform {
                    translation: Vec3::X * position,
                    scale: Vec3::new(LINE_THICKNESS, BOARD_SIZE, 1.0),
                    ..Default::default()
                },
                ..Default::default()
            });
        }

        const BUTTON_SIZE: f32 = CELL_SIZE / 1.2;
        const BUTTON_MARGIN: f32 = (CELL_SIZE + LINE_THICKNESS - BUTTON_SIZE) / 2.0;

        const TEXT_COLOR: Color = Color::rgb(0.5, 0.5, 1.0);
        const FONT_SIZE: f32 = 40.0;

        commands
            .spawn(NodeBundle {
                style: Style {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_children(|parent| {
                parent
                    .spawn(NodeBundle {
                        style: Style {
                            flex_direction: FlexDirection::Column,
                            width: Val::Px(BOARD_SIZE - LINE_THICKNESS),
                            height: Val::Px(BOARD_SIZE - LINE_THICKNESS),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .with_children(|parent| {
                        parent
                            .spawn((
                                GridNode,
                                NodeBundle {
                                    style: Style {
                                        display: Display::Grid,
                                        grid_template_columns: vec![GridTrack::auto(); GRID_SIZE],
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                },
                            ))
                            .with_children(|parent| {
                                for _ in 0..GRID_SIZE * GRID_SIZE {
                                    parent.spawn(ButtonBundle {
                                        style: Style {
                                            width: Val::Px(BUTTON_SIZE),
                                            height: Val::Px(BUTTON_SIZE),
                                            margin: UiRect::all(Val::Px(BUTTON_MARGIN)),
                                            ..Default::default()
                                        },
                                        background_color: BACKGROUND_COLOR.into(),
                                        ..Default::default()
                                    });
                                }
                            });

                        parent
                            .spawn(NodeBundle {
                                style: Style {
                                    margin: UiRect::top(Val::Px(20.0)),
                                    justify_content: JustifyContent::Center,
                                    ..Default::default()
                                },
                                ..Default::default()
                            })
                            .with_children(|parent| {
                                parent.spawn((
                                    TextBundle::from_sections([
                                        TextSection::new(
                                            String::new(),
                                            TextStyle {
                                                font_size: FONT_SIZE,
                                                color: TEXT_COLOR,
                                                ..Default::default()
                                            },
                                        ),
                                        TextSection::new(
                                            String::new(),
                                            TextStyle {
                                                font: symbol_font.0.clone(),
                                                font_size: FONT_SIZE,
                                                ..Default::default()
                                            },
                                        ),
                                    ]),
                                    BottomText,
                                ));
                            });
                    });
            });
    }

    fn cli_system(
        mut commands: Commands,
        mut game_state: ResMut<NextState<GameState>>,
        cli: Res<Cli>,
        channels: Res<RepliconChannels>,
    ) -> Result<(), Box<dyn Error>> {
        match *cli {
            Cli::Hotseat => {
                // Set all players to server to play from a single machine and start the game right away.
                commands.spawn(PlayerBundle::server(Symbol::Cross));
                commands.spawn(PlayerBundle::server(Symbol::Nought));
                game_state.set(GameState::InGame);
            }
            Cli::Server { port, symbol } => {
                let server_channels_config = channels.get_server_configs();
                let client_channels_config = channels.get_client_configs();

                let server = RenetServer::new(ConnectionConfig {
                    server_channels_config,
                    client_channels_config,
                    ..Default::default()
                });

                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let public_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
                let socket = UdpSocket::bind(public_addr)?;
                let server_config = ServerConfig {
                    current_time,
                    max_clients: 1,
                    protocol_id: PROTOCOL_ID,
                    authentication: ServerAuthentication::Unsecure,
                    public_addresses: vec![public_addr],
                };
                let transport = NetcodeServerTransport::new(server_config, socket)?;

                commands.insert_resource(server);
                commands.insert_resource(transport);
                commands.spawn(PlayerBundle::server(symbol));
            }
            Cli::Client { port, ip } => {
                let server_channels_config = channels.get_server_configs();
                let client_channels_config = channels.get_client_configs();

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
            }
        }

        Ok(())
    }

    fn turn_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[0].value = "Current turn: ".into();
    }

    fn symbol_turn_text_system(
        mut bottom_text: Query<&mut Text, With<BottomText>>,
        current_turn: Res<CurrentTurn>,
    ) {
        let symbol_section = &mut bottom_text.single_mut().sections[SYMBOL_SECTION];
        symbol_section.value = current_turn.glyph().into();
        symbol_section.style.color = current_turn.color();
    }

    fn client_disconnected_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        let sections = &mut bottom_text.single_mut().sections;
        sections[TEXT_SECTION].value = "Client disconnected".into();
        sections[SYMBOL_SECTION].value.clear();
    }

    fn winner_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[TEXT_SECTION].value = "Winner: ".into();
    }

    fn tie_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        let sections = &mut bottom_text.single_mut().sections;
        sections[TEXT_SECTION].value = "Tie".into();
        sections[SYMBOL_SECTION].value.clear();
    }

    fn connecting_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[TEXT_SECTION].value = "Connecting".into();
    }

    fn server_waiting_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[TEXT_SECTION].value = "Waiting player".into();
    }

    /// Waits for client to connect to start the game or disconnect to finish it.
    ///
    /// Only for server.
    fn server_event_system(
        mut commands: Commands,
        mut peer_events: EventReader<PeerEvent>,
        mut game_state: ResMut<NextState<GameState>>,
        players: Query<&Symbol, With<Player>>,
    ) {
        for event in peer_events.read() {
            match event {
                PeerEvent::PeerConnected { peer_id } => {
                    let server_symbol = players.single();
                    commands.spawn(PlayerBundle::new(*peer_id, server_symbol.next()));
                    game_state.set(GameState::InGame);
                }
                PeerEvent::PeerDisconnected { .. } => {
                    game_state.set(GameState::Disconnected);
                }
            }
        }
    }

    fn start_game_system(mut game_state: ResMut<NextState<GameState>>) {
        game_state.set(GameState::InGame);
    }

    fn cell_interatction_system(
        mut buttons: Query<
            (Entity, &Parent, &Interaction, &mut BackgroundColor),
            Changed<Interaction>,
        >,
        children: Query<&Children>,
        mut pick_events: EventWriter<CellPick>,
    ) {
        const HOVER_COLOR: Color = Color::rgb(0.85, 0.85, 0.85);

        for (button_entity, button_parent, interaction, mut background) in &mut buttons {
            match interaction {
                Interaction::Pressed => {
                    let buttons = children.get(**button_parent).unwrap();
                    let index = buttons
                        .iter()
                        .position(|&entity| entity == button_entity)
                        .unwrap();

                    // We send a pick event and wait for the pick to be replicated back to the client.
                    // In case of server or single-player the event will re-translated into [`FromPeer`] event to re-use the logic.
                    pick_events.send(CellPick(index));
                }
                Interaction::Hovered => *background = HOVER_COLOR.into(),
                Interaction::None => *background = BACKGROUND_COLOR.into(),
            };
        }
    }

    /// Handles cell pick events.
    ///
    /// Only for single-player and server.
    fn picking_system(
        mut commands: Commands,
        mut pick_events: EventReader<FromPeer<CellPick>>,
        symbols: Query<&CellIndex>,
        current_turn: Res<CurrentTurn>,
        players: Query<(&Player, &Symbol)>,
    ) {
        for FromPeer { peer_id, event } in pick_events.read().copied() {
            // It's good to check the received data, client could be cheating.
            if event.0 > GRID_SIZE * GRID_SIZE {
                debug!("received invalid cell index {:?}", event.0);
                continue;
            }

            if !players
                .iter()
                .any(|(player, &symbol)| player.0 == peer_id && symbol == current_turn.0)
            {
                debug!("{peer_id:?} chose cell {:?} at wrong turn", event.0);
                continue;
            }

            if symbols.iter().any(|cell_index| cell_index.0 == event.0) {
                debug!(
                    "{peer_id:?} has chosen an already occupied cell {:?}",
                    event.0
                );
                continue;
            }

            // Spawn "blueprint" of the cell that client will replicate.
            commands.spawn(SymbolBundle::new(current_turn.0, event.0));
        }
    }

    /// Initializes spawned symbol on client after replication and on server / single-player right after the spawn.
    fn symbol_init_system(
        mut commands: Commands,
        symbol_font: Res<SymbolFont>,
        symbols: Query<(Entity, &CellIndex, &Symbol), Added<Symbol>>,
        grid_nodes: Query<&Children, With<GridNode>>,
        mut background_colors: Query<&mut BackgroundColor>,
    ) {
        for (symbol_entity, cell_index, symbol) in &symbols {
            let children = grid_nodes.single();
            let button_entity = *children
                .get(cell_index.0)
                .expect("symbols should point to valid buttons");

            let mut background = background_colors
                .get_mut(button_entity)
                .expect("buttons should be initialized with color");
            *background = BACKGROUND_COLOR.into();

            commands
                .entity(button_entity)
                .remove::<Interaction>()
                .add_child(symbol_entity);

            commands
                .entity(symbol_entity)
                .insert(TextBundle::from_section(
                    symbol.glyph(),
                    TextStyle {
                        font: symbol_font.0.clone(),
                        font_size: 80.0,
                        color: symbol.color(),
                    },
                ));
        }
    }

    /// Checks the winner and advances the turn.
    fn turn_advance_system(
        mut current_turn: ResMut<CurrentTurn>,
        mut game_state: ResMut<NextState<GameState>>,
        symbols: Query<(&CellIndex, &Symbol)>,
    ) {
        let mut board = [None; GRID_SIZE * GRID_SIZE];
        for (cell_index, &symbol) in &symbols {
            board[cell_index.0] = Some(symbol);
        }

        const WIN_CONDITIONS: [[usize; GRID_SIZE]; 8] = [
            [0, 1, 2],
            [3, 4, 5],
            [6, 7, 8],
            [0, 3, 6],
            [1, 4, 7],
            [2, 5, 8],
            [0, 4, 8],
            [2, 4, 6],
        ];

        for indexes in WIN_CONDITIONS {
            let symbols = indexes.map(|index| board[index]);
            if symbols[0].is_some() && symbols.windows(2).all(|symbols| symbols[0] == symbols[1]) {
                game_state.set(GameState::Winner);
                return;
            }
        }

        if board.iter().all(Option::is_some) {
            game_state.set(GameState::Tie);
        } else {
            current_turn.0 = current_turn.next();
        }
    }
}

/// Returns `true` if the local player can select cells.
fn local_player_turn(
    current_turn: Res<CurrentTurn>,
    client: Res<RepliconClient>,
    players: Query<(&Player, &Symbol)>,
) -> bool {
    let peer_id = client.peer_id().unwrap_or(PeerId::SERVER);
    players
        .iter()
        .any(|(player, &symbol)| player.0 == peer_id && symbol == current_turn.0)
}

/// A condition for systems to check if any component of type `T` was added to the world.
fn any_component_added<T: Component>(components: Query<(), Added<T>>) -> bool {
    !components.is_empty()
}

const PORT: u16 = 5000;

#[derive(Parser, PartialEq, Resource)]
enum Cli {
    Hotseat,
    Server {
        #[arg(short, long, default_value_t = PORT)]
        port: u16,

        #[arg(short, long, default_value_t = Symbol::Cross)]
        symbol: Symbol,
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

/// Font to display unicode characters for [`Symbol`].
#[derive(Resource)]
struct SymbolFont(Handle<Font>);

impl FromWorld for SymbolFont {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self(asset_server.load("NotoEmoji-Regular.ttf"))
    }
}

#[derive(States, Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
enum GameState {
    #[default]
    WaitingPlayer,
    InGame,
    Winner,
    Tie,
    Disconnected,
}

/// Contains symbol to be used this turn.
#[derive(Resource, Default, Deref)]
struct CurrentTurn(Symbol);

/// A component that defines the symbol of a player or a filled cell.
#[derive(Clone, Component, Copy, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
enum Symbol {
    #[default]
    Cross,
    Nought,
}

impl Symbol {
    fn glyph(self) -> &'static str {
        match self {
            Symbol::Cross => "❌",
            Symbol::Nought => "⭕",
        }
    }

    fn color(self) -> Color {
        match self {
            Symbol::Cross => Color::rgb(1.0, 0.5, 0.5),
            Symbol::Nought => Color::rgb(0.5, 0.5, 1.0),
        }
    }

    fn next(self) -> Self {
        match self {
            Symbol::Cross => Symbol::Nought,
            Symbol::Nought => Symbol::Cross,
        }
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::Cross => f.write_str("cross"),
            Symbol::Nought => f.write_str("nought"),
        }
    }
}

/// Marker for UI node with bottom text.
#[derive(Component)]
struct BottomText;

/// Marker for UI node with cells.
#[derive(Component)]
struct GridNode;

#[derive(Bundle)]
struct SymbolBundle {
    symbol: Symbol,
    cell_index: CellIndex,
    replication: Replication,
}

impl SymbolBundle {
    fn new(symbol: Symbol, index: usize) -> Self {
        Self {
            cell_index: CellIndex(index),
            symbol,
            replication: Replication,
        }
    }
}

/// Marks that the entity is a cell and contains its location in grid.
#[derive(Component, Deserialize, Serialize)]
struct CellIndex(usize);

/// Contains player ID and it's playing symbol.
#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    symbol: Symbol,
    replication: Replication,
}

impl PlayerBundle {
    fn new(peer_id: PeerId, symbol: Symbol) -> Self {
        Self {
            player: Player(peer_id),
            symbol,
            replication: Replication,
        }
    }

    /// Same as [`Self::new`], but with [`PeerId::SERVER`].
    fn server(symbol: Symbol) -> Self {
        Self::new(PeerId::SERVER, symbol)
    }
}

#[derive(Component, Serialize, Deserialize)]
struct Player(PeerId);

/// An event that indicates a symbol pick.
///
/// We don't replicate the whole UI, so we can't just send the picked entity because on server it may be different.
/// So we send the cell location in grid and calculate the entity on server based on this.
#[derive(Clone, Copy, Deserialize, Event, Serialize)]
struct CellPick(usize);
