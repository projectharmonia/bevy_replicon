pub mod client_entity_map;
pub mod connected_clients;
pub(super) mod despawn_buffer;
pub(super) mod removal_buffer;
pub(super) mod replicated_archetypes;
pub(super) mod replication_messages;
pub mod replicon_server;
pub mod server_tick;

use std::{io::Cursor, mem, time::Duration};

use bevy::{
    ecs::{
        archetype::ArchetypeEntity,
        component::{ComponentId, ComponentTicks, StorageType, Tick},
        storage::{SparseSets, Table},
        system::SystemChangeTick,
    },
    prelude::*,
    ptr::Ptr,
    time::common_conditions::on_timer,
};

use crate::core::{
    common_conditions::{server_just_stopped, server_running},
    replication_fns::{ctx::SerializeCtx, ReplicationFns},
    replication_rules::ReplicationRules,
    replicon_channels::{ReplicationChannel, RepliconChannels},
    replicon_tick::RepliconTick,
    ClientId,
};
use client_entity_map::ClientEntityMap;
use connected_clients::{
    client_visibility::Visibility, ClientBuffers, ConnectedClient, ConnectedClients,
};
use despawn_buffer::{DespawnBuffer, DespawnBufferPlugin};
use removal_buffer::{RemovalBuffer, RemovalBufferPlugin};
use replicated_archetypes::ReplicatedArchetypes;
use replication_messages::ReplicationMessages;
use replicon_server::RepliconServer;
use server_tick::ServerTick;

pub struct ServerPlugin {
    /// Tick configuration.
    pub tick_policy: TickPolicy,

    /// Visibility configuration.
    pub visibility_policy: VisibilityPolicy,

    /// The time after which updates will be considered lost if an acknowledgment is not received for them.
    ///
    /// In practice updates will live at least `update_timeout`, and at most `2*update_timeout`.
    pub update_timeout: Duration,
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self {
            tick_policy: TickPolicy::MaxTickRate(30),
            visibility_policy: Default::default(),
            update_timeout: Duration::from_secs(10),
        }
    }
}

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((DespawnBufferPlugin, RemovalBufferPlugin))
            .init_resource::<RepliconServer>()
            .init_resource::<ServerTick>()
            .init_resource::<ClientBuffers>()
            .init_resource::<ClientEntityMap>()
            .insert_resource(ConnectedClients::new(self.visibility_policy))
            .add_event::<ServerEvent>()
            .configure_sets(
                PreUpdate,
                (
                    ServerSet::ReceivePackets,
                    ServerSet::SendEvents,
                    ServerSet::Receive,
                )
                    .chain(),
            )
            .configure_sets(
                PostUpdate,
                (
                    ServerSet::StoreHierarchy,
                    ServerSet::Send,
                    ServerSet::SendPackets,
                )
                    .chain(),
            )
            .add_systems(Startup, Self::setup_channels)
            .add_systems(
                PreUpdate,
                (
                    Self::handle_connections,
                    Self::receive_acks,
                    Self::cleanup_acks(self.update_timeout).run_if(on_timer(self.update_timeout)),
                )
                    .chain()
                    .in_set(ServerSet::Receive)
                    .run_if(server_running),
            )
            .add_systems(
                PostUpdate,
                (
                    Self::send_replication
                        .map(Result::unwrap)
                        .in_set(ServerSet::Send)
                        .run_if(server_running)
                        .run_if(resource_changed::<ServerTick>),
                    Self::reset.run_if(server_just_stopped),
                ),
            );

        match self.tick_policy {
            TickPolicy::MaxTickRate(max_tick_rate) => {
                let tick_time = Duration::from_millis(1000 / max_tick_rate as u64);
                app.add_systems(
                    PostUpdate,
                    Self::increment_tick
                        .before(Self::send_replication)
                        .run_if(server_running)
                        .run_if(on_timer(tick_time)),
                );
            }
            TickPolicy::EveryFrame => {
                app.add_systems(
                    PostUpdate,
                    Self::increment_tick
                        .before(Self::send_replication)
                        .run_if(server_running),
                );
            }
            TickPolicy::Manual => (),
        }
    }
}

impl ServerPlugin {
    fn setup_channels(mut server: ResMut<RepliconServer>, channels: Res<RepliconChannels>) {
        server.setup_client_channels(channels.client_channels().len());
    }

    /// Increments current server tick which causes the server to replicate this frame.
    pub fn increment_tick(mut server_tick: ResMut<ServerTick>) {
        server_tick.increment();
        trace!("incremented {server_tick:?}");
    }

