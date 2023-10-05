pub(super) mod despawn_tracker;
pub(super) mod prediction_tracker;
pub(super) mod removal_tracker;
pub(super) mod replication_buffer;

use std::time::Duration;

use bevy::{
    ecs::{
        archetype::ArchetypeId,
        component::{StorageType, Tick},
        system::SystemChangeTick,
    },
    prelude::*,
    scene::DynamicEntity,
    time::common_conditions::on_timer,
    utils::HashMap,
};
use bevy_renet::{
    renet::{RenetClient, RenetServer, ServerEvent},
    transport::NetcodeServerPlugin,
    RenetServerPlugin,
};

use crate::replicon_core::{
    replication_rules::ReplicationRules, replicon_tick::RepliconTick, REPLICATION_CHANNEL_ID,
};
use despawn_tracker::{DespawnTracker, DespawnTrackerPlugin};
use prediction_tracker::PredictionTracker;
use removal_tracker::{RemovalTracker, RemovalTrackerPlugin};
use replication_buffer::ReplicationBuffer;

pub const SERVER_ID: u64 = 0;

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
        .init_resource::<RepliconTick>()
        .init_resource::<PredictionTracker>()
        .configure_set(
            PreUpdate,
            ServerSet::Receive.after(NetcodeServerPlugin::update_system),
        )
        .configure_set(
            PostUpdate,
            ServerSet::Send
                .before(NetcodeServerPlugin::send_packets)
                .run_if(resource_changed::<RepliconTick>()),
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
                    .pipe(unwrap)
                    .in_set(ServerSet::Send)
                    .run_if(resource_exists::<RenetServer>()),
                Self::reset_system.run_if(resource_removed::<RenetServer>()),
            ),
        );

        match self.tick_policy {
            TickPolicy::MaxTickRate(max_tick_rate) => {
                let tick_time = Duration::from_millis(1000 / max_tick_rate as u64);
                app.add_systems(
                    PostUpdate,
                    Self::increment_tick
                        .before(Self::diffs_sending_system)
                        .run_if(on_timer(tick_time)),
                );
            }
            TickPolicy::EveryFrame => {
                app.add_systems(
                    PostUpdate,
                    Self::increment_tick.before(Self::diffs_sending_system),
                );
            }
            TickPolicy::Manual => (),
        }
    }
}

impl ServerPlugin {
    pub fn new(tick_policy: TickPolicy) -> Self {
        Self { tick_policy }
    }

    /// Increments current server tick which causes the server to send a diff packet this frame.
    pub fn increment_tick(mut tick: ResMut<RepliconTick>) {
        tick.increment();
    }

    fn acks_receiving_system(
        mut acked_ticks: ResMut<AckedTicks>,
        mut server: ResMut<RenetServer>,
        mut predictions: ResMut<PredictionTracker>,
    ) {
        for client_id in server.clients_id() {
            while let Some(message) = server.receive_message(client_id, REPLICATION_CHANNEL_ID) {
                match bincode::deserialize::<RepliconTick>(&message) {
                    Ok(tick) => {
                        let acked_tick = acked_ticks.clients.entry(client_id).or_default();
                        if *acked_tick < tick {
                            *acked_tick = tick;
                            predictions.cleanup_acked(client_id, *acked_tick);
                        }
                    }
                    Err(e) => error!("unable to deserialize tick from client {client_id}: {e}"),
                }
            }
        }
        acked_ticks.cleanup_system_ticks();
    }

