pub(super) mod clients_info;
pub(super) mod removals_reader;
pub(super) mod replication_buffer;
pub(super) mod replication_messages;

use std::{mem, time::Duration};

use bevy::{
    ecs::{
        archetype::ArchetypeId,
        component::{ComponentTicks, StorageType, Tick},
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
    replication_rules::{Replication, ReplicationId, ReplicationInfo, ReplicationRules},
    replicon_tick::RepliconTick,
    ReplicationChannel,
};
use clients_info::{ClientInfo, ClientsInfo};
use removals_reader::RemovedComponentIds;
use replication_messages::ReplicationMessages;

pub const SERVER_ID: ClientId = ClientId::from_raw(0);

pub struct ServerPlugin {
    tick_policy: TickPolicy,
    update_timeout: Duration,
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
        app.add_plugins((RenetServerPlugin, NetcodeServerPlugin))
            .init_resource::<ClientsInfo>()
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
    // TODO: better constructors or remove them at all.
    pub fn new(tick_policy: TickPolicy) -> Self {
        Self {
            tick_policy,
            ..Default::default()
        }
    }

    /// Increments current server tick which causes the server to replicate this frame.
    pub fn increment_tick(mut replicon_tick: ResMut<RepliconTick>) {
        replicon_tick.increment();
        trace!("incremented {replicon_tick:?}");
    }

    fn handle_connections_system(
        mut server_events: EventReader<ServerEvent>,
        mut entity_map: ResMut<ClientEntityMap>,
        mut clients_info: ResMut<ClientsInfo>,
    ) {
        for event in server_events.read() {
            match *event {
                ServerEvent::ClientDisconnected { client_id, .. } => {
                    entity_map.0.remove(&client_id);
                    clients_info.remove(client_id);
                }
                ServerEvent::ClientConnected { client_id } => {
                    clients_info.init(client_id);
                }
            }
        }
    }

    fn acks_cleanup_system(update_timeout: Duration) -> impl FnMut(ResMut<ClientsInfo>, Res<Time>) {
        move |mut clients_info: ResMut<ClientsInfo>, time: Res<Time>| {
            let ClientsInfo {
                info,
                entity_buffer,
                ..
            } = &mut *clients_info;

            let min_timestamp = time.elapsed() - update_timeout;
            for client_info in info {
                client_info.updates.retain(|_, update_info| {
                    if update_info.timestamp < min_timestamp {
                        entity_buffer.push(mem::take(&mut update_info.entities));
                        true
                    } else {
                        false
                    }
                });
            }
        }
    }

    fn acks_receiving_system(
        change_tick: SystemChangeTick,
        mut server: ResMut<RenetServer>,
        mut clients_info: ResMut<ClientsInfo>,
    ) {
        let ClientsInfo {
            info,
            entity_buffer,
            ..
        } = &mut *clients_info;

        for client_info in info.iter_mut() {
            while let Some(message) =
                server.receive_message(client_info.id, ReplicationChannel::Reliable)
            {
                match bincode::deserialize::<u16>(&message) {
                    Ok(update_index) => {
                        let Some(mut update_info) = client_info.updates.remove(&update_index)
                        else {
                            debug!(
                                "received unknown update index {update_index} from client {}",
                                client_info.id
                            );
                            continue;
                        };

                        for entity in &update_info.entities {
                            let last_tick = client_info
                                .ticks
                                .get_mut(entity)
                                .expect("tick should be inserted on any component insertion");

                            // Received tick could be outdated because we bump it
                            // if we detect any insertion on the entity in `collect_changes`.
                            if !last_tick.is_newer_than(update_info.tick, change_tick.this_run()) {
                                *last_tick = update_info.tick;
                            }
                        }
                        update_info.entities.clear();
                        entity_buffer.push(update_info.entities);

                        trace!(
                            "client {} acknowledged an update with {:?}",
                            client_info.id,
                            update_info.tick,
                        );
                    }
                    Err(e) => debug!(
                        "unable to deserialize update index from client {}: {e}",
                        client_info.id
                    ),
                }
            }
        }
    }

    /// Collects [`ReplicationMessages`] and sends them.
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub(super) fn replication_sending_system(
        mut messages: Local<ReplicationMessages>,
        change_tick: SystemChangeTick,
        mut removed_replication: RemovedComponents<Replication>,
        mut removed_ids: RemovedComponentIds,
        mut set: ParamSet<(
            &World,
            ResMut<ClientsInfo>,
            ResMut<ClientEntityMap>,
            ResMut<RenetServer>,
            ResMut<LastChangeTick>,
        )>,
        replication_rules: Res<ReplicationRules>,
        replicon_tick: Res<RepliconTick>,
        time: Res<Time>,
    ) -> bincode::Result<()> {
        let info = mem::take(&mut set.p1().info); // Take ownership to avoid borrowing issues.
        messages.prepare(info, *replicon_tick)?;

        collect_mappings(&mut messages, &mut set.p2())?;
        collect_changes(&mut messages, set.p0(), &change_tick, &replication_rules)?;
        collect_despawns(&mut messages, &mut removed_replication, &mut removed_ids)?;
        collect_removals(
            &mut messages,
            &mut removed_ids,
            change_tick.this_run(),
            &replication_rules,
        )?;

        let last_change_tick = *set.p4();
        let mut entity_buffer = mem::take(&mut set.p1().entity_buffer);
        let (last_change_tick, info) = messages.send(
            &mut set.p3(),
            &mut entity_buffer,
            last_change_tick,
            *replicon_tick,
            change_tick.this_run(),
            time.elapsed(),
        )?;

        // Return borrowed data back.
        set.p1().info = info;
        set.p1().entity_buffer = entity_buffer;
        *set.p4() = last_change_tick;

        Ok(())
    }

    fn reset_system(
        mut replicon_tick: ResMut<RepliconTick>,
        mut entity_map: ResMut<ClientEntityMap>,
        mut clients_info: ResMut<ClientsInfo>,
    ) {
        *replicon_tick = Default::default();
        entity_map.0.clear();
        clients_info.clear();
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

        if let Some(mappings) = entity_map.0.get_mut(&client_info.id) {
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
    world: &World,
    change_tick: &SystemChangeTick,
    replication_rules: &ReplicationRules,
) -> bincode::Result<()> {
    for (init_message, _) in messages.iter_mut() {
        init_message.start_array();
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
            for (init_message, update_message) in messages.iter_mut() {
                init_message.start_entity_data(archetype_entity.entity());
                update_message.start_entity_data(archetype_entity.entity())
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

                        collect_component_change(
                            messages,
                            archetype_entity.entity(),
                            ticks,
                            change_tick,
                            replication_info,
                            replication_id,
                            component,
                        )?;
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

                        collect_component_change(
                            messages,
                            entity,
                            ticks,
                            change_tick,
                            replication_info,
                            replication_id,
                            component,
                        )?;
                    }
                }
            }

            for (init_message, update_message, client_info) in messages.iter_mut_with_info() {
                if init_message.entity_data_len() != 0 {
                    // If there is any insertion, include all updates into init message
                    // and bump the last acknowledged tick to keep entity updates atomic.
                    init_message.take_entity_data(update_message);
                    client_info
                        .ticks
                        .insert(archetype_entity.entity(), change_tick.this_run());
                } else {
                    update_message.register_entity();
                    update_message.end_entity_data()?;
                }

                init_message.end_entity_data()?;
            }
        }
    }

    for (init_message, _, client_info) in messages.iter_mut_with_info() {
        client_info.just_connected = false;
        init_message.end_array()?;
    }

    Ok(())
}

