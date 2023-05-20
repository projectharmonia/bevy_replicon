pub(super) mod despawn_tracker;
pub(super) mod removal_tracker;

use std::time::Duration;

use bevy::ecs::schedule::run_enter_schedule;
use bevy::{
    ecs::{
        archetype::ArchetypeId,
        component::{ComponentTicks, StorageType},
        system::SystemChangeTick,
        world::EntityRef,
    },
    prelude::*,
    reflect::TypeRegistryInternal,
    time::common_conditions::on_timer,
    utils::HashMap,
};
use bevy_renet::{
    renet::{
        transport::{NetcodeClientTransport, NetcodeServerTransport},
        RenetServer, ServerEvent,
    },
    transport::NetcodeServerPlugin,
    RenetServerPlugin,
};

use crate::{
    client::LastTick,
    replication_core::ReplicationRules,
    tick::Tick,
    world_diff::{ComponentDiff, WorldDiff, WorldDiffSerializer},
    REPLICATION_CHANNEL_ID,
};
use despawn_tracker::{DespawnTracker, DespawnTrackerPlugin};
use removal_tracker::{RemovalTracker, RemovalTrackerPlugin};

pub const SERVER_ID: u64 = 0;

pub struct ServerPlugin {
    /// Number of updates sent from server per second.
    ///
    /// By default it's 30 updates per second.
    pub tick_rate: u64,
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self { tick_rate: 30 }
    }
}

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(RenetServerPlugin)
            .add_plugin(NetcodeServerPlugin)
            .add_plugin(RemovalTrackerPlugin)
            .add_plugin(DespawnTrackerPlugin)
            .configure_set(
                ServerSet::Authority.run_if(not(resource_exists::<NetcodeClientTransport>())),
            )
            .init_resource::<AckedTicks>()
            .add_state::<ServerState>()
            .add_systems(
                (
                    Self::no_server_state_system
                        .run_if(state_exists_and_equals(ServerState::Hosting))
                        .run_if(resource_removed::<NetcodeServerTransport>()),
                    Self::hosting_state_system
                        .run_if(resource_added::<NetcodeServerTransport>())
                        .run_if(state_exists_and_equals(ServerState::NoServer)),
                )
                    .before(run_enter_schedule::<ServerState>)
                    .in_base_set(CoreSet::StateTransitions),
            )
            .add_systems(
                (
                    Self::tick_acks_receiving_system,
                    Self::acked_ticks_cleanup_system,
                )
                    .in_set(OnUpdate(ServerState::Hosting)),
            )
            .add_systems((
                Self::world_diffs_sending_system
                    .in_set(OnUpdate(ServerState::Hosting))
                    .in_set(ServerSet::Tick),
                Self::server_reset_system.in_schedule(OnExit(ServerState::Hosting)),
            ));

        // Remove delay for tests.
        if cfg!(not(test)) {
            let tick_time = Duration::from_millis(1000 / self.tick_rate);
            app.configure_set(ServerSet::Tick.run_if(on_timer(tick_time)));
        }
    }
}

impl ServerPlugin {
    fn no_server_state_system(mut server_state: ResMut<NextState<ServerState>>) {
        server_state.set(ServerState::NoServer);
    }

    fn hosting_state_system(mut server_state: ResMut<NextState<ServerState>>) {
        server_state.set(ServerState::Hosting);
    }

    fn server_reset_system(mut commands: Commands) {
        commands.insert_resource(AckedTicks::default());
    }

    fn world_diffs_sending_system(
        change_tick: SystemChangeTick,
        mut set: ParamSet<(&World, ResMut<RenetServer>)>,
        acked_ticks: Res<AckedTicks>,
        registry: Res<AppTypeRegistry>,
        replication_rules: Res<ReplicationRules>,
        despawn_tracker: Res<DespawnTracker>,
        removal_trackers: Query<(Entity, &RemovalTracker)>,
    ) {
        // Initialize [`WorldDiff`]s with latest acknowledged tick for each client.
        let registry = registry.read();
        let mut client_diffs: HashMap<_, _> = acked_ticks
            .iter()
            .map(|(&client_id, &last_tick)| (client_id, WorldDiff::new(last_tick)))
            .collect();
        collect_changes(&mut client_diffs, set.p0(), &registry, &replication_rules);
        collect_removals(&mut client_diffs, set.p0(), &change_tick, &removal_trackers);
        collect_despawns(&mut client_diffs, &change_tick, &despawn_tracker);

        let current_tick = set.p0().read_change_tick();
        for (client_id, mut world_diff) in client_diffs {
            world_diff.tick.set(current_tick); // Replace last acknowledged tick with the current.
            let serializer = WorldDiffSerializer::new(&world_diff, &registry);
            let message =
                bincode::serialize(&serializer).expect("world diff should be serializable");
            set.p1()
                .send_message(client_id, REPLICATION_CHANNEL_ID, message);
        }
    }

    fn tick_acks_receiving_system(
        mut acked_ticks: ResMut<AckedTicks>,
        mut server: ResMut<RenetServer>,
    ) {
        for client_id in server.clients_id() {
            let mut last_message = None;
            while let Some(message) = server.receive_message(client_id, REPLICATION_CHANNEL_ID) {
                last_message = Some(message);
            }

            if let Some(last_message) = last_message {
                match bincode::deserialize::<LastTick>(&last_message) {
                    Ok(tick) => {
                        acked_ticks.insert(client_id, tick.0);
                    }
                    Err(e) => error!("unable to deserialize tick from client {client_id}: {e}"),
                }
            }
        }
    }