    fn handle_connections(
        mut server_events: EventReader<ServerEvent>,
        mut entity_map: ResMut<ClientEntityMap>,
        mut connected_clients: ResMut<ConnectedClients>,
        mut server: ResMut<RepliconServer>,
        mut client_buffers: ResMut<ClientBuffers>,
    ) {
        for event in server_events.read() {
            match *event {
                ServerEvent::ClientDisconnected { client_id, .. } => {
                    entity_map.0.remove(&client_id);
                    connected_clients.remove(&mut client_buffers, client_id);
                    server.remove_client(client_id);
                }
                ServerEvent::ClientConnected { client_id } => {
                    connected_clients.add(&mut client_buffers, client_id);
                }
            }
        }
    }

    fn cleanup_acks(
        update_timeout: Duration,
    ) -> impl FnMut(ResMut<ConnectedClients>, ResMut<ClientBuffers>, Res<Time>) {
        move |mut connected_clients: ResMut<ConnectedClients>,
              mut client_buffers: ResMut<ClientBuffers>,
              time: Res<Time>| {
            let min_timestamp = time.elapsed().saturating_sub(update_timeout);
            for client in connected_clients.iter_mut() {
                client.remove_older_updates(&mut client_buffers, min_timestamp);
            }
        }
    }

    fn receive_acks(
        change_tick: SystemChangeTick,
        mut server: ResMut<RepliconServer>,
        mut connected_clients: ResMut<ConnectedClients>,
        mut client_buffers: ResMut<ClientBuffers>,
    ) {
        for (client_id, message) in server.receive(ReplicationChannel::Init) {
            let mut cursor = Cursor::new(&*message);
            let message_end = message.len() as u64;
            while cursor.position() < message_end {
                match bincode::deserialize_from(&mut cursor) {
                    Ok(update_index) => {
                        let client = connected_clients.client_mut(client_id);
                        client.acknowledge(
                            &mut client_buffers,
                            change_tick.this_run(),
                            update_index,
                        );
                    }
                    Err(e) => debug!("unable to deserialize update index from {client_id:?}: {e}"),
                }
            }
        }
    }

    /// Collects [`ReplicationMessages`] and sends them.
    pub(super) fn send_replication(
        mut messages: Local<ReplicationMessages>,
        mut replicated_archetypes: Local<ReplicatedArchetypes>,
        change_tick: SystemChangeTick,
        mut set: ParamSet<(
            &World,
            ResMut<ConnectedClients>,
            ResMut<ClientEntityMap>,
            ResMut<DespawnBuffer>,
            ResMut<RemovalBuffer>,
            ResMut<ClientBuffers>,
            ResMut<RepliconServer>,
        )>,
        replication_fns: Res<ReplicationFns>,
        rules: Res<ReplicationRules>,
        server_tick: Res<ServerTick>,
        time: Res<Time>,
    ) -> bincode::Result<()> {
        replicated_archetypes.update(set.p0(), &rules);

        let connected_clients = mem::take(&mut *set.p1()); // Take ownership to avoid borrowing issues.
        messages.prepare(connected_clients);

        collect_mappings(&mut messages, &mut set.p2())?;
        collect_despawns(&mut messages, &mut set.p3())?;
        collect_removals(&mut messages, &mut set.p4(), change_tick.this_run())?;
        collect_changes(
            &mut messages,
            &replicated_archetypes,
            &replication_fns,
            set.p0(),
            &change_tick,
            **server_tick,
        )?;

        let mut client_buffers = mem::take(&mut *set.p5());
        let connected_clients = messages.send(
            &mut set.p6(),
            &mut client_buffers,
            **server_tick,
            change_tick.this_run(),
            time.elapsed(),
        )?;

        // Return borrowed data back.
        *set.p1() = connected_clients;
        *set.p5() = client_buffers;

        Ok(())
    }

    fn reset(
        mut server_tick: ResMut<ServerTick>,
        mut entity_map: ResMut<ClientEntityMap>,
        mut connected_clients: ResMut<ConnectedClients>,
        mut client_buffers: ResMut<ClientBuffers>,
    ) {
        *server_tick = Default::default();
        entity_map.0.clear();
        connected_clients.clear(&mut client_buffers);
    }
}

/// Collects and writes any new entity mappings that happened in this tick.
///
/// On deserialization mappings should be processed first, so all referenced entities after it will behave correctly.
fn collect_mappings(
    messages: &mut ReplicationMessages,
    entity_map: &mut ClientEntityMap,
) -> bincode::Result<()> {
    for (message, _, client) in messages.iter_mut_with_clients() {
        message.start_array();

        if let Some(mappings) = entity_map.0.get_mut(&client.id()) {
            for mapping in mappings.drain(..) {
                message.write_client_mapping(&mapping)?;
            }
        }

        message.end_array()?;
    }
    Ok(())
}

