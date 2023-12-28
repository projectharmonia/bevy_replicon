pub(super) mod clients_info;
pub(super) mod despawn_buffer;
pub(super) mod removal_buffer;
pub(super) mod replicated_archetypes_info;
pub(super) mod replication_buffer;
pub(super) mod replication_messages;

use std::{mem, time::Duration};

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
    utils::HashMap,
};
use bevy_renet::{
    renet::{ClientId, RenetClient, RenetServer, ServerEvent},
    transport::NetcodeServerPlugin,
    RenetReceive, RenetSend, RenetServerPlugin,
};

use crate::replicon_core::{
    replication_rules::ReplicationRules, replicon_tick::RepliconTick, ReplicationChannel,
};
use clients_info::{ClientBuffers, ClientInfo, ClientsInfo};
use despawn_buffer::{DespawnBuffer, DespawnBufferPlugin};
use removal_buffer::{RemovalBuffer, RemovalBufferPlugin};
use replicated_archetypes_info::ReplicatedArchetypesInfo;
use replication_messages::ReplicationMessages;

pub const SERVER_ID: ClientId = ClientId::from_raw(0);

pub struct ServerPlugin {
    /// Tick configuration.
    pub tick_policy: TickPolicy,

    /// The time after which updates will be considered lost if an acknowledgment is not received for them.
    ///
    /// In practice updates will live at least `update_timeout`, and at most `2*update_timeout`.
    pub update_timeout: Duration,
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self {
            tick_policy: TickPolicy::MaxTickRate(30),
            update_timeout: Duration::from_secs(10),
        }
    }
}

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            DespawnBufferPlugin,
            RemovalBufferPlugin,
            RenetServerPlugin,
            NetcodeServerPlugin,
        ))
        .init_resource::<ClientsInfo>()
        .init_resource::<ClientBuffers>()
        .init_resource::<LastChangeTick>()
        .init_resource::<ClientEntityMap>()
        .configure_sets(PreUpdate, ServerSet::Receive.after(RenetReceive))
        .configure_sets(PostUpdate, ServerSet::Send.before(RenetSend))
        .add_systems(
            PreUpdate,
            (
                Self::handle_connections_system,
                Self::acks_receiving_system,
                Self::acks_cleanup_system(self.update_timeout)
                    .run_if(on_timer(self.update_timeout)),
            )
                .chain()
                .in_set(ServerSet::Receive)
                .run_if(resource_exists::<RenetServer>()),
        )
        .add_systems(
            PostUpdate,
            (
                Self::replication_sending_system
                    .map(Result::unwrap)
                    .in_set(ServerSet::Send)
                    .run_if(resource_exists::<RenetServer>())
                    .run_if(resource_changed::<RepliconTick>()),
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
                        .run_if(resource_exists::<RenetServer>())
                        .run_if(on_timer(tick_time)),
                );
            }
            TickPolicy::EveryFrame => {
                app.add_systems(
                    PostUpdate,
                    Self::increment_tick
                        .before(Self::replication_sending_system)
                        .run_if(resource_exists::<RenetServer>()),
                );
            }
            TickPolicy::Manual => (),
        }
    }
}

impl ServerPlugin {
    /// Increments current server tick which causes the server to replicate this frame.
    pub fn increment_tick(mut replicon_tick: ResMut<RepliconTick>) {
        replicon_tick.increment();
        trace!("incremented {replicon_tick:?}");
    }

    fn handle_connections_system(
        mut server_events: EventReader<ServerEvent>,
        mut entity_map: ResMut<ClientEntityMap>,
        mut clients_info: ResMut<ClientsInfo>,
        mut client_buffers: ResMut<ClientBuffers>,
    ) {
        for event in server_events.read() {
            match *event {
                ServerEvent::ClientDisconnected { client_id, .. } => {
                    entity_map.0.remove(&client_id);
                    clients_info.remove(&mut client_buffers, client_id);
                }
                ServerEvent::ClientConnected { client_id } => {
                    clients_info.init(&mut client_buffers, client_id);
                }
            }
        }
    }

    fn acks_cleanup_system(
        update_timeout: Duration,
    ) -> impl FnMut(ResMut<ClientsInfo>, ResMut<ClientBuffers>, Res<Time>) {
        move |mut clients_info: ResMut<ClientsInfo>,
              mut client_buffers: ResMut<ClientBuffers>,
              time: Res<Time>| {
            let min_timestamp = time.elapsed().saturating_sub(update_timeout);
            for client_info in clients_info.iter_mut() {
                client_info.remove_older_updates(&mut client_buffers, min_timestamp);
            }
        }
    }

