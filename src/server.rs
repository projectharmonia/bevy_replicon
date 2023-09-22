pub(super) mod despawn_tracker;
pub(super) mod removal_tracker;

use std::{io::Cursor, mem, time::Duration};

use bevy::{
    ecs::{
        archetype::ArchetypeId,
        component::{StorageType, Tick},
        system::SystemChangeTick,
    },
    prelude::*,
    ptr::Ptr,
    time::common_conditions::on_timer,
    utils::HashMap,
};
use bevy_renet::{
    renet::{Bytes, RenetClient, RenetServer, ServerEvent},
    transport::NetcodeServerPlugin,
    RenetServerPlugin,
};
use bincode::{DefaultOptions, Options};
use derive_more::Constructor;

use crate::replicon_core::{
    NetworkTick, ReplicationId, ReplicationInfo, ReplicationRules, REPLICATION_CHANNEL_ID,
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
        .init_resource::<ServerTicks>()
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
                    .pipe(unwrap)
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
    fn acks_receiving_system(
        mut server_ticks: ResMut<ServerTicks>,
        mut server: ResMut<RenetServer>,
    ) {
        for client_id in server.clients_id() {
            while let Some(message) = server.receive_message(client_id, REPLICATION_CHANNEL_ID) {
                match bincode::deserialize::<NetworkTick>(&message) {
                    Ok(tick) => {
                        let acked_tick = server_ticks.acked_ticks.entry(client_id).or_default();
                        if *acked_tick < tick {
                            *acked_tick = tick;
                        }
                    }
                    Err(e) => error!("unable to deserialize tick from client {client_id}: {e}"),
                }
            }
        }

        server_ticks.cleanup_system_ticks();
    }

    fn acks_cleanup_system(
        mut server_events: EventReader<ServerEvent>,
        mut server_ticks: ResMut<ServerTicks>,
    ) {
        for event in &mut server_events {
            match event {
                ServerEvent::ClientDisconnected { client_id, .. } => {
                    server_ticks.acked_ticks.remove(client_id);
                }
                ServerEvent::ClientConnected { client_id } => {
                    server_ticks.acked_ticks.entry(*client_id).or_default();
                }
            }
        }
    }

    fn diffs_sending_system(
        mut buffers: Local<Vec<ReplicationBuffer>>,
        change_tick: SystemChangeTick,
        mut set: ParamSet<(&World, ResMut<RenetServer>, ResMut<ServerTicks>)>,
        replication_rules: Res<ReplicationRules>,
        despawn_tracker: Res<DespawnTracker>,
        removal_trackers: Query<(Entity, &RemovalTracker)>,
    ) -> Result<(), bincode::Error> {
        let mut server_ticks = set.p2();
        server_ticks.increment(change_tick.this_run());

        let buffers = prepare_buffers(&mut buffers, &server_ticks)?;
        collect_changes(
            buffers,
            set.p0(),
            change_tick.this_run(),
            &replication_rules,
        )?;
        collect_removals(buffers, &removal_trackers, change_tick.this_run())?;
        collect_despawns(buffers, &despawn_tracker, change_tick.this_run())?;

        for buffer in buffers {
            debug_assert_eq!(buffer.array_len, 0);
            debug_assert_eq!(buffer.entity_data_len, 0);

            if buffer.arrays_with_data > 0 {
                set.p1().send_message(
                    buffer.client_id,
                    REPLICATION_CHANNEL_ID,
                    Bytes::copy_from_slice(buffer.message.get_ref()),
                );
            }
        }

        Ok(())
    }

    fn reset_system(mut server_ticks: ResMut<ServerTicks>) {
        server_ticks.acked_ticks.clear();
        server_ticks.system_ticks.clear();
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
    server_ticks: &ServerTicks,
) -> Result<&'a mut [ReplicationBuffer], bincode::Error> {
    buffers.reserve(server_ticks.acked_ticks.len());
    for (index, (&client_id, &tick)) in server_ticks.acked_ticks.iter().enumerate() {
        let system_tick = *server_ticks
            .system_ticks
            .get(&tick)
            .unwrap_or(&Tick::new(0));

        if let Some(buffer) = buffers.get_mut(index) {
            buffer.reset(client_id, system_tick, server_ticks.current_tick)?;
        } else {
            buffers.push(ReplicationBuffer::new(
                client_id,
                system_tick,
                server_ticks.current_tick,
            )?);
        }
    }

    Ok(&mut buffers[..server_ticks.acked_ticks.len()])
}

