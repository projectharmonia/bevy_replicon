pub(super) mod despawn_tracker;
pub(super) mod removal_tracker;

use std::time::Duration;

use bevy::{
    ecs::{
        archetype::{Archetype, ArchetypeId},
        component::{ComponentId, StorageType, Tick},
        storage::Table,
        system::SystemChangeTick,
    },
    prelude::*,
    time::common_conditions::on_timer,
    utils::HashMap,
};
use bevy_renet::{
    renet::{RenetClient, RenetServer, ServerEvent},
    transport::NetcodeServerPlugin,
    RenetServerPlugin,
};
use derive_more::Constructor;

use crate::{
    client::LastTick,
    replicon_core::{
        ComponentDiff, ReplicationId, ReplicationRules, WorldDiff, REPLICATION_CHANNEL_ID,
    },
};
use despawn_tracker::{DespawnTracker, DespawnTrackerPlugin};
use removal_tracker::{RemovalTracker, RemovalTrackerPlugin};

pub const SERVER_ID: u64 = 0;

#[derive(Constructor)]
pub struct ServerPlugin {
    tick_policy: TickPolicy,
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self {
            tick_policy: TickPolicy::MaxTickRate(30),
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
                        acked_ticks.0.insert(client_id, tick.into());
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
                acked_ticks.0.remove(id);
            }
        }
    }

    fn diffs_sending_system(
        change_tick: SystemChangeTick,
        mut set: ParamSet<(&World, ResMut<RenetServer>)>,
        acked_ticks: Res<AckedTicks>,
        replication_rules: Res<ReplicationRules>,
        despawn_tracker: Res<DespawnTracker>,
        removal_trackers: Query<(Entity, &RemovalTracker)>,
    ) {
        let current_tick = set.p0().read_change_tick();

        // Initialize [`WorldDiff`]s with latest acknowledged tick for each client.
        let mut client_diffs: Vec<_> = acked_ticks
            .iter()
            .map(|(&client_id, &last_tick)| (client_id, WorldDiff::new(last_tick)))
            .collect();
        collect_changes(&mut client_diffs, set.p0(), &replication_rules);
        collect_removals(&mut client_diffs, &change_tick, &removal_trackers);
        collect_despawns(&mut client_diffs, &change_tick, &despawn_tracker);

        let mut messages = Vec::with_capacity(client_diffs.len());
        for (client_id, mut world_diff) in client_diffs {
            world_diff.tick = current_tick; // Replace last acknowledged tick with the current.
            let mut message = Vec::new();
            world_diff
                .serialize(&replication_rules, &mut message)
                .expect("world diff should be serializable");
            messages.push((client_id, message));
        }

        for (client_id, message) in messages {
            set.p1()
                .send_message(client_id, REPLICATION_CHANNEL_ID, message);
        }
    }

    fn reset_system(mut acked_ticks: ResMut<AckedTicks>) {
        acked_ticks.0.clear();
    }
}

fn collect_changes<'a>(
    client_diffs: &mut [(u64, WorldDiff<'a>)],
    world: &'a World,
    replication_rules: &ReplicationRules,
) {
    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.id() != ArchetypeId::EMPTY)
        .filter(|archetype| archetype.id() != ArchetypeId::INVALID)
        .filter(|archetype| archetype.contains(replication_rules.replication_id()))
    {
        let table = world
            .storages()
            .tables
            .get(archetype.table_id())
            .expect("archetype should be valid");

        for component_id in archetype.components() {
            let Some(replication_id) = replication_rules.get_id(component_id) else {
                continue;
            };
            let replication_info = replication_rules.get_info(replication_id);
            if archetype.contains(replication_info.ignored_id) {
                continue;
            }

            let storage_type = archetype
                .get_storage_type(component_id)
                .unwrap_or_else(|| panic!("{component_id:?} be in archetype"));

            match storage_type {
                StorageType::Table => {
                    collect_table_components(
                        client_diffs,
                        world,
                        table,
                        archetype,
                        replication_id,
                        component_id,
                    );
                }
                StorageType::SparseSet => {
                    collect_sparse_set_components(
                        client_diffs,
                        world,
                        archetype,
                        replication_id,
                        component_id,
                    );
                }
            }
        }
    }
}

fn collect_table_components<'a>(
    client_diffs: &mut [(u64, WorldDiff<'a>)],
    world: &World,
    table: &'a Table,
    archetype: &Archetype,
    replication_id: ReplicationId,
    component_id: ComponentId,
) {
    let column = table
        .get_column(component_id)
        .unwrap_or_else(|| panic!("{component_id:?} should belong to table"));

    for archetype_entity in archetype.entities() {
        // SAFETY: the table row obtained from the world state.
        let ticks = unsafe { column.get_ticks_unchecked(archetype_entity.table_row()) };
        // SAFETY: component obtained from the archetype.
        let component = unsafe { column.get_data_unchecked(archetype_entity.table_row()) };

        for (_, world_diff) in &mut *client_diffs {
            if ticks.is_changed(world_diff.tick, world.read_change_tick()) {
                world_diff
                    .entities
                    .entry(archetype_entity.entity())
                    .or_default()
                    .push(ComponentDiff::Changed((replication_id, component)));
            }
        }
    }
}

fn collect_sparse_set_components<'a>(
    client_diffs: &mut [(u64, WorldDiff<'a>)],
    world: &'a World,
    archetype: &Archetype,
    replication_id: ReplicationId,
    component_id: ComponentId,
) {
    let sparse_set = world
        .storages()
        .sparse_sets
        .get(component_id)
        .unwrap_or_else(|| panic!("{component_id:?} should belong to sparse set"));

    for archetype_entity in archetype.entities() {
        let entity = archetype_entity.entity();
        let ticks = sparse_set
            .get_ticks(entity)
            .unwrap_or_else(|| panic!("{entity:?} should have {component_id:?}"));
        let component = sparse_set
            .get(entity)
            .unwrap_or_else(|| panic!("{entity:?} should have {component_id:?}"));

        for (_, world_diff) in &mut *client_diffs {
            if ticks.is_changed(world_diff.tick, world.read_change_tick()) {
                world_diff
                    .entities
                    .entry(entity)
                    .or_default()
                    .push(ComponentDiff::Changed((replication_id, component)));
            }
        }
    }
}

fn collect_removals(
    client_diffs: &mut [(u64, WorldDiff)],
    change_tick: &SystemChangeTick,
    removal_trackers: &Query<(Entity, &RemovalTracker)>,
) {
    for (entity, removal_tracker) in removal_trackers {
        for (_, world_diff) in &mut *client_diffs {
            for (&replication_id, &tick) in removal_tracker.iter() {
                if tick.is_newer_than(world_diff.tick, change_tick.this_run()) {
                    world_diff
                        .entities
                        .entry(entity)
                        .or_default()
                        .push(ComponentDiff::Removed(replication_id));
                }
            }
        }
    }
}

fn collect_despawns(
    client_diffs: &mut [(u64, WorldDiff)],
    change_tick: &SystemChangeTick,
    despawn_tracker: &DespawnTracker,
) {
    for (entity, tick) in despawn_tracker.despawns.iter().copied() {
        for (_, world_diff) in &mut *client_diffs {
            if tick.is_newer_than(world_diff.tick, change_tick.this_run()) {
                world_diff.despawns.push(entity);
            }
        }
    }
}

/// Condition that returns `true` for server or in singleplayer and `false` for client.
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
#[derive(Default, Deref, Resource)]
pub struct AckedTicks(pub(super) HashMap<u64, Tick>);
