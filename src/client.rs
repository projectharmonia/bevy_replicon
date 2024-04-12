pub mod client_mapper;
pub mod diagnostics;
pub mod replicon_client;

use std::io::Cursor;

use bevy::{
    ecs::{entity::EntityHashMap, system::SystemState},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;
use varint_rs::VarintReader;

use crate::core::{
    common_conditions::{client_connected, client_just_connected, client_just_disconnected},
    replication_fns::ReplicationFns,
    replicon_channels::{ReplicationChannel, RepliconChannels},
    replicon_tick::RepliconTick,
    Replication,
};
use client_mapper::ServerEntityMap;
use diagnostics::ClientStats;
use replicon_client::RepliconClient;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RepliconClient>()
            .init_resource::<ServerEntityMap>()
            .init_resource::<ServerEntityTicks>()
            .init_resource::<BufferedUpdates>()
            .configure_sets(
                PreUpdate,
                (
                    ClientSet::ReceivePackets,
                    (
                        ClientSet::ResetEvents.run_if(client_just_connected),
                        ClientSet::Reset.run_if(client_just_disconnected),
                    ),
                    ClientSet::Receive,
                )
                    .chain(),
            )
            .configure_sets(
                PostUpdate,
                (ClientSet::Send, ClientSet::SendPackets).chain(),
            )
            .add_systems(Startup, Self::setup_channels)
            .add_systems(
                PreUpdate,
                Self::receive_replication
                    .map(Result::unwrap)
                    .in_set(ClientSet::Receive)
                    .run_if(client_connected),
            )
            .add_systems(PreUpdate, Self::reset.in_set(ClientSet::Reset));
    }
}

impl ClientPlugin {
    fn setup_channels(mut client: ResMut<RepliconClient>, channels: Res<RepliconChannels>) {
        client.setup_server_channels(channels.server_channels().len());
    }

    /// Receives and applies replication messages from the server.
    ///
    /// Tick init messages are sent over the [`ReplicationChannel::Init`] and are applied first to ensure valid state
    /// for entity updates.
    ///
    /// Entity update messages are sent over [`ReplicationChannel::Update`], which means they may appear
    /// ahead-of or behind init messages from the same server tick. An update will only be applied if its
    /// change tick has already appeared in an init message, otherwise it will be buffered while waiting.
    /// Since entity updates can arrive in any order, updates will only be applied if they correspond to a more
    /// recent server tick than the last acked server tick for each entity.
    ///
    /// Buffered entity update messages are processed last.
    ///
    /// Acknowledgments for received entity update messages are sent back to the server.
    ///
    /// See also [`ReplicationMessages`](crate::server::replication_messages::ReplicationMessages).
    pub(super) fn receive_replication(
        world: &mut World,
        state: &mut SystemState<(Commands, Query<EntityMut>)>,
    ) -> bincode::Result<()> {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, mut entity_ticks: Mut<ServerEntityTicks>| {
                    world.resource_scope(|world, mut buffered_updates: Mut<BufferedUpdates>| {
                        world.resource_scope(|world, replication_fns: Mut<ReplicationFns>| {
                            let mut stats = world.remove_resource::<ClientStats>();
                            apply_replication(
                                world,
                                state,
                                &mut client,
                                &mut entity_map,
                                &mut entity_ticks,
                                &mut buffered_updates,
                                stats.as_mut(),
                                &replication_fns,
                            )?;

                            if let Some(stats) = stats {
                                world.insert_resource(stats);
                            }

                            Ok(())
                        })
                    })
                })
            })
        })
    }

    fn reset(
        mut replicon_tick: ResMut<RepliconTick>,
        mut entity_map: ResMut<ServerEntityMap>,
        mut entity_ticks: ResMut<ServerEntityTicks>,
        mut buffered_updates: ResMut<BufferedUpdates>,
    ) {
        *replicon_tick = Default::default();
        entity_map.clear();
        entity_ticks.clear();
        buffered_updates.clear();
    }
}