/// Collects component insertions from this tick into init messages, and changes into update messages
/// since the last entity tick.
fn collect_changes(
    messages: &mut ReplicationMessages,
    replicated_archetypes: &ReplicatedArchetypes,
    replication_fns: &ReplicationFns,
    world: &World,
    change_tick: &SystemChangeTick,
    server_tick: RepliconTick,
) -> bincode::Result<()> {
    for (init_message, _) in messages.iter_mut() {
        init_message.start_array();
    }

    for replicated_archetype in replicated_archetypes.iter() {
        // SAFETY: all IDs from replicated archetypes obtained from real archetypes.
        let archetype = unsafe {
            world
                .archetypes()
                .get(replicated_archetype.id)
                .unwrap_unchecked()
        };
        // SAFETY: table obtained from this archetype.
        let table = unsafe {
            world
                .storages()
                .tables
                .get(archetype.table_id())
                .unwrap_unchecked()
        };

        for entity in archetype.entities() {
            for (init_message, update_message, client) in messages.iter_mut_with_clients() {
                init_message.start_entity_data(entity.id());
                update_message.start_entity_data(entity.id());
                client.visibility_mut().cache_visibility(entity.id());
            }

            // SAFETY: all replicated archetypes have marker component with table storage.
            let (_, marker_ticks) = unsafe {
                get_component_unchecked(
                    table,
                    &world.storages().sparse_sets,
                    entity,
                    StorageType::Table,
                    replicated_archetypes.marker_id(),
                )
            };
            // If the marker was added in this tick, the entity just started replicating.
            // It could be a newly spawned entity or an old entity with just-enabled replication,
            // so we need to include even old components that were registered for replication.
            let marker_added =
                marker_ticks.is_added(change_tick.last_run(), change_tick.this_run());

            for replicated_component in &replicated_archetype.components {
                // SAFETY: component and storage were obtained from this archetype.
                let (component, ticks) = unsafe {
                    get_component_unchecked(
                        table,
                        &world.storages().sparse_sets,
                        entity,
                        replicated_component.storage_type,
                        replicated_component.component_id,
                    )
                };

                let (component_fns, rule_fns) = replication_fns.get(replicated_component.fns_id);
                let ctx = SerializeCtx { server_tick };
                let mut shared_bytes = None;
                for (init_message, update_message, client) in messages.iter_mut_with_clients() {
                    let visibility = client.visibility().cached_visibility();
                    if visibility == Visibility::Hidden {
                        continue;
                    }

                    let new_entity = marker_added || visibility == Visibility::Gained;
                    if new_entity || ticks.is_added(change_tick.last_run(), change_tick.this_run())
                    {
                        init_message.write_component(
                            &mut shared_bytes,
                            rule_fns,
                            component_fns,
                            &ctx,
                            replicated_component.fns_id,
                            component,
                        )?;
                    } else {
                        let tick = client
                            .get_change_limit(entity.id())
                            .expect("entity should be present after adding component");
                        if ticks.is_changed(tick, change_tick.this_run()) {
                            update_message.write_component(
                                &mut shared_bytes,
                                rule_fns,
                                component_fns,
                                &ctx,
                                replicated_component.fns_id,
                                component,
                            )?;
                        }
                    }
                }
            }

            for (init_message, update_message, client) in messages.iter_mut_with_clients() {
                let visibility = client.visibility().cached_visibility();
                if visibility == Visibility::Hidden {
                    continue;
                }

                let new_entity = marker_added || visibility == Visibility::Gained;
                if new_entity || init_message.entity_data_size() != 0 {
                    // If there is any insertion or we must initialize, include all updates into init message
                    // and bump the last acknowledged tick to keep entity updates atomic.
                    init_message.take_entity_data(update_message)?;
                    client.set_change_limit(entity.id(), change_tick.this_run());
                } else {
                    update_message.end_entity_data()?;
                }

                init_message.end_entity_data(new_entity)?;
            }
        }
    }

    for (init_message, _) in messages.iter_mut() {
        init_message.end_array()?;
    }

    Ok(())
}

/// Extracts component in form of [`Ptr`] and its ticks from table or sparse set based on its storage type.
///
/// # Safety
///
/// Component should be present in this archetype and have this storage type.
unsafe fn get_component_unchecked<'w>(
    table: &'w Table,
    sparse_sets: &'w SparseSets,
    entity: &ArchetypeEntity,
    storage_type: StorageType,
    component_id: ComponentId,
) -> (Ptr<'w>, ComponentTicks) {
    match storage_type {
        StorageType::Table => {
            let column = table.get_column(component_id).unwrap_unchecked();
            let component = column.get_data_unchecked(entity.table_row());
            let ticks = column.get_ticks_unchecked(entity.table_row());

            (component, ticks)
        }
        StorageType::SparseSet => {
            let sparse_set = sparse_sets.get(component_id).unwrap_unchecked();
            let component = sparse_set.get(entity.id()).unwrap_unchecked();
            let ticks = sparse_set.get_ticks(entity.id()).unwrap_unchecked();

            (component, ticks)
        }
    }
}