    fn acked_ticks_cleanup_system(
        mut server_events: EventReader<ServerEvent>,
        mut acked_ticks: ResMut<AckedTicks>,
    ) {
        for event in &mut server_events {
            if let ServerEvent::ClientDisconnected {
                client_id: id,
                reason: _,
            } = event
            {
                acked_ticks.remove(id);
            }
        }
    }
}

fn collect_changes(
    client_diffs: &mut HashMap<u64, WorldDiff>,
    world: &World,
    registry: &TypeRegistryInternal,
    replication_rules: &ReplicationRules,
) {
    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.id() != ArchetypeId::EMPTY)
        .filter(|archetype| archetype.id() != ArchetypeId::INVALID)
        .filter(|archetype| replication_rules.is_replicated_archetype(archetype))
    {
        let table = world
            .storages()
            .tables
            .get(archetype.table_id())
            .expect("archetype should be in storage");

        for component_id in archetype.components().filter(|&component_id| {
            replication_rules.is_replicated_component(archetype, component_id)
        }) {
            let storage_type = archetype
                .get_storage_type(component_id)
                .expect("component should be a part of the archetype");

            // SAFETY: `component_id` obtained from the world.
            let component_info = unsafe { world.components().get_info_unchecked(component_id) };
            let type_name = component_info.name();
            let type_id = component_info
                .type_id()
                .unwrap_or_else(|| panic!("{type_name} should have registered TypeId"));
            let registration = registry
                .get(type_id)
                .unwrap_or_else(|| panic!("{type_name} should be registered"));
            let reflect_component = registration
                .data::<ReflectComponent>()
                .unwrap_or_else(|| panic!("{type_name} should have reflect(Component)"));

            match storage_type {
                StorageType::Table => {
                    let column = table
                        .get_column(component_id)
                        .unwrap_or_else(|| panic!("{type_name} should have a valid column"));

                    for archetype_entity in archetype.entities() {
                        // SAFETY: the table row obtained from the world state.
                        let ticks =
                            unsafe { column.get_ticks_unchecked(archetype_entity.table_row()) };
                        collect_if_changed(
                            client_diffs,
                            world.entity(archetype_entity.entity()),
                            world,
                            ticks,
                            reflect_component,
                            type_name,
                        );
                    }
                }
                StorageType::SparseSet => {
                    let sparse_set = world
                        .storages()
                        .sparse_sets
                        .get(component_id)
                        .unwrap_or_else(|| panic!("{type_name} should exists in a sparse set"));

                    for archetype_entity in archetype.entities() {
                        let ticks = sparse_set
                            .get_ticks(archetype_entity.entity())
                            .expect("{type_name} should have ticks");
                        collect_if_changed(
                            client_diffs,
                            world.entity(archetype_entity.entity()),
                            world,
                            ticks,
                            reflect_component,
                            type_name,
                        );
                    }
                }
            }
        }
    }
}

fn collect_if_changed(
    client_diffs: &mut HashMap<u64, WorldDiff>,
    entity: EntityRef,
    world: &World,
    ticks: ComponentTicks,
    reflect_component: &ReflectComponent,
    type_name: &str,
) {
    for world_diff in client_diffs.values_mut() {
        if ticks.is_changed(world_diff.tick.get(), world.read_change_tick()) {
            let component = reflect_component
                .reflect(entity)
                .unwrap_or_else(|| panic!("entity should have {type_name}"))
                .clone_value();
            world_diff
                .entities
                .entry(entity.id())
                .or_default()
                .push(ComponentDiff::Changed(component));
        }
    }
}

fn collect_removals(
    client_diffs: &mut HashMap<u64, WorldDiff>,
    world: &World,
    change_tick: &SystemChangeTick,
    removal_trackers: &Query<(Entity, &RemovalTracker)>,
) {
    for (entity, removal_tracker) in removal_trackers {
        for world_diff in client_diffs.values_mut() {
            for (&component_id, &tick) in removal_tracker.iter() {
                if tick.is_newer_than(world_diff.tick, Tick::new(change_tick.change_tick())) {
                    // SAFETY: `component_id` obtained from `RemovalTracker` that always contains valid components.
                    let component_info =
                        unsafe { world.components().get_info_unchecked(component_id) };
                    world_diff
                        .entities
                        .entry(entity)
                        .or_default()
                        .push(ComponentDiff::Removed(component_info.name().to_string()));
                }
            }
        }
    }
}

fn collect_despawns(
    client_diffs: &mut HashMap<u64, WorldDiff>,
    change_tick: &SystemChangeTick,
    despawn_tracker: &DespawnTracker,
) {
    for (entity, tick) in despawn_tracker.despawns.iter().copied() {
        for world_diff in client_diffs.values_mut() {
            if tick.is_newer_than(world_diff.tick, Tick::new(change_tick.change_tick())) {
                world_diff.despawns.push(entity);
            }
        }
    }
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ServerSet {
    /// Runs with server or in singleplayer.
    Authority,
    /// Runs on server tick.
    Tick,
}

#[derive(States, Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
pub enum ServerState {
    #[default]
    NoServer,
    Hosting,
}

/// Last acknowledged server ticks from all clients.
///
/// Used only on server.
#[derive(Default, Deref, DerefMut, Resource)]
pub(super) struct AckedTicks(HashMap<u64, Tick>);