/// Reads all received messages and applies them.
///
/// Sends acknowledgments for update messages back.
fn apply_replication(
    world: &mut World,
    state: &mut SystemState<(Commands, Query<EntityMut>)>,
    client: &mut RepliconClient,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    buffered_updates: &mut BufferedUpdates,
    mut stats: Option<&mut ClientStats>,
    replication_fns: &ReplicationFns,
) -> Result<(), Box<bincode::ErrorKind>> {
    while let Some(message) = client.receive(ReplicationChannel::Init) {
        apply_init_message(
            &message,
            world,
            state,
            entity_map,
            entity_ticks,
            stats.as_deref_mut(),
            replication_fns,
        )?;
    }

    let replicon_tick = *world.resource::<RepliconTick>();
    while let Some(message) = client.receive(ReplicationChannel::Update) {
        let index = apply_update_message(
            message,
            world,
            state,
            entity_map,
            entity_ticks,
            buffered_updates,
            stats.as_deref_mut(),
            replication_fns,
            replicon_tick,
        )?;

        client.send(ReplicationChannel::Init, bincode::serialize(&index)?)
    }

    let mut result = Ok(());
    buffered_updates.0.retain(|update| {
        if update.change_tick > replicon_tick {
            return true;
        }

        trace!("applying buffered update message for {replicon_tick:?}");
        if let Err(e) = apply_update_components(
            &mut Cursor::new(&*update.message),
            world,
            state,
            entity_map,
            entity_ticks,
            stats.as_deref_mut(),
            replication_fns,
            update.message_tick,
        ) {
            result = Err(e);
        }

        false
    });
    result?;

    Ok(())
}

/// Applies [`InitMessage`](crate::server::replication_messages::InitMessage).
fn apply_init_message(
    message: &[u8],
    world: &mut World,
    state: &mut SystemState<(Commands, Query<EntityMut>)>,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    replication_fns: &ReplicationFns,
) -> bincode::Result<()> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(message);
    if let Some(stats) = &mut stats {
        stats.packets += 1;
        stats.bytes += end_pos;
    }

    let replicon_tick = bincode::deserialize_from(&mut cursor)?;
    trace!("applying init message for {replicon_tick:?}");
    *world.resource_mut::<RepliconTick>() = replicon_tick;
    debug_assert!(cursor.position() < end_pos, "init message can't be empty");

    apply_entity_mappings(&mut cursor, world, entity_map, stats.as_deref_mut())?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_despawns(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats.as_deref_mut(),
        replication_fns,
        replicon_tick,
    )?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_init_components(
        &mut cursor,
        world,
        state,
        entity_map,
        entity_ticks,
        stats.as_deref_mut(),
        ComponentsKind::Removal,
        replication_fns,
        replicon_tick,
    )?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_init_components(
        &mut cursor,
        world,
        state,
        entity_map,
        entity_ticks,
        stats,
        ComponentsKind::Insert,
        replication_fns,
        replicon_tick,
    )?;

    Ok(())
}

/// Applies [`UpdateMessage`](crate::server::replication_messages::UpdateMessage).
///
/// If the update message can't be applied yet (because the init message with the
/// corresponding tick hasn't arrived), it will be buffered.
///
/// Returns update index to be used for acknowledgment.
fn apply_update_message(
    message: Bytes,
    world: &mut World,
    state: &mut SystemState<(Commands, Query<EntityMut>)>,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    buffered_updates: &mut BufferedUpdates,
    mut stats: Option<&mut ClientStats>,
    replication_fns: &ReplicationFns,
    replicon_tick: RepliconTick,
) -> bincode::Result<u16> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(&*message);
    if let Some(stats) = &mut stats {
        stats.packets += 1;
        stats.bytes += end_pos;
    }

    let (change_tick, message_tick, update_index) = bincode::deserialize_from(&mut cursor)?;
    if change_tick > replicon_tick {
        trace!("buffering update message for {message_tick:?}");
        buffered_updates.0.push(BufferedUpdate {
            change_tick,
            message_tick,
            message: message.slice(cursor.position() as usize..),
        });
        return Ok(update_index);
    }

    trace!("applying update message for {message_tick:?}");
    apply_update_components(
        &mut cursor,
        world,
        state,
        entity_map,
        entity_ticks,
        stats,
        replication_fns,
        message_tick,
    )?;

    Ok(update_index)
}