    fn acks_receiving_system(
        change_tick: SystemChangeTick,
        mut server: ResMut<RenetServer>,
        mut clients_info: ResMut<ClientsInfo>,
        mut client_buffers: ResMut<ClientBuffers>,
    ) {
        for client_info in clients_info.iter_mut() {
            while let Some(message) =
                server.receive_message(client_info.id(), ReplicationChannel::Reliable)
            {
                match bincode::deserialize::<u16>(&message) {
                    Ok(update_index) => {
                        client_info.acknowledge(
                            &mut client_buffers,
                            change_tick.this_run(),
                            update_index,
                        );
                    }
                    Err(e) => debug!(
                        "unable to deserialize update index from client {}: {e}",
                        client_info.id()
                    ),
                }
            }
        }
    }

    /// Collects [`ReplicationMessages`] and sends them.
    #[allow(clippy::type_complexity)]
    pub(super) fn replication_sending_system(
        mut messages: Local<ReplicationMessages>,
        mut archetypes_info: Local<ReplicatedArchetypesInfo>,
        change_tick: SystemChangeTick,
        mut set: ParamSet<(
            &World,
            ResMut<ClientsInfo>,
            ResMut<ClientEntityMap>,
            ResMut<DespawnBuffer>,
            ResMut<RemovalBuffer>,
            ResMut<LastChangeTick>,
            ResMut<ClientBuffers>,
            ResMut<RenetServer>,
        )>,
        replication_rules: Res<ReplicationRules>,
        replicon_tick: Res<RepliconTick>,
        time: Res<Time>,
    ) -> bincode::Result<()> {
        archetypes_info.update(set.p0().archetypes(), &replication_rules);

        let clients_info = mem::take(&mut *set.p1()); // Take ownership to avoid borrowing issues.
        messages.prepare(clients_info, *replicon_tick)?;

        collect_mappings(&mut messages, &mut set.p2())?;
        collect_changes(
            &mut messages,
            &archetypes_info,
            &replication_rules,
            set.p0(),
            &change_tick,
        )?;
        collect_despawns(&mut messages, &mut set.p3())?;
        collect_removals(&mut messages, &mut set.p4(), change_tick.this_run())?;

        let last_change_tick = *set.p5();
        let mut client_buffers = mem::take(&mut *set.p6());
        let (last_change_tick, clients_info) = messages.send(
            &mut set.p7(),
            &mut client_buffers,
            last_change_tick,
            *replicon_tick,
            change_tick.this_run(),
            time.elapsed(),
        )?;

        // Return borrowed data back.
        *set.p1() = clients_info;
        *set.p5() = last_change_tick;
        *set.p6() = client_buffers;

        Ok(())
    }

    fn reset_system(
        mut replicon_tick: ResMut<RepliconTick>,
        mut entity_map: ResMut<ClientEntityMap>,
        mut clients_info: ResMut<ClientsInfo>,
        mut client_buffers: ResMut<ClientBuffers>,
    ) {
        *replicon_tick = Default::default();
        entity_map.0.clear();
        clients_info.clear(&mut client_buffers);
    }
}