/// Collect component changes into buffers based on last acknowledged tick.
fn collect_changes(
    buffers: &mut [ReplicationBuffer],
    world: &World,
    system_tick: Tick,
    replication_rules: &ReplicationRules,
) -> Result<(), bincode::Error> {
    for buffer in &mut *buffers {
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
                let Some(&replication_id) = replication_rules.get_ids().get(&component_id) else {
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
                            if ticks.is_changed(buffer.system_tick, system_tick) {
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
                            if ticks.is_changed(buffer.system_tick, system_tick) {
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
                if tick.is_newer_than(buffer.system_tick, system_tick) {
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
            if tick.is_newer_than(buffer.system_tick, system_tick) {
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
    /// [`ServerSet::Send`] should be manually configured.
    Manual,
}

/// Stores information about ticks.
///
/// Used only on server.
#[derive(Resource, Default)]
pub struct ServerTicks {
    /// Current server tick.
    current_tick: NetworkTick,

    /// Last acknowledged server ticks for all clients.
    acked_ticks: HashMap<u64, NetworkTick>,

    /// Stores mapping from server ticks to system change ticks.
    system_ticks: HashMap<NetworkTick, Tick>,
}

impl ServerTicks {
    /// Increments current tick by 1 and makes corresponding system tick mapping for it.
    fn increment(&mut self, system_tick: Tick) {
        self.current_tick.increment();
        self.system_ticks.insert(self.current_tick, system_tick);
    }

    /// Removes system tick mappings for acks that was acknowledged by everyone.
    fn cleanup_system_ticks(&mut self) {
        self.system_ticks.retain(|tick, _| {
            self.acked_ticks
                .values()
                .all(|acked_tick| acked_tick > tick)
        })
    }

    /// Returns current server tick.
    pub fn current_tick(&self) -> NetworkTick {
        self.current_tick
    }

    /// Returns last acknowledged server ticks for all clients.
    pub fn acked_ticks(&self) -> &HashMap<u64, NetworkTick> {
        &self.acked_ticks
    }
}

/// A reusable buffer with replicated data for a client.
///
/// See also [Limits](../index.html#limits)
struct ReplicationBuffer {
    /// ID of a client for which this buffer is written.
    client_id: u64,

    /// Last system tick acknowledged by the client.
    ///
    /// Used for changes preparation.
    system_tick: Tick,

    /// Buffer with serialized data.
    message: Cursor<Vec<u8>>,

    /// Position of the array from last call of [`Self::start_array`].
    array_pos: u64,

    /// Length of the array that updated automatically after writing data.
    array_len: u16,

    /// The number of non-empty arrays stored.
    arrays_with_data: u8,

    /// Position of the entity map from last call of [`Self::start_entity_data`] or [`Self::write_current_entity`].
    entity_data_pos: u64,

    /// Length of the map that updated automatically after writing data.
    entity_data_len: u8,

    /// Entity from last call of [`Self::start_entity_data`].
    current_entity: Entity,
}

impl ReplicationBuffer {
    /// Creates a new buffer with assigned client ID and acknowledged system tick
    /// and writes current server tick into buffer data.
    fn new(
        client_id: u64,
        system_tick: Tick,
        current_tick: NetworkTick,
    ) -> Result<Self, bincode::Error> {
        let mut message = Default::default();
        bincode::serialize_into(&mut message, &current_tick)?;
        Ok(Self {
            client_id,
            system_tick,
            message,
            array_pos: Default::default(),
            array_len: Default::default(),
            arrays_with_data: Default::default(),
            entity_data_pos: Default::default(),
            entity_data_len: Default::default(),
            current_entity: Entity::PLACEHOLDER,
        })
    }

    /// Reassigns current client ID and acknowledged system tick to the buffer
    /// and replaces buffer data with current server tick.
    ///
    /// Keeps allocated capacity of the buffer data.
    fn reset(
        &mut self,
        client_id: u64,
        system_tick: Tick,
        current_tick: NetworkTick,
    ) -> Result<(), bincode::Error> {
        self.client_id = client_id;
        self.system_tick = system_tick;
        self.message.set_position(0);
        self.message.get_mut().clear();
        self.arrays_with_data = 0;
        bincode::serialize_into(&mut self.message, &current_tick)?;

        Ok(())
    }

    /// Starts writing array by remembering its position to write length after.
    ///
    /// Arrays can contain entity data or despawns inside.
    /// Length will be increased automatically after writing data.
    /// See also [`Self::end_array`], [`Self::start_entity_data`] and [`Self::write_despawn`].
    fn start_array(&mut self) {
        debug_assert_eq!(self.array_len, 0);

        self.array_pos = self.message.position();
        self.message
            .set_position(self.array_pos + mem::size_of_val(&self.array_len) as u64);
    }

    /// Ends writing array by writing its length into the last remembered position.
    ///
    /// See also [`Self::start_array`].
    fn end_array(&mut self) -> Result<(), bincode::Error> {
        if self.array_len != 0 {
            let previous_pos = self.message.position();
            self.message.set_position(self.array_pos);

            bincode::serialize_into(&mut self.message, &self.array_len)?;

            self.message.set_position(previous_pos);
            self.array_len = 0;
            self.arrays_with_data += 1;
        } else {
            self.message.set_position(self.array_pos);
            bincode::serialize_into(&mut self.message, &self.array_len)?;
        }

        Ok(())
    }

    /// Starts writing entity and its data by remembering [`Entity`].
    ///
    /// Arrays can contain component changes or removals inside.
    /// Length will be increased automatically after writing data.
    /// Entity will be written lazily after first data write and its position will be remembered to write length later.
    /// See also [`Self::end_entity_data`], [`Self::write_current_entity`], [`Self::write_change`] and [`Self::write_removal`].
    fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_len, 0);

        self.current_entity = entity;
    }

    /// Writes entity for current data and updates remembered position for it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_current_entity(&mut self) -> Result<(), bincode::Error> {
        DefaultOptions::new().serialize_into(&mut self.message, &self.current_entity)?;
        self.entity_data_pos = self.message.position();
        self.message
            .set_position(self.entity_data_pos + mem::size_of_val(&self.entity_data_len) as u64);

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written.
    /// See also [`Self::start_array`], [`Self::write_current_entity`], [`Self::write_change`] and [`Self::write_removal`].
    fn end_entity_data(&mut self) -> Result<(), bincode::Error> {
        if self.entity_data_len != 0 {
            let previous_pos = self.message.position();
            self.message.set_position(self.entity_data_pos);

            bincode::serialize_into(&mut self.message, &self.entity_data_len)?;

            self.message.set_position(previous_pos);
            self.entity_data_len = 0;
            self.array_len = self
                .array_len
                .checked_add(1)
                .ok_or(bincode::ErrorKind::SizeLimit)?;
        } else {
            self.message.set_position(self.entity_data_pos);
        }

        Ok(())
    }

    /// Serializes [`ReplicationId`] and component into the buffer data.
    ///
    /// Should be called only inside entity data.
    /// Increases entity data length by 1.
    /// See also [`Self::start_entity_data`].
    fn write_change(
        &mut self,
        replication_info: &ReplicationInfo,
        replication_id: ReplicationId,
        ptr: Ptr,
    ) -> Result<(), bincode::Error> {
        if self.entity_data_len == 0 {
            self.write_current_entity()?;
        }

        DefaultOptions::new().serialize_into(&mut self.message, &replication_id)?;
        (replication_info.serialize)(ptr, &mut self.message)?;
        self.entity_data_len += 1;

        Ok(())
    }

    /// Serializes [`ReplicationId`] of the removed component into the buffer data.
    ///
    /// Should be called only inside entity data.
    /// Increases entity data length by 1.
    /// See also [`Self::start_entity_data`].
    fn write_removal(&mut self, replication_id: ReplicationId) -> Result<(), bincode::Error> {
        if self.entity_data_len == 0 {
            self.write_current_entity()?;
        }

        DefaultOptions::new().serialize_into(&mut self.message, &replication_id)?;
        self.entity_data_len += 1;

        Ok(())
    }

    /// Serializes despawned [`Entity`].
    ///
    /// Should be called only inside array.
    /// Increases array length by 1.
    /// See also [`Self::start_array`].
    fn write_despawn(&mut self, entity: Entity) -> Result<(), bincode::Error> {
        DefaultOptions::new().serialize_into(&mut self.message, &entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }
}