/// Applies received server mappings from client's pre-spawned entities.
fn apply_entity_mappings(
    cursor: &mut Cursor<&[u8]>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    stats: Option<&mut ClientStats>,
) -> bincode::Result<()> {
    let mappings_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = stats {
        stats.mappings += mappings_len as u32;
    }
    for _ in 0..mappings_len {
        let server_entity = deserialize_entity(cursor)?;
        let client_entity = deserialize_entity(cursor)?;

        if let Some(mut entity) = world.get_entity_mut(client_entity) {
            debug!("received mapping from {server_entity:?} to {client_entity:?}");
            entity.insert(Replication);
            entity_map.insert(server_entity, client_entity);
        } else {
            // Entity could be despawned on client already.
            debug!("received mapping from {server_entity:?} to {client_entity:?}, but the entity doesn't exists");
        }
    }
    Ok(())
}

/// Deserializes replicated components of `components_kind` and applies them to the `world`.
fn apply_init_components(
    cursor: &mut Cursor<&[u8]>,
    world: &mut World,
    state: &mut SystemState<(Commands, Query<EntityMut>)>,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    components_kind: ComponentsKind,
    replication_fns: &ReplicationFns,
    replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    for _ in 0..entities_len {
        let entity = deserialize_entity(cursor)?;
        let data_size: u16 = bincode::deserialize_from(&mut *cursor)?;

        let entity = entity_map.get_by_server_or_insert(entity, || world.spawn(Replication).id());
        let (mut commands, mut query) = state.get_mut(world);
        let mut entity = query.get_mut(entity).unwrap();
        entity_ticks.insert(entity.id(), replicon_tick);

        let end_pos = cursor.position() + data_size as u64;
        let mut components_len = 0u32;
        while cursor.position() < end_pos {
            let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            let serde_fns = replication_fns.serde_fns(fns_id);
            let command_fns = replication_fns.command_fns(serde_fns.commands_id());
            match components_kind {
                ComponentsKind::Insert => unsafe {
                    // SAFETY: User ensured that the registered write function can
                    // safely call its deserialize function.
                    command_fns.write(
                        serde_fns,
                        &mut commands,
                        &mut entity,
                        cursor,
                        entity_map,
                        replicon_tick,
                    )?
                },
                ComponentsKind::Removal => {
                    command_fns.remove(commands.entity(entity.id()), replicon_tick)
                }
            }
            components_len += 1;
        }
        if let Some(stats) = &mut stats {
            stats.entities_changed += 1;
            stats.components_changed += components_len;
        }
        state.apply(world);
    }

    Ok(())
}

/// Deserializes despawns and applies them to the `world`.
fn apply_despawns(
    cursor: &mut Cursor<&[u8]>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    stats: Option<&mut ClientStats>,
    replication_fns: &ReplicationFns,
    replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = stats {
        stats.despawns += entities_len as u32;
    }
    for _ in 0..entities_len {
        // The entity might have already been despawned because of hierarchy or
        // with the last replication message, but the server might not yet have received confirmation
        // from the client and could include the deletion in the this message.
        let server_entity = deserialize_entity(cursor)?;
        if let Some(client_entity) = entity_map
            .remove_by_server(server_entity)
            .and_then(|entity| world.get_entity_mut(entity))
        {
            entity_ticks.remove(&client_entity.id());
            (replication_fns.despawn)(client_entity, replicon_tick);
        }
    }

    Ok(())
}

///  Deserializes replicated component updates and applies them to the `world`.
///
/// Consumes all remaining bytes in the cursor.
fn apply_update_components(
    cursor: &mut Cursor<&[u8]>,
    world: &mut World,
    state: &mut SystemState<(Commands, Query<EntityMut>)>,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    replication_fns: &ReplicationFns,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let message_end = cursor.get_ref().len() as u64;
    while cursor.position() < message_end {
        let entity = deserialize_entity(cursor)?;
        let data_size: u16 = bincode::deserialize_from(&mut *cursor)?;

        let Some(entity) = entity_map.get_by_server(entity) else {
            // Update could arrive after a despawn from init message.
            debug!("ignoring update received for unknown server's {entity:?}");
            cursor.set_position(cursor.position() + data_size as u64);
            continue;
        };
        let entity_tick = entity_ticks
            .get_mut(&entity)
            .expect("all entities from update should have assigned ticks");
        if message_tick <= *entity_tick {
            trace!("ignoring outdated update for client's {entity:?}");
            cursor.set_position(cursor.position() + data_size as u64);
            continue;
        }
        *entity_tick = message_tick;

        let entity = entity_map.get_by_server_or_insert(entity, || world.spawn(Replication).id());
        let (mut commands, mut query) = state.get_mut(world);
        let mut entity = query.get_mut(entity).unwrap();

        let end_pos = cursor.position() + data_size as u64;
        let mut components_count = 0u32;
        while cursor.position() < end_pos {
            let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            let serde_fns = replication_fns.serde_fns(fns_id);
            let command_fns = replication_fns.command_fns(serde_fns.commands_id());
            unsafe {
                command_fns.write(
                    serde_fns,
                    &mut commands,
                    &mut entity,
                    cursor,
                    entity_map,
                    message_tick,
                )?;
            }
            components_count += 1;
        }
        if let Some(stats) = &mut stats {
            stats.entities_changed += 1;
            stats.components_changed += components_count;
        }
        state.apply(world);
    }

    Ok(())
}

