pub(super) mod despawn_tracker;
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
    time::common_conditions::on_timer,
    utils::HashMap,
};
use bevy_renet::{
    renet::{Bytes, ClientId, RenetClient, RenetServer, ServerEvent},
    transport::NetcodeServerPlugin,
    RenetReceive, RenetSend, RenetServerPlugin,
};

use crate::replicon_core::{
    replication_rules::ReplicationRules, replicon_tick::RepliconTick, REPLICATION_CHANNEL_ID,
};
use despawn_tracker::{DespawnTracker, DespawnTrackerPlugin};
use removal_tracker::{RemovalTracker, RemovalTrackerPlugin};
use replication_buffer::ReplicationBuffer;

pub const SERVER_ID: ClientId = ClientId::from_raw(0);

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
        .init_resource::<MinRepliconTick>()
        .init_resource::<ClientEntityMap>()
        .configure_sets(PreUpdate, ServerSet::Receive.after(RenetReceive))
        .configure_sets(
            PostUpdate,
            ServerSet::Send
                .before(RenetSend)
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
                Self::replication_sending_system
                    .map(Result::unwrap)
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
                        .before(Self::replication_sending_system)
                        .run_if(on_timer(tick_time)),
                );
            }
            TickPolicy::EveryFrame => {
                app.add_systems(
                    PostUpdate,
                    Self::increment_tick.before(Self::replication_sending_system),
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

    /// Increments current server tick which causes the server to replicate this frame.
    pub fn increment_tick(mut tick: ResMut<RepliconTick>) {
        tick.increment();
        trace!("incremented {tick:?}");
    }

    fn acks_receiving_system(
        mut acked_ticks: ResMut<AckedTicks>,
        mut server: ResMut<RenetServer>,
        mut entity_map: ResMut<ClientEntityMap>,
    ) {
        for client_id in server.clients_id() {
            while let Some(message) = server.receive_message(client_id, REPLICATION_CHANNEL_ID) {
                match bincode::deserialize::<RepliconTick>(&message) {
                    Ok(tick) => {
                        let acked_tick = acked_ticks.clients.entry(client_id).or_default();
                        if *acked_tick < tick {
                            *acked_tick = tick;
                            entity_map.cleanup_acked(client_id, *acked_tick);
                            trace!("client {client_id} acknowledged {tick:?}");
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
        for event in server_events.read() {
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
    pub(super) fn replication_sending_system(
        mut messages: Local<Vec<ReplicationMessage>>,
        change_tick: SystemChangeTick,
        mut set: ParamSet<(&World, ResMut<RenetServer>, ResMut<AckedTicks>)>,
        replication_rules: Res<ReplicationRules>,
        despawn_tracker: Res<DespawnTracker>,
        replicon_tick: Res<RepliconTick>,
        min_replicon_tick: Res<MinRepliconTick>,
        removal_trackers: Query<(Entity, &RemovalTracker)>,
        entity_map: Res<ClientEntityMap>,
    ) -> bincode::Result<()> {
        let mut acked_ticks = set.p2();
        acked_ticks.register_tick(*replicon_tick, change_tick.this_run());

        let messages = prepare_messages(
            &mut messages,
            &acked_ticks,
            *replicon_tick,
            *min_replicon_tick,
        )?;

        collect_mappings(messages, &entity_map)?;
        collect_changes(
            messages,
            set.p0(),
            change_tick.this_run(),
            &replication_rules,
        )?;
        collect_removals(messages, &removal_trackers, change_tick.this_run())?;
        collect_despawns(messages, &despawn_tracker, change_tick.this_run())?;

        for messages in messages {
            messages.send_to(&mut set.p1());
        }

        Ok(())
    }

    fn reset_system(mut acked_ticks: ResMut<AckedTicks>) {
        acked_ticks.clients.clear();
        acked_ticks.system_ticks.clear();
    }
}

/// Initializes message for each client and returns it as mutable slice.
///
/// Reuses already allocated messages.
/// Creates new messages if number of clients is bigger then the number of allocated messages.
/// If there are more messages than the number of clients, then the extra messages remain untouched
/// and the returned slice will not include them.
fn prepare_messages<'a>(
    messages: &'a mut Vec<ReplicationMessage>,
    acked_ticks: &AckedTicks,
    replicon_tick: RepliconTick,
    min_replicon_tick: MinRepliconTick,
) -> bincode::Result<&'a mut [ReplicationMessage]> {
    messages.reserve(acked_ticks.clients.len());
    for (index, (&client_id, &acked_tick)) in acked_ticks.clients.iter().enumerate() {
        let system_tick = *acked_ticks
            .system_ticks
            .get(&acked_tick)
            .unwrap_or(&Tick::new(0));

        let send_empty = acked_tick < *min_replicon_tick;
        if let Some(message) = messages.get_mut(index) {
            message.reset(replicon_tick, client_id, system_tick, send_empty)?;
        } else {
            messages.push(ReplicationMessage::new(
                replicon_tick,
                client_id,
                system_tick,
                send_empty,
            )?);
        }
    }

    Ok(&mut messages[..acked_ticks.clients.len()])
}

/// Collect and write any new entity mappings into messages since last acknowledged tick.
///
/// Mappings will be processed first, so all referenced entities after it will behave correctly.
fn collect_mappings(
    messages: &mut [ReplicationMessage],
    entity_map: &ClientEntityMap,
) -> bincode::Result<()> {
    for message in &mut *messages {
        message.start_array();

        if let Some(mappings) = entity_map.get(&message.client_id) {
            for mapping in mappings {
                message.write_client_mapping(mapping)?;
            }
        }

        message.end_array()?;
    }
    Ok(())
}

/// Collect component changes into messages based on last acknowledged tick.
fn collect_changes(
    messages: &mut [ReplicationMessage],
    world: &World,
    system_tick: Tick,
    replication_rules: &ReplicationRules,
) -> bincode::Result<()> {
    for message in &mut *messages {
        message.start_array();
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
            for message in &mut *messages {
                message.start_entity_data(archetype_entity.entity());
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

                        for message in &mut *messages {
                            if ticks.is_changed(message.system_tick, system_tick) {
                                message.write_component(
                                    replication_info,
                                    replication_id,
                                    component,
                                )?;
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

                        for message in &mut *messages {
                            if ticks.is_changed(message.system_tick, system_tick) {
                                message.write_component(
                                    replication_info,
                                    replication_id,
                                    component,
                                )?;
                            }
                        }
                    }
                }
            }

            for message in &mut *messages {
                message.end_entity_data()?;
            }
        }
    }

    for message in &mut *messages {
        message.end_array()?;
    }

    Ok(())
}

/// Collect component removals into messages based on last acknowledged tick.
fn collect_removals(
    messages: &mut [ReplicationMessage],
    removal_trackers: &Query<(Entity, &RemovalTracker)>,
    system_tick: Tick,
) -> bincode::Result<()> {
    for message in &mut *messages {
        message.start_array();
    }

    for (entity, removal_tracker) in removal_trackers {
        for message in &mut *messages {
            message.start_entity_data(entity);
            for (&replication_id, &tick) in &removal_tracker.0 {
                if tick.is_newer_than(message.system_tick, system_tick) {
                    message.write_replication_id(replication_id)?;
                }
            }
            message.end_entity_data()?;
        }
    }

    for message in &mut *messages {
        message.end_array()?;
    }

    Ok(())
}

/// Collect entity despawns into messages based on last acknowledged tick.
fn collect_despawns(
    messages: &mut [ReplicationMessage],
    despawn_tracker: &DespawnTracker,
    system_tick: Tick,
) -> bincode::Result<()> {
    for message in &mut *messages {
        message.start_array();
    }

    for &(entity, tick) in &despawn_tracker.0 {
        for message in &mut *messages {
            if tick.is_newer_than(message.system_tick, system_tick) {
                message.write_entity(entity)?;
            }
        }
    }

    for message in &mut *messages {
        message.end_array()?;
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

/// A reusable message with replicated data for a client.
///
/// See also [Limits](../index.html#limits)
#[derive(Deref, DerefMut)]
pub(crate) struct ReplicationMessage {
    /// ID of a client for which this message is written.
    client_id: ClientId,

    /// Last system tick acknowledged by the client.
    ///
    /// Used for changes preparation.
    system_tick: Tick,

    /// Send message even if it doesn't contain replication data.
    ///
    /// See also [`Self::send_to`]
    send_empty: bool,

    /// Message data.
    #[deref]
    buffer: ReplicationBuffer,
}

impl ReplicationMessage {
    /// Creates a new message with assigned client ID.
    ///
    /// `replicon_tick` is the current tick that will be written into
    ///  the message to read by client on receive.
    ///
    /// `system_tick` is the last acknowledged system tick for this client.
    ///  Changes since this tick should be written into the message.
    ///
    /// If `send_empty` is set to `true`, then [`Self::send_to`]
    /// will send the message even if it doesn't contain any data.
    fn new(
        replicon_tick: RepliconTick,
        client_id: ClientId,
        system_tick: Tick,
        send_empty: bool,
    ) -> bincode::Result<Self> {
        Ok(Self {
            client_id,
            system_tick,
            send_empty,
            buffer: ReplicationBuffer::new(replicon_tick)?,
        })
    }

    /// Clears the message and assigns it to a different client ID.
    ///
    /// Keeps allocated capacity of the buffer.
    fn reset(
        &mut self,
        replicon_tick: RepliconTick,
        client_id: ClientId,
        system_tick: Tick,
        send_empty: bool,
    ) -> bincode::Result<()> {
        self.client_id = client_id;
        self.system_tick = system_tick;
        self.send_empty = send_empty;
        self.buffer.reset(replicon_tick)
    }

    /// Sends the message to the designated client.
    fn send_to(&mut self, server: &mut RenetServer) {
        if !self.buffer.contains_data() && !self.send_empty {
            trace!("no changes to send for client {}", self.client_id);
            return;
        }

        self.buffer.trim_empty_arrays();

        trace!("sending replication message to client {}", self.client_id);
        server.send_message(
            self.client_id,
            REPLICATION_CHANNEL_ID,
            Bytes::copy_from_slice(self.buffer.as_slice()),
        );
    }
}

/// Stores information about ticks.
///
/// Used only on server.
#[derive(Resource, Default)]
pub struct AckedTicks {
    /// Last acknowledged server ticks for all clients.
    clients: HashMap<ClientId, RepliconTick>,

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
    pub fn acked_ticks(&self) -> &HashMap<ClientId, RepliconTick> {
        &self.clients
    }
}

/// Contains the lowest replicon tick that should be acknowledged by clients.
///
/// If a client has not acked this tick, then replication messages >= this tick
/// will be sent even if they do not contain data.
///
/// Used to synchronize server-sent events with clients. A client cannot consume
/// a server-sent event until it has acknowledged the tick where that event was
/// created. This means we need to replicate ticks after a server-sent event is
/// emitted to guarantee the client can eventually consume the event.
#[derive(Clone, Copy, Debug, Default, Deref, DerefMut, Resource)]
pub(super) struct MinRepliconTick(RepliconTick);

/**
A resource that exists on the server for mapping server entities to
entities that clients have already spawned. The mappings are sent to clients as part of replication
and injected into the client's [`ServerEntityMap`](crate::client::ServerEntityMap).

Sometimes you don't want to wait for the server to spawn something before it appears on the
client – when a client performs an action, they can immediately simulate it on the client,
then match up that entity with the eventual replicated server spawn, rather than have replication spawn
a brand new entity on the client.

In this situation, the client can send the server its pre-spawned entity id, then the server can spawn its own entity
and inject the [`ClientMapping`] into its [`ClientEntityMap`].

Replication packets will send a list of such mappings to clients, which will
be inserted into the client's [`ServerEntityMap`](crate::client::ServerEntityMap). Using replication
to propagate the mappings ensures any replication messages related to the pre-mapped
server entities will synchronize with updating the client's [`ServerEntityMap`](crate::client::ServerEntityMap).

### Example:

```
use bevy::prelude::*;
use bevy_replicon::prelude::*;

#[derive(Event)]
struct SpawnBullet(Entity);

#[derive(Component)]
struct Bullet;

/// System that shoots a bullet and spawns it on the client.
fn shoot_system(mut commands: Commands, mut bullet_events: EventWriter<SpawnBullet>) {
    let entity = commands.spawn(Bullet).id();
    bullet_events.send(SpawnBullet(entity));
}

/// Validation to check if client is not cheating or the simulation is correct.
///
/// Depending on the type of game you may want to correct the client or disconnect it.
/// In this example we just always confirm the spawn.
fn confirm_bullet(
    mut commands: Commands,
    mut bullet_events: EventReader<FromClient<SpawnBullet>>,
    mut entity_map: ResMut<ClientEntityMap>,
    tick: Res<RepliconTick>,
) {
    for FromClient { client_id, event } in bullet_events.read() {
        let server_entity = commands.spawn(Bullet).id(); // You can insert more components, they will be sent to the client's entity correctly.

        entity_map.insert(
            *client_id,
            ClientMapping {
                tick: *tick,
                server_entity,
                client_entity: event.0,
            },
        );
    }
}
```

If the client is connected and receives the replication data for the server entity mapping,
replicated data will be applied to the client's original entity instead of spawning a new one.
You can detect when the mapping is replicated by querying for `Added<Replication>` on your original
client entity.

If client's original entity is not found, a new entity will be spawned on the client,
just the same as when no client entity is provided.
**/
#[derive(Resource, Debug, Default, Deref)]
pub struct ClientEntityMap(HashMap<ClientId, Vec<ClientMapping>>);

impl ClientEntityMap {
    /// Registers `mapping` for a client entity pre-spawned by the specified client.
    ///
    /// This will be sent as part of replication data and added to the client's [`ServerEntityMap`](crate::client::ServerEntityMap).
    pub fn insert(&mut self, client_id: ClientId, mapping: ClientMapping) {
        self.0.entry(client_id).or_default().push(mapping);
    }

    /// Removes acknowledged mappings.
    fn cleanup_acked(&mut self, client_id: ClientId, acked_tick: RepliconTick) {
        if let Some(mappings) = self.0.get_mut(&client_id) {
            mappings.retain(|mapping| mapping.tick > acked_tick);
        }
    }
}

/// Stores the server entity corresponding to a client's pre-spawned entity.
///
/// The `tick` is stored here so that this prediction data can be cleaned up once the tick
/// has been acked by the client.
#[derive(Debug)]
pub struct ClientMapping {
    pub tick: RepliconTick,
    pub server_entity: Entity,
    pub client_entity: Entity,
}
