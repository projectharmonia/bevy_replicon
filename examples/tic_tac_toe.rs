//! A game to showcase single-player and multiplier game.
//! Run it with `--hotseat` to play locally or with `--client` / `--server`

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use anyhow::Result;
use bevy::prelude::*;
use bevy_replicon::{
    prelude::*,
    renet::{
        ClientAuthentication, RenetConnectionConfig, ServerAuthentication, ServerConfig,
        ServerEvent,
    },
};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use strum::Display;

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins(DefaultPlugins.build().set(WindowPlugin {
            primary_window: Some(Window {
                title: "Tic-Tac-Toe".into(),
                resolution: (800.0, 600.0).into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .add_plugins(ReplicationPlugins)
        .add_plugin(TicTacToePlugin)
        .run();
}

struct TicTacToePlugin;

impl Plugin for TicTacToePlugin {
    fn build(&self, app: &mut App) {
        app.add_state::<GameState>()
            .init_resource::<GameFont>()
            .init_resource::<SymbolFont>()
            .init_resource::<CurrentTurn>()
            .replicate::<Symbol>()
            .replicate::<BoardCell>()
            .replicate::<Player>()
            .add_client_event::<CellPick>()
            .insert_resource(ClearColor(BACKGROUND_COLOR))
            .add_startup_systems((
                Self::ui_setup_system,
                Self::cli_system.pipe(system_adapter::unwrap),
            ))
            .add_systems((
                Self::connecting_text_system.in_schedule(OnEnter(ClientState::Connecting)),
                Self::server_waiting_text_system.in_schedule(OnEnter(ServerState::Hosting)),
                Self::client_disconnected_text_system.in_schedule(OnEnter(GameState::Disconnected)),
                Self::winner_text_system.in_schedule(OnEnter(GameState::Winner)),
                Self::tie_text_system.in_schedule(OnEnter(GameState::Tie)),
                Self::server_event_system.in_set(OnUpdate(ServerState::Hosting)),
                Self::start_game_system
                    .in_set(OnUpdate(ClientState::Connected))
                    .in_set(OnUpdate(GameState::WaitingForPlayer))
                    .run_if(any_component_added::<Player>), // Wait until client replicates players before starting the game.
            ))
            .add_systems(
                (Self::turn_text_system, Self::symbol_turn_text_system)
                    .in_schedule(OnEnter(GameState::InGame)),
            )
            .add_systems(
                (
                    Self::picking_system.in_set(ServerSet::Authority),
                    Self::symbol_init_system,
                    Self::turn_advance_system.run_if(any_component_added::<BoardCell>),
                    Self::cell_interatction_system.run_if(Self::local_player_turn),
                    Self::symbol_turn_text_system.run_if(resource_changed::<CurrentTurn>()),
                )
                    .in_set(OnUpdate(GameState::InGame)),
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
    fn ui_setup_system(
        mut commands: Commands,
        game_font: Res<GameFont>,
        symbol_font: Res<SymbolFont>,
    ) {
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
        const FONT_SIZE: f32 = 50.0;

        commands
            .spawn(NodeBundle {
                style: Style {
                    size: Size::all(Val::Percent(100.0)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_children(|parent| {
                parent
                    .spawn((
                        NodeBundle {
                            style: Style {
                                flex_direction: FlexDirection::Column,
                                size: Size::all(Val::Px(BOARD_SIZE - LINE_THICKNESS)),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        GridNode,
                    ))
                    .with_children(|parent| {
                        for _ in 0..GRID_SIZE {
                            parent
                                .spawn(NodeBundle {
                                    style: Style {
                                        size: Size::new(
                                            Val::Px(BOARD_SIZE - LINE_THICKNESS),
                                            Val::Px(CELL_SIZE + LINE_THICKNESS),
                                        ),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })
                                .with_children(|parent| {
                                    for _ in 0..GRID_SIZE {
                                        parent.spawn(ButtonBundle {
                                            style: Style {
                                                size: Size::all(Val::Px(BUTTON_SIZE)),
                                                margin: UiRect::all(Val::Px(BUTTON_MARGIN)),
                                                ..Default::default()
                                            },
                                            background_color: BACKGROUND_COLOR.into(),
                                            ..Default::default()
                                        });
                                    }
                                });
                        }

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
                                                font: game_font.0.clone(),
                                                font_size: FONT_SIZE,
                                                color: TEXT_COLOR,
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
        settings: Res<Cli>,
        network_channels: Res<NetworkChannels>,
    ) -> Result<()> {
        match *settings {
            Cli::Hotseat => {
                // Set all players to server to play from a single machine and start the game right away.
                commands.spawn(PlayerBundle::server(Symbol::Cross));
                commands.spawn(PlayerBundle::server(Symbol::Nought));
                game_state.set(GameState::InGame);
            }
            Cli::Server { port, symbol } => {
                let send_channels_config = network_channels.server_channels();
                let receive_channels_config = network_channels.client_channels();
                const MAX_CLIENTS: usize = 1;
                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let server_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
                let socket = UdpSocket::bind(server_addr)?;
                let server_config = ServerConfig::new(
                    MAX_CLIENTS,
                    PROTOCOL_ID,
                    server_addr,
                    ServerAuthentication::Unsecure,
                );

                let connection_config = RenetConnectionConfig {
                    send_channels_config,
                    receive_channels_config,
                    ..Default::default()
                };

                let server =
                    RenetServer::new(current_time, server_config, connection_config, socket)?;

                commands.insert_resource(server);
                commands.spawn(PlayerBundle::server(symbol));
            }
            Cli::Client { port, ip } => {
                let receive_channels_config = network_channels.server_channels();
                let send_channels_config = network_channels.client_channels();
                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let client_id = current_time.as_millis() as u64;
                let server_addr = SocketAddr::new(ip, port);
                let socket = UdpSocket::bind((ip, 0)).expect("localhost should be bindable");
                let authentication = ClientAuthentication::Unsecure {
                    client_id,
                    protocol_id: PROTOCOL_ID,
                    server_addr,
                    user_data: None,
                };

                let connection_config = RenetConnectionConfig {
                    send_channels_config,
                    receive_channels_config,
                    ..Default::default()
                };

                let client =
                    RenetClient::new(current_time, socket, connection_config, authentication)?;
                commands.insert_resource(client);
            }
        }

        Ok(())
    }

    /// Waits for client to connect to start the game or disconnect to finish it.
    ///
    /// Only for server.
    fn server_event_system(
        mut commands: Commands,
        mut server_event: EventReader<ServerEvent>,
        mut game_state: ResMut<NextState<GameState>>,
        players: Query<&Symbol, With<Player>>,
    ) {
        for event in server_event.iter() {
            match event {
                ServerEvent::ClientConnected(client_id, _) => {
                    let server_symbol = players.single();
                    commands.spawn(PlayerBundle::new(*client_id, server_symbol.next()));
                    game_state.set(GameState::InGame);
                }
                ServerEvent::ClientDisconnected(_) => {
                    game_state.set(GameState::Disconnected);
                }
            }
        }
    }

    fn start_game_system(mut game_state: ResMut<NextState<GameState>>) {
        game_state.set(GameState::InGame);
    }

    fn cell_interatction_system(
        mut cells: Query<
            (Entity, &Parent, &Interaction, &mut BackgroundColor),
            Changed<Interaction>,
        >,
        children: Query<&Children>,
        parent: Query<&Parent>,
        mut pick_events: EventWriter<CellPick>,
    ) {
        const HOVER_COLOR: Color = Color::rgb(0.85, 0.85, 0.85);

        for (button_entity, button_parent, interaction, mut background) in &mut cells {
            match interaction {
                Interaction::Clicked => {
                    let buttons = children.get(**button_parent).unwrap();
                    let column = buttons
                        .iter()
                        .position(|&entity| entity == button_entity)
                        .unwrap();

                    let row_parent = parent
                        .get(**button_parent)
                        .expect("column node should be a parent of row node");
                    let rows = children.get(**row_parent).unwrap();
                    let row = rows
                        .iter()
                        .position(|&entity| entity == **button_parent)
                        .unwrap();
                    // We send a pick event and wait for the pick to be replicated back to the client.
                    // In case of server or single-player the event will re-translated into [`FromClient`] event to re-use the logic.
                    pick_events.send(CellPick(BoardCell { row, column }));
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
        mut pick_events: EventReader<FromClient<CellPick>>,
        cells: Query<&BoardCell>,
        current_turn: Res<CurrentTurn>,
        players: Query<(&Player, &Symbol)>,
    ) {
        for FromClient { client_id, event } in pick_events.iter().copied() {
            // It's good to check the received data, client could be cheating.
            if event.column > GRID_SIZE || event.row > GRID_SIZE {
                error!("received invalid cell {:?}", event.0);
                continue;
            }

            if !players
                .iter()
                .any(|(player, &symbol)| player.0 == client_id && symbol == current_turn.0)
            {
                error!("player {client_id} chose cell {:?} at wrong turn", event.0);
                continue;
            }

            if cells.iter().any(|&cell| cell == event.0) {
                error!(
                    "player {client_id} has chosen an already occupied cell {:?}",
                    event.0
                );
                continue;
            }

            // Spawn "blueprint" of the cell that client will replicate.
            commands.spawn(BoardCellBundle::new(event.0, current_turn.0));
        }
    }

    /// Initializes spawned symbol on client after replication and on server / single-player right after the spawn.
    fn symbol_init_system(
        mut commands: Commands,
        symbol_font: Res<SymbolFont>,
        cells: Query<(Entity, &Symbol, &BoardCell), Added<Symbol>>,
        grid_nodes: Query<&Children, With<GridNode>>,
        children: Query<&Children>,
        mut background_colors: Query<&mut BackgroundColor>,
    ) {
        for (cell_entity, symbol, cell) in &cells {
            let rows = grid_nodes.single();
            let row_entity = rows[cell.row];
            let buttons = children
                .get(row_entity)
                .expect("rows should have buttons as children");
            let button_entity = buttons[cell.column];

            let mut background = background_colors
                .get_mut(button_entity)
                .expect("buttons should be initialized with color");
            *background = BACKGROUND_COLOR.into();

            commands
                .entity(button_entity)
                .remove::<Interaction>()
                .add_child(cell_entity);

            commands
                .entity(cell_entity)
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
        cells: Query<(&BoardCell, &Symbol)>,
    ) {
        let mut board = [None; GRID_SIZE * GRID_SIZE];
        for (cell, &symbol) in &cells {
            board[cell.row * GRID_SIZE + cell.column] = Some(symbol);
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

    fn connecting_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[TEXT_SECTION].value = "Connecting".into();
    }

    fn server_waiting_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[TEXT_SECTION].value = "Waiting for player".into();
    }

    fn client_disconnected_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        let sections = &mut bottom_text.single_mut().sections;
        sections[TEXT_SECTION].value = "Client disconnected".into();
        sections[SYMBOL_SECTION].value.clear();
    }

    fn turn_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[0].value = "Current turn: ".into();
    }

    fn symbol_turn_text_system(
        mut bottom_text: Query<&mut Text, With<BottomText>>,
        current_turn: Res<CurrentTurn>,
    ) {
        let mut symbol_section = &mut bottom_text.single_mut().sections[SYMBOL_SECTION];
        symbol_section.value = current_turn.glyph().into();
        symbol_section.style.color = current_turn.color();
    }

    fn winner_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        bottom_text.single_mut().sections[TEXT_SECTION].value = "Winner: ".into();
    }

    fn tie_text_system(mut bottom_text: Query<&mut Text, With<BottomText>>) {
        let sections = &mut bottom_text.single_mut().sections;
        sections[TEXT_SECTION].value = "Tie".into();
        sections[SYMBOL_SECTION].value.clear();
    }

    /// Returns `true` if the local player can select cells.
    fn local_player_turn(
        current_turn: Res<CurrentTurn>,
        client: Option<Res<RenetClient>>,
        players: Query<(&Player, &Symbol)>,
    ) -> bool {
        let client_id = client.map(|client| client.client_id()).unwrap_or(SERVER_ID);
        players
            .iter()
            .any(|(player, &symbol)| player.0 == client_id && symbol == current_turn.0)
    }
}

/// A condition for systems to check if any component of type `T` was added to the world.
fn any_component_added<T: Component>(components: Query<(), Added<T>>) -> bool {
    !components.is_empty()
}

const PORT: u16 = 4761;

#[derive(Debug, Parser, PartialEq, Resource)]
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

/// Font for in-game text.
#[derive(Resource)]
struct GameFont(Handle<Font>);

impl FromWorld for GameFont {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self(asset_server.load("FiraSans-Bold.ttf"))
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
    WaitingForPlayer,
    InGame,
    Winner,
    Tie,
    Disconnected,
}

/// Contains symbol to be used this turn.
#[derive(Resource, Default, Deref)]
struct CurrentTurn(Symbol);

/// A component that defines the symbol of a player or a filled cell.
#[derive(
    Clone,
    Component,
    Copy,
    Debug,
    Default,
    Deserialize,
    Display,
    Eq,
    Hash,
    PartialEq,
    Serialize,
    ValueEnum,
    Reflect,
)]
#[strum(serialize_all = "kebab-case")]
#[reflect(Component)]
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

/// Marker for UI node with bottom text.
#[derive(Component)]
struct BottomText;

/// Marker for UI node with cells.
#[derive(Component)]
struct GridNode;

#[derive(Bundle)]
struct BoardCellBundle {
    cell: BoardCell,
    symbol: Symbol,
    replication: Replication,
}

impl BoardCellBundle {
    fn new(cell: BoardCell, symbol: Symbol) -> Self {
        Self {
            cell,
            symbol,
            replication: Replication,
        }
    }
}

/// Marks that the entity is a cell and contains its location in grid.
#[derive(Clone, Component, Copy, Debug, Default, Deserialize, PartialEq, Reflect, Serialize)]
#[reflect(Component)]
struct BoardCell {
    row: usize,
    column: usize,
}

/// Contains player ID and it's playing symbol.
#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    symbol: Symbol,
    replication: Replication,
}

impl PlayerBundle {
    fn new(id: u64, symbol: Symbol) -> Self {
        Self {
            player: Player(id),
            symbol,
            replication: Replication,
        }
    }

    /// Same as [`Self::new`], but with [`SERVER_ID`].
    fn server(symbol: Symbol) -> Self {
        Self::new(SERVER_ID, symbol)
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Player(u64);

/// An event that indicates a symbol pick.
///
/// We don't replicate the whole UI, so we can't just send the picked entity because on server it may be different.
/// So we send the cell location in grid and calculate the entity on server based on this.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Deref)]
struct CellPick(BoardCell);