/// Collect entity despawns from this tick into init messages.
fn collect_despawns(
    messages: &mut ReplicationMessages,
    despawn_buffer: &mut DespawnBuffer,
) -> bincode::Result<()> {
    for (message, _) in messages.iter_mut() {
        message.start_array();
    }

    for entity in despawn_buffer.drain(..) {
        let mut shared_bytes = None;
        for (message, _, client) in messages.iter_mut_with_clients() {
            client.remove_despawned(entity);
            message.write_entity(&mut shared_bytes, entity)?;
        }
    }

    for (message, _, client) in messages.iter_mut_with_clients() {
        for entity in client.drain_lost_visibility() {
            message.write_entity(&mut None, entity)?;
        }

        message.end_array()?;
    }

    Ok(())
}

/// Collects component removals from this tick into init messages.
fn collect_removals(
    messages: &mut ReplicationMessages,
    removal_buffer: &mut RemovalBuffer,
    tick: Tick,
) -> bincode::Result<()> {
    for (message, _) in messages.iter_mut() {
        message.start_array();
    }

    for (entity, remove_ids) in removal_buffer.iter() {
        for (message, _, client) in messages.iter_mut_with_clients() {
            message.start_entity_data(entity);
            for fns_info in remove_ids {
                client.set_change_limit(entity, tick);
                message.write_fns_id(fns_info.fns_id())?;
            }
            message.end_entity_data(false)?;
        }
    }
    removal_buffer.clear();

    for (message, _) in messages.iter_mut() {
        message.end_array()?;
    }

    Ok(())
}

/// Set with replication and event systems related to server.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ServerSet {
    /// Systems that receive packets from the messaging backend.
    ///
    /// Used by the messaging backend.
    ///
    /// Runs in [`PreUpdate`].
    ReceivePackets,
    /// Systems that emit [`ServerEvent`].
    ///
    /// The messaging backend should convert its own connection events into [`ServerEvents`](ServerEvent)
    /// in this set.
    ///
    /// Runs in [`PreUpdate`].
    SendEvents,
    /// Systems that receive data from [`RepliconServer`].
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PreUpdate`].
    Receive,
    /// Systems that store hierarchy changes in [`ParentSync`](super::parent_sync::ParentSync).
    ///
    /// Runs in [`PostUpdate`].
    StoreHierarchy,
    /// Systems that send data to [`RepliconServer`].
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PostUpdate`] on server tick, see [`TickPolicy`].
    Send,
    /// Systems that send packets to the messaging backend.
    ///
    /// Used by the messaging backend.
    ///
    /// Runs in [`PostUpdate`] on server tick, see [`TickPolicy`].
    SendPackets,
}

/// Controls how often [`RepliconTick`] is incremented on the server.
///
/// When [`RepliconTick`] is mutated, the server's replication
/// system will run. This means the tick policy controls how often server state is replicated.
///
/// Note that component updates are replicated over the unreliable channel, so if a component update packet is lost
/// then component updates won't be resent until the server's replication system runs again.
pub enum TickPolicy {
    /// The replicon tick is incremented at most max ticks per second. In practice the tick rate may be lower if the
    /// app's update cycle duration is too long.
    ///
    /// By default it's 30 ticks per second.
    MaxTickRate(u16),
    /// The replicon tick is incremented every frame.
    EveryFrame,
    /// The user should manually configure [`ServerPlugin::increment_tick`] or manually increment
    /// [`RepliconTick`].
    Manual,
}

/// Controls how visibility will be managed via [`ClientVisibility`](connected_clients::client_visibility::ClientVisibility).
#[derive(Default, Debug, Clone, Copy)]
pub enum VisibilityPolicy {
    /// All entities are visible by default and visibility can't be changed.
    #[default]
    All,
    /// All entities are visible by default and should be explicitly registered to be hidden.
    Blacklist,
    /// All entities are hidden by default and should be explicitly registered to be visible.
    Whitelist,
}

/// Connection and disconnection events on the server.
///
/// The messaging backend is responsible for emitting these in [`ServerSet::SendEvents`].
#[derive(Event)]
pub enum ServerEvent {
    ClientConnected { client_id: ClientId },
    ClientDisconnected { client_id: ClientId, reason: String },
}