/// Collects and writes any new entity mappings that happened in this tick.
///
/// On deserialization mappings should be processed first, so all referenced entities after it will behave correctly.
fn collect_mappings(
    messages: &mut ReplicationMessages,
    entity_map: &mut ClientEntityMap,
) -> bincode::Result<()> {
    for (message, _, client_info) in messages.iter_mut_with_info() {
        message.start_array();

        if let Some(mappings) = entity_map.0.get_mut(&client_info.id()) {
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
    archetypes_info: &ReplicatedArchetypesInfo,
    replication_rules: &ReplicationRules,
    world: &World,
    change_tick: &SystemChangeTick,
) -> bincode::Result<()> {
    for (init_message, _) in messages.iter_mut() {
        init_message.start_array();
    }

    for archetype_info in archetypes_info.iter() {
        // SAFETY: all IDs from replicated archetypes obtained from real archetypes.
        let archetype = unsafe { world.archetypes().get(archetype_info.id).unwrap_unchecked() };
        // SAFETY: table obtained from this archetype.
        let table = unsafe {
            world
                .storages()
                .tables
                .get(archetype.table_id())
                .unwrap_unchecked()
        };

        for entity in archetype.entities() {
            for (init_message, update_message) in messages.iter_mut() {
                init_message.start_entity_data(entity.entity());
                update_message.start_entity_data(entity.entity())
            }

            // SAFETY: all replicated archetypes have marker component with table storage.
            let (_, marker_ticks) = unsafe {
                get_component_unchecked(
                    table,
                    &world.storages().sparse_sets,
                    entity,
                    StorageType::Table,
                    replication_rules.get_marker_id(),
                )
            };
            // If the marker was added in this tick, the entity just started replicating.
            // It could be a newly spawned entity or an old entity with just-enabled replication,
            // so we need to include even old components that were registered for replication.
            let new_entity = marker_ticks.is_added(change_tick.last_run(), change_tick.this_run());

            for component_info in &archetype_info.components {
                // SAFETY: component and storage were obtained from this archetype.
                let (component, ticks) = unsafe {
                    get_component_unchecked(
                        table,
                        &world.storages().sparse_sets,
                        entity,
                        component_info.storage_type,
                        component_info.component_id,
                    )
                };

                for (init_message, update_message, client_info) in messages.iter_mut_with_info() {
                    let must_init = new_entity || client_info.just_connected;

                    if must_init || ticks.is_added(change_tick.last_run(), change_tick.this_run()) {
                        init_message.write_component(
                            &component_info.replication_info,
                            component_info.replication_id,
                            component,
                        )?;
                    } else {
                        let tick = *client_info
                            .ticks
                            .get(&entity.entity())
                            .expect("entity should be present after adding component");
                        if ticks.is_changed(tick, change_tick.this_run()) {
                            update_message.write_component(
                                &component_info.replication_info,
                                component_info.replication_id,
                                component,
                            )?;
                        }
                    }
                }
            }

            for (init_message, update_message, client_info) in messages.iter_mut_with_info() {
                let must_init = new_entity || client_info.just_connected;

                if must_init || init_message.entity_data_len() != 0 {
                    // If there is any insertion or we must initialize, include all updates into init message
                    // and bump the last acknowledged tick to keep entity updates atomic.
                    init_message.take_entity_data(update_message);
                    client_info
                        .ticks
                        .insert(entity.entity(), change_tick.this_run());
                } else {
                    update_message.register_entity();
                    update_message.end_entity_data(false)?;
                }

                init_message.end_entity_data(must_init)?;
            }
        }
    }

    for (init_message, _, client_info) in messages.iter_mut_with_info() {
        client_info.just_connected = false;
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
            let component = sparse_set.get(entity.entity()).unwrap_unchecked();
            let ticks = sparse_set.get_ticks(entity.entity()).unwrap_unchecked();

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
        for (message, _, client_info) in messages.iter_mut_with_info() {
            client_info.ticks.remove(&entity);
            message.write_entity(entity)?;
        }
    }

    for (message, _) in messages.iter_mut() {
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

    for (entity, components) in removal_buffer.iter() {
        for (message, _, client_info) in messages.iter_mut_with_info() {
            message.start_entity_data(entity);
            for &replication_id in components {
                client_info.ticks.insert(entity, tick);
                message.write_replication_id(replication_id)?;
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

/// Contains the last tick in which a replicated entity was spawned, despawned, or gained/lost a component.
///
/// It should be included in update messages and server events instead of the current tick
/// to avoid needless waiting for the next init message to arrive.
#[derive(Clone, Copy, Debug, Default, Deref, Resource)]
pub struct LastChangeTick(RepliconTick);

/**
A resource that exists on the server for mapping server entities to
entities that clients have already spawned. The mappings are sent to clients as part of replication
and injected into the client's [`ServerEntityMap`](crate::client::client_mapper::ServerEntityMap).

Sometimes you don't want to wait for the server to spawn something before it appears on the
client – when a client performs an action, they can immediately simulate it on the client,
then match up that entity with the eventual replicated server spawn, rather than have replication spawn
a brand new entity on the client.

In this situation, the client can send the server its pre-spawned entity id, then the server can spawn its own entity
and inject the [`ClientMapping`] into its [`ClientEntityMap`].

Replication packets will send a list of such mappings to clients, which will
be inserted into the client's [`ServerEntityMap`](crate::client::client_mapper::ServerEntityMap). Using replication
to propagate the mappings ensures any replication messages related to the pre-mapped
server entities will synchronize with updating the client's [`ServerEntityMap`](crate::client::client_mapper::ServerEntityMap).

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
) {
    for FromClient { client_id, event } in bullet_events.read() {
        let server_entity = commands.spawn(Bullet).id(); // You can insert more components, they will be sent to the client's entity correctly.

        entity_map.insert(
            *client_id,
            ClientMapping {
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
    /// This will be sent as part of replication data and added to the client's [`ServerEntityMap`](crate::client::client_mapper::ServerEntityMap).
    pub fn insert(&mut self, client_id: ClientId, mapping: ClientMapping) {
        self.0.entry(client_id).or_default().push(mapping);
    }
}

/// Stores the server entity corresponding to a client's pre-spawned entity.
#[derive(Debug)]
pub struct ClientMapping {
    pub server_entity: Entity,
    pub client_entity: Entity,
}