/// Collects the component if it has been changed.
///
/// If the component was added since the client's last init message, it will be collected into
/// init buffer.
fn collect_component_change(
    messages: &mut ReplicationMessages,
    entity: Entity,
    ticks: ComponentTicks,
    change_tick: &SystemChangeTick,
    replication_info: &ReplicationInfo,
    replication_id: ReplicationId,
    component: Ptr,
) -> bincode::Result<()> {
    for (init_message, update_message, client_info) in messages.iter_mut_with_info() {
        if client_info.just_connected
            || ticks.is_added(change_tick.last_run(), change_tick.this_run())
        {
            init_message.write_component(replication_info, replication_id, component)?;
        } else {
            let tick = *client_info
                .ticks
                .get(&entity)
                .expect("entity should be present after adding component");
            if ticks.is_changed(tick, change_tick.this_run()) {
                update_message.write_component(replication_info, replication_id, component)?;
            }
        }
    }

    Ok(())
}

/// Collect entity despawns from this tick into init messages.
fn collect_despawns(
    messages: &mut ReplicationMessages,
    removed_replication: &mut RemovedComponents<Replication>,
    removed_ids: &mut RemovedComponentIds,
) -> bincode::Result<()> {
    for (message, _) in messages.iter_mut() {
        message.start_array();
    }

    for entity in removed_replication.read() {
        for (message, _, client_info) in messages.iter_mut_with_info() {
            client_info.ticks.remove(&entity);
            removed_ids.register_despawn(entity);
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
    removed_ids: &mut RemovedComponentIds,
    tick: Tick,
    replication_rules: &ReplicationRules,
) -> bincode::Result<()> {
    for (message, _) in messages.iter_mut() {
        message.start_array();
    }

    for (entity, components) in removed_ids.read(replication_rules) {
        for (message, _, client_info) in messages.iter_mut_with_info() {
            message.start_entity_data(entity);
            for &replication_id in components {
                client_info.ticks.insert(entity, tick);
                message.write_replication_id(replication_id)?;
            }
            message.end_entity_data()?;
        }
    }

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