    fn acks_cleanup_system(
        mut server_events: EventReader<ServerEvent>,
        mut acked_ticks: ResMut<AckedTicks>,
    ) {
        for event in &mut server_events {
            match event {
                ServerEvent::ClientDisconnected { client_id, .. } => {
                    acked_ticks.clients.remove(client_id);
                }
                ServerEvent::ClientConnected { client_id } => {
                    acked_ticks.clients.entry(*client_id).or_default();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn diffs_sending_system(
        mut buffers: Local<Vec<ReplicationBuffer>>,
        change_tick: SystemChangeTick,
        mut set: ParamSet<(&World, ResMut<RenetServer>, ResMut<AckedTicks>)>,
        replication_rules: Res<ReplicationRules>,
        despawn_tracker: Res<DespawnTracker>,
        replicon_tick: Res<RepliconTick>,
        removal_trackers: Query<(Entity, &RemovalTracker)>,
        predictions: Res<PredictionTracker>,
    ) -> Result<(), bincode::Error> {
        let mut acked_ticks = set.p2();
        acked_ticks.register_tick(*replicon_tick, change_tick.this_run());

        let buffers = prepare_buffers(&mut buffers, &acked_ticks, *replicon_tick)?;

        collect_mappings(buffers, &acked_ticks, &predictions)?;
        collect_changes(
            buffers,
            set.p0(),
            change_tick.this_run(),
            &replication_rules,
        )?;
        collect_removals(buffers, &removal_trackers, change_tick.this_run())?;
        collect_despawns(buffers, &despawn_tracker, change_tick.this_run())?;

        for buffer in buffers {
            buffer.send_to(&mut set.p1(), REPLICATION_CHANNEL_ID);
        }

        Ok(())
    }

    fn reset_system(mut acked_ticks: ResMut<AckedTicks>) {
        acked_ticks.clients.clear();
        acked_ticks.system_ticks.clear();
    }
}

/// Initializes buffer for each client and returns it as mutable slice.
///
/// Reuses already allocated buffers.
/// Creates new buffers if number of clients is bigger then the number of allocated buffers.
/// If there are more buffers than the number of clients, then the extra buffers remain untouched
/// and the returned slice will not include them.
fn prepare_buffers<'a>(
    buffers: &'a mut Vec<ReplicationBuffer>,
    acked_ticks: &AckedTicks,
    replicon_tick: RepliconTick,
) -> Result<&'a mut [ReplicationBuffer], bincode::Error> {
    buffers.reserve(acked_ticks.clients.len());
    for (index, (&client_id, &tick)) in acked_ticks.clients.iter().enumerate() {
        let system_tick = *acked_ticks.system_ticks.get(&tick).unwrap_or(&Tick::new(0));

        if let Some(buffer) = buffers.get_mut(index) {
            buffer.reset(client_id, system_tick, replicon_tick)?;
        } else {
            buffers.push(ReplicationBuffer::new(
                client_id,
                system_tick,
                replicon_tick,
            )?);
        }
    }

    Ok(&mut buffers[..acked_ticks.clients.len()])
}

/// Collect and write any new entity mappings into buffers since last acknowledged tick
fn collect_mappings(
    buffers: &mut [ReplicationBuffer],
    acked_ticks: &ResMut<AckedTicks>,
    predictions: &Res<PredictionTracker>,
) -> Result<(), bincode::Error> {
    for buffer in &mut *buffers {
        // Include all entity mappings since the last acknowledged tick.
        //
        // if the spawn command for a mapped client entity was lost, a mapped component on another
        // entity could arrive first, referencing the mapped entity, so we include all until acked.
        //
        // could this grow very large? probably not, since mappings only get created in response to
        // clients sending specific types of command to the server, and if there is any packet loss
        // resulting in a larger unacked backlog, it's unlikely the server received those commands
        // anyway, so won't have created more mappings during the packet loss.
        let acked_tick = acked_ticks
            .acked_ticks()
            .get(&buffer.client_id())
            .unwrap_or(&RepliconTick(0));
        let mappings = predictions.get_mappings(buffer.client_id(), *acked_tick);
        buffer.write_entity_mappings(mappings)?;
    }
    Ok(())
}

/// Collect component changes into buffers based on last acknowledged tick.
fn collect_changes(
    buffers: &mut [ReplicationBuffer],
    world: &World,
    system_tick: Tick,
    replication_rules: &ReplicationRules,
) -> Result<(), bincode::Error> {
    for buffer in &mut *buffers {
        // start the array for entity change data:
        buffer.start_array();
    }

    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.id() != ArchetypeId::EMPTY)
        .filter(|archetype| archetype.id() != ArchetypeId::INVALID)
        .filter(|archetype| archetype.contains(replication_rules.get_marker_id()))
    {
        let table = world
            .storages()
            .tables
            .get(archetype.table_id())
            .expect("archetype should be valid");

        for archetype_entity in archetype.entities() {
            for buffer in &mut *buffers {
                buffer.start_entity_data(archetype_entity.entity());
            }

            for component_id in archetype.components() {
                let Some((replication_id, replication_info)) = replication_rules.get(component_id)
                else {
                    continue;
                };
                if archetype.contains(replication_info.ignored_id) {
                    continue;
                }

                let storage_type = archetype
                    .get_storage_type(component_id)
                    .unwrap_or_else(|| panic!("{component_id:?} be in archetype"));

                match storage_type {
                    StorageType::Table => {
                        let column = table
                            .get_column(component_id)
                            .unwrap_or_else(|| panic!("{component_id:?} should belong to table"));

                        // SAFETY: the table row obtained from the world state.
                        let ticks =
                            unsafe { column.get_ticks_unchecked(archetype_entity.table_row()) };
                        // SAFETY: component obtained from the archetype.
                        let component =
                            unsafe { column.get_data_unchecked(archetype_entity.table_row()) };

                        for buffer in &mut *buffers {
                            if ticks.is_changed(buffer.system_tick(), system_tick) {
                                buffer.write_change(replication_info, replication_id, component)?;
                            }
                        }
                    }
                    StorageType::SparseSet => {
                        let sparse_set = world
                            .storages()
                            .sparse_sets
                            .get(component_id)
                            .unwrap_or_else(|| panic!("{component_id:?} should be in sparse set"));

                        let entity = archetype_entity.entity();
                        let ticks = sparse_set
                            .get_ticks(entity)
                            .unwrap_or_else(|| panic!("{entity:?} should have {component_id:?}"));
                        let component = sparse_set
                            .get(entity)
                            .unwrap_or_else(|| panic!("{entity:?} should have {component_id:?}"));

                        for buffer in &mut *buffers {
                            if ticks.is_changed(buffer.system_tick(), system_tick) {
                                buffer.write_change(replication_info, replication_id, component)?;
                            }
                        }
                    }
                }
            }

            for buffer in &mut *buffers {
                buffer.end_entity_data()?;
            }
        }
    }

    for buffer in &mut *buffers {
        // ending the array of entity change data
        buffer.end_array()?;
    }

    Ok(())
}

/// Collect component removals into buffers based on last acknowledged tick.
fn collect_removals(
    buffers: &mut [ReplicationBuffer],
    removal_trackers: &Query<(Entity, &RemovalTracker)>,
    system_tick: Tick,
) -> Result<(), bincode::Error> {
    for buffer in &mut *buffers {
        buffer.start_array();
    }

    for (entity, removal_tracker) in removal_trackers {
        for buffer in &mut *buffers {
            buffer.start_entity_data(entity);
            for (&replication_id, &tick) in &removal_tracker.0 {
                if tick.is_newer_than(buffer.system_tick(), system_tick) {
                    buffer.write_removal(replication_id)?;
                }
            }
            buffer.end_entity_data()?;
        }
    }

    for buffer in &mut *buffers {
        buffer.end_array()?;
    }

    Ok(())
}

/// Collect entity despawns into buffers based on last acknowledged tick.
fn collect_despawns(
    buffers: &mut [ReplicationBuffer],
    despawn_tracker: &DespawnTracker,
    system_tick: Tick,
) -> Result<(), bincode::Error> {
    for buffer in &mut *buffers {
        buffer.start_array();
    }

    for &(entity, tick) in &despawn_tracker.despawns {
        for buffer in &mut *buffers {
            if tick.is_newer_than(buffer.system_tick(), system_tick) {
                buffer.write_despawn(entity)?;
            }
        }
    }

    for buffer in &mut *buffers {
        buffer.end_array()?;
    }

    Ok(())
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
    /// Send updates from server every frame.
    EveryFrame,
    /// [`ServerSet::Send`] should be manually configured.
    Manual,
}

/// Stores information about ticks.
///
/// Used only on server.
#[derive(Resource, Default)]
pub struct AckedTicks {
    /// Last acknowledged server ticks for all clients.
    clients: HashMap<u64, RepliconTick>,