/// Deserializes `entity` from compressed index and generation.
///
/// For details see
/// [`ReplicationBuffer::write_entity`](crate::server::replication_message::replication_buffer::write_entity).
fn deserialize_entity(cursor: &mut Cursor<&[u8]>) -> bincode::Result<Entity> {
    let flagged_index: u64 = cursor.read_u64_varint()?;
    let has_generation = (flagged_index & 1) > 0;
    let generation = if has_generation {
        cursor.read_u32_varint()? + 1
    } else {
        1u32
    };

    let bits = (generation as u64) << 32 | (flagged_index >> 1);

    Ok(Entity::from_bits(bits))
}

/// Type of components replication.
///
/// Parameter for [`apply_components`].
enum ComponentsKind {
    Insert,
    Removal,
}

/// Set with replication and event systems related to client.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ClientSet {
    /// Systems that receive packets from the messaging backend.
    ///
    /// Used by messaging backend implementations.
    ///
    /// Runs in [`PreUpdate`].
    ReceivePackets,
    /// Systems that receive data from [`RepliconClient`].
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PreUpdate`].
    Receive,
    /// Systems that send data to [`RepliconClient`].
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PostUpdate`].
    Send,
    /// Systems that send packets to the messaging backend.
    ///
    /// Used by messaging backend implementations.
    ///
    /// Runs in [`PostUpdate`].
    SendPackets,
    /// Systems that reset queued server events.
    ///
    /// Runs in [`PreUpdate`] immediately after the client connects to ensure client sessions have a fresh start.
    ///
    /// This is a separate set from [`ClientSet::Reset`] because the reset requirements for events are different
    /// from the replicon client internals.
    /// It is best practice to discard client-sent and server-received events while the client is not connected
    /// in order to guarantee clean separation between connection sessions.
    ResetEvents,
    /// Systems that reset the client.
    ///
    /// Runs in [`PreUpdate`] when the client just disconnected.
    ///
    /// You may want to disable this set if you want to preserve client replication state across reconnects.
    /// In that case, you need to manually repair the client state (or use something like
    /// [`bevy_replicon_repair`](https://docs.rs/bevy_replicon_repair)).
    ///
    /// If this set is disabled and you don't want to repair client state, then you need to manually clean up
    /// the client after a disconnect or when reconnecting.
    Reset,
}

/// Last received tick for each entity.
///
/// Used to avoid applying old updates.
///
/// If [`ClientSet::Reset`] is disabled, then this needs to be cleaned up manually.
#[derive(Default, Deref, DerefMut, Resource)]
pub struct ServerEntityTicks(EntityHashMap<RepliconTick>);

/// All cached buffered updates, used by the replicon client to align replication updates with initialization
/// messages.
///
/// If [`ClientSet::Reset`] is disabled, then this needs to be cleaned up manually with [`Self::clear`].
#[derive(Default, Resource)]
pub struct BufferedUpdates(Vec<BufferedUpdate>);

impl BufferedUpdates {
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Caches a partially-deserialized entity update message that is waiting for its tick to appear in an init message.
///
/// See also [`crate::server::replication_messages::UpdateMessage`].
pub(super) struct BufferedUpdate {
    /// Required tick to wait for.
    change_tick: RepliconTick,

    /// The tick this update corresponds to.
    message_tick: RepliconTick,

    /// Update data.
    message: Bytes,
}
