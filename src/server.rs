pub(super) mod despawn_tracker;
pub(super) mod removal_tracker;

use std::time::Duration;

use bevy::{
    ecs::{
        archetype::ArchetypeId,
        component::{ComponentTicks, StorageType, Tick},
        system::SystemChangeTick,
        world::EntityRef,
    },
    prelude::*,
    reflect::TypeRegistryInternal,
    time::common_conditions::on_timer,
    utils::HashMap,
};
use bevy_renet::{
    renet::{RenetClient, RenetServer, ServerEvent},
    transport::NetcodeServerPlugin,
    RenetServerPlugin,
};

use crate::{
    client::LastTick,
    replication_core::{ReplicationRules, REPLICATION_CHANNEL_ID},
    world_diff::{ComponentDiff, WorldDiff, WorldDiffSerializer},
};
use despawn_tracker::{DespawnTracker, DespawnTrackerPlugin};
use removal_tracker::{RemovalTracker, RemovalTrackerPlugin};

pub const SERVER_ID: u64 = 0;

pub struct ServerPlugin {
    pub tick_policy: TickPolicy,
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self {
            tick_policy: if cfg!(test) {
                // Remove delay for tests.
                TickPolicy::Manual
            } else {
                TickPolicy::MaxTickRate(30)
            },
        }
    }
}

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RenetServerPlugin,
            NetcodeServerPlugin,
            RemovalTrackerPlugin,
            DespawnTrackerPlugin,
        ))
        .init_resource::<AckedTicks>()
        .configure_set(
            PreUpdate,
            ServerSet::Receive.after(NetcodeServerPlugin::update_system),
        )
        .configure_set(
            PostUpdate,
            ServerSet::Send.before(NetcodeServerPlugin::send_packets),
        )
        .add_systems(
            PreUpdate,
            (Self::acks_receiving_system, Self::acks_cleanup_system)
                .in_set(ServerSet::Receive)
                .run_if(resource_exists::<RenetServer>()),
        )
        .add_systems(
            PostUpdate,
            (
                Self::diffs_sending_system
                    .in_set(ServerSet::Send)
                    .run_if(resource_exists::<RenetServer>()),
                Self::reset_system.run_if(resource_removed::<RenetServer>()),
            ),
        );

        if let TickPolicy::MaxTickRate(max_tick_rate) = self.tick_policy {
            let tick_time = Duration::from_millis(1000 / max_tick_rate as u64);
            app.configure_set(PostUpdate, ServerSet::Send.run_if(on_timer(tick_time)));
        }
    }
}

impl ServerPlugin {
    fn acks_receiving_system(mut acked_ticks: ResMut<AckedTicks>, mut server: ResMut<RenetServer>) {
        for client_id in server.clients_id() {
            let mut last_message = None;
            while let Some(message) = server.receive_message(client_id, REPLICATION_CHANNEL_ID) {
                last_message = Some(message);
            }

            if let Some(last_message) = last_message {
                match bincode::deserialize::<LastTick>(&last_message) {
                    Ok(tick) => {
                        acked_ticks.insert(client_id, tick.into());
                    }
                    Err(e) => error!("unable to deserialize tick from client {client_id}: {e}"),
                }
            }
        }
    }

    fn acks_cleanup_system(
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

    fn diffs_sending_system(
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
            world_diff.tick = current_tick; // Replace last acknowledged tick with the current.
            let serializer = WorldDiffSerializer::new(&world_diff, &registry);
            let message =
                bincode::serialize(&serializer).expect("world diff should be serializable");
            set.p1()
                .send_message(client_id, REPLICATION_CHANNEL_ID, message);
        }
    }

    fn reset_system(mut acked_ticks: ResMut<AckedTicks>) {
        acked_ticks.clear();
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
        if ticks.is_changed(world_diff.tick, world.read_change_tick()) {
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
                if tick.is_newer_than(world_diff.tick, change_tick.this_run()) {
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
            if tick.is_newer_than(world_diff.tick, change_tick.this_run()) {
                world_diff.despawns.push(entity);
            }
        }
    }
}

/// Condition that returns `true` if server is present or in singleplayer.
pub fn has_authority() -> impl FnMut(Option<Res<RenetClient>>) -> bool + Clone {
    move |client| client.is_none()
}

/// Set with replication and event systems related to server.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ServerSet {
    /// Systems that receive data.
    ///
    /// Runs in `PreUpdate`.
    Receive,
    /// Systems that send data.
    ///
    /// Runs in `PostUpdate` on server tick, see [`TickPolicy`].
    Send,
}

pub enum TickPolicy {
    /// Max number of updates sent from server per second. May be lower if update cycle duration is too long.
    ///
    /// By default it's 30 updates per second.
    MaxTickRate(u16),
    /// [`ServerSet::Send`] should be manually configured.
    Manual,
}

/// Last acknowledged server ticks from all clients.
///
/// Used only on server.
#[derive(Default, Deref, DerefMut, Resource)]
pub(super) struct AckedTicks(HashMap<u64, Tick>);