    /// Stores mapping from server ticks to system change ticks.
    system_ticks: HashMap<RepliconTick, Tick>,
}

impl AckedTicks {
    /// Stores mapping between `replicon_tick` and the current `system_tick`.
    fn register_tick(&mut self, replicon_tick: RepliconTick, system_tick: Tick) {
        self.system_ticks.insert(replicon_tick, system_tick);
    }

    /// Removes system tick mappings for acks that was acknowledged by everyone.
    fn cleanup_system_ticks(&mut self) {
        self.system_ticks
            .retain(|tick, _| self.clients.values().any(|acked_tick| acked_tick <= tick));
    }

    /// Returns last acknowledged server ticks for all clients.
    #[inline]
    pub fn acked_ticks(&self) -> &HashMap<u64, RepliconTick> {
        &self.clients
    }
}

/// Fills scene with all replicated entities and their components.
///
/// # Panics
///
/// Panics if any replicated component is not registered using `register_type()`
/// or missing `#[reflect(Component)]`.
pub fn replicate_into_scene(scene: &mut DynamicScene, world: &World) {
    let registry = world.resource::<AppTypeRegistry>();
    let replication_rules = world.resource::<ReplicationRules>();

    let registry = registry.read();
    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.id() != ArchetypeId::EMPTY)
        .filter(|archetype| archetype.id() != ArchetypeId::INVALID)
        .filter(|archetype| archetype.contains(replication_rules.get_marker_id()))
    {
        let entities_offset = scene.entities.len();
        for archetype_entity in archetype.entities() {
            scene.entities.push(DynamicEntity {
                entity: archetype_entity.entity(),
                components: Vec::new(),
            });
        }

        for component_id in archetype.components() {
            let Some((_, replication_info)) = replication_rules.get(component_id) else {
                continue;
            };
            if archetype.contains(replication_info.ignored_id) {
                continue;
            }

            // SAFETY: `component_info` obtained from the world.
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

            for (index, archetype_entity) in archetype.entities().iter().enumerate() {
                let component = reflect_component
                    .reflect(world.entity(archetype_entity.entity()))
                    .unwrap_or_else(|| panic!("entity should have {type_name}"));

                scene.entities[entities_offset + index]
                    .components
                    .push(component.clone_value());
            }
        }
    }
}
