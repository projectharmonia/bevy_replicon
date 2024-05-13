pub mod confirmed;
pub mod diagnostics;
pub mod replicon_client;
pub mod server_entity_map;

use std::{io::Cursor, mem};

use bevy::{ecs::world::CommandQueue, prelude::*};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;
use varint_rs::VarintReader;

use crate::core::{
    command_markers::{CommandMarkers, EntityMarkers},
    common_conditions::{client_connected, client_just_connected, client_just_disconnected},
    replication_fns::{
        ctx::{DespawnCtx, RemoveCtx, WriteCtx},
        ReplicationFns,
    },
    replicon_channels::{ReplicationChannel, RepliconChannels},
    replicon_tick::RepliconTick,
    Replicated,
};
use confirmed::Confirmed;
use diagnostics::ClientStats;
use replicon_client::RepliconClient;
use server_entity_map::ServerEntityMap;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RepliconClient>()
            .init_resource::<ServerEntityMap>()
            .init_resource::<ServerInitTick>()
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
        mut queue: Local<CommandQueue>,
        mut entity_markers: Local<EntityMarkers>,
    ) -> bincode::Result<()> {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, mut buffered_updates: Mut<BufferedUpdates>| {
                    world.resource_scope(|world, command_markers: Mut<CommandMarkers>| {
                        world.resource_scope(|world, replication_fns: Mut<ReplicationFns>| {
                            let mut stats = world.remove_resource::<ClientStats>();
                            let mut params = ReceiveParams {
                                queue: &mut queue,
                                entity_markers: &mut entity_markers,
                                entity_map: &mut entity_map,
                                stats: stats.as_mut(),
                                command_markers: &command_markers,
                                replication_fns: &replication_fns,
                            };

                            apply_replication(
                                world,
                                &mut params,
                                &mut client,
                                &mut buffered_updates,
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
        mut init_tick: ResMut<ServerInitTick>,
        mut entity_map: ResMut<ServerEntityMap>,
        mut buffered_updates: ResMut<BufferedUpdates>,
    ) {
        *init_tick = Default::default();
        entity_map.clear();
        buffered_updates.clear();
    }
}

/// Reads all received messages and applies them.
///
/// Sends acknowledgments for update messages back.
fn apply_replication(
    world: &mut World,
    params: &mut ReceiveParams,
    client: &mut RepliconClient,
    buffered_updates: &mut BufferedUpdates,
) -> bincode::Result<()> {
    for message in client.receive(ReplicationChannel::Init) {
        apply_init_message(world, params, &message)?;
    }

    // Unlike init messages, we read all updates first, sort them by tick
    // in descending order to ensure that the last update will be applied first.
    // Since update messages manually split by packet size, we apply all messages,
    // but skip outdated data per-entity by checking last received tick for it
    // (unless user requested history via marker).
    let init_tick = *world.resource::<ServerInitTick>();
    let acks_size = mem::size_of::<u16>() * client.received_count(ReplicationChannel::Update);
    let mut acks = Vec::with_capacity(acks_size);
    for message in client.receive(ReplicationChannel::Update) {
        let update_index = read_update_message(params, buffered_updates, message)?;
        bincode::serialize_into(&mut acks, &update_index)?;
    }
    client.send(ReplicationChannel::Init, acks);

    apply_update_messages(world, params, buffered_updates, init_tick)
}

/// Applies [`InitMessage`](crate::server::replication_messages::InitMessage).
fn apply_init_message(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &[u8],
) -> bincode::Result<()> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(message);
    if let Some(stats) = &mut params.stats {
        stats.packets += 1;
        stats.bytes += end_pos;
    }

    let message_tick = bincode::deserialize_from(&mut cursor)?;
    trace!("applying init message for {message_tick:?}");
    world.resource_mut::<ServerInitTick>().0 = message_tick;
    debug_assert!(cursor.position() < end_pos, "init message can't be empty");

    apply_entity_mappings(world, params, &mut cursor)?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_despawns(world, params, &mut cursor, message_tick)?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_init_components(
        world,
        params,
        ComponentsKind::Removal,
        &mut cursor,
        message_tick,
    )?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_init_components(
        world,
        params,
        ComponentsKind::Insert,
        &mut cursor,
        message_tick,
    )?;

    Ok(())
}

/// Reads and buffers [`UpdateMessage`](crate::server::replication_messages::UpdateMessage).
///
/// Returns update index to be used for acknowledgment.
fn read_update_message(
    params: &mut ReceiveParams,
    buffered_updates: &mut BufferedUpdates,
    message: Bytes,
) -> bincode::Result<u16> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(&*message);
    if let Some(stats) = &mut params.stats {
        stats.packets += 1;
        stats.bytes += end_pos;
    }

    let (init_tick, message_tick, update_index) = bincode::deserialize_from(&mut cursor)?;
    trace!("received update message for {message_tick:?}");
    buffered_updates.insert(BufferedUpdate {
        init_tick,
        message_tick,
        message: message.slice(cursor.position() as usize..),
    });

    Ok(update_index)
}

/// Applies updates from [`BufferedUpdates`].
///
/// If the update message can't be applied yet (because the init message with the
/// corresponding tick hasn't arrived), it will be kept in the buffer.
fn apply_update_messages(
    world: &mut World,
    params: &mut ReceiveParams,
    buffered_updates: &mut BufferedUpdates,
    init_tick: ServerInitTick,
) -> bincode::Result<()> {
    let mut result = Ok(());
    buffered_updates.0.retain(|update| {
        if update.init_tick > *init_tick {
            return true;
        }

        trace!("applying update message for {:?}", update.message_tick);
        if let Err(e) = apply_update_components(
            world,
            params,
            &mut Cursor::new(&*update.message),
            update.message_tick,
        ) {
            result = Err(e);
        }

        false
    });

    result
}

/// Applies received server mappings from client's pre-spawned entities.
fn apply_entity_mappings(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    let mappings_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = &mut params.stats {
        stats.mappings += mappings_len as u32;
    }
    for _ in 0..mappings_len {
        let server_entity = deserialize_entity(cursor)?;
        let client_entity = deserialize_entity(cursor)?;

        if let Some(mut entity) = world.get_entity_mut(client_entity) {
            debug!("received mapping from {server_entity:?} to {client_entity:?}");
            entity.insert(Replicated);
            params.entity_map.insert(server_entity, client_entity);
        } else {
            // Entity could be despawned on client already.
            debug!("received mapping from {server_entity:?} to {client_entity:?}, but the entity doesn't exists");
        }
    }
    Ok(())
}

/// Deserializes replicated components of `components_kind` and applies them to the `world`.
fn apply_init_components(
    world: &mut World,
    params: &mut ReceiveParams,
    components_kind: ComponentsKind,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    for _ in 0..entities_len {
        let server_entity = deserialize_entity(cursor)?;
        let data_size: u16 = bincode::deserialize_from(&mut *cursor)?;

        let client_entity = params
            .entity_map
            .get_by_server_or_insert(server_entity, || world.spawn(Replicated).id());

        let world_cell = world.as_unsafe_world_cell();
        // SAFETY: access is unique and used to obtain `EntityMut`, which is just a wrapper over `UnsafeEntityCell`.
        let mut client_entity: EntityMut =
            unsafe { world_cell.world_mut().entity_mut(client_entity).into() };
        let mut commands = Commands::new_from_entities(params.queue, world_cell.entities());
        params
            .entity_markers
            .read(params.command_markers, &client_entity);

        if let Some(mut confirmed) = client_entity.get_mut::<Confirmed>() {
            confirmed.set_last_tick(message_tick);
        } else {
            commands
                .entity(client_entity.id())
                .insert(Confirmed::new(message_tick));
        }

        let end_pos = cursor.position() + data_size as u64;
        let mut components_len = 0u32;
        while cursor.position() < end_pos {
            let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            let (component_fns, rule_fns) = params.replication_fns.get(fns_id);
            match components_kind {
                ComponentsKind::Insert => {
                    let mut ctx = WriteCtx::new(&mut commands, params.entity_map, message_tick);

                    // SAFETY: `rule_fns` and `component_fns` were created for the same type.
                    unsafe {
                        component_fns.write(
                            &mut ctx,
                            rule_fns,
                            params.entity_markers,
                            &mut client_entity,
                            cursor,
                        )?;
                    }
                }
                ComponentsKind::Removal => {
                    let mut ctx = RemoveCtx::new(&mut commands, message_tick);
                    component_fns.remove(&mut ctx, params.entity_markers, &mut client_entity);
                }
            }
            components_len += 1;
        }

        if let Some(stats) = &mut params.stats {
            stats.entities_changed += 1;
            stats.components_changed += components_len;
        }

        params.queue.apply(world);
    }

    Ok(())
}

/// Deserializes despawns and applies them to the `world`.
fn apply_despawns(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = &mut params.stats {
        stats.despawns += entities_len as u32;
    }
    for _ in 0..entities_len {
        // The entity might have already been despawned because of hierarchy or
        // with the last replication message, but the server might not yet have received confirmation
        // from the client and could include the deletion in the this message.
        let server_entity = deserialize_entity(cursor)?;
        if let Some(client_entity) = params
            .entity_map
            .remove_by_server(server_entity)
            .and_then(|entity| world.get_entity_mut(entity))
        {
            let ctx = DespawnCtx { message_tick };
            (params.replication_fns.despawn)(&ctx, client_entity);
        }
    }

    Ok(())
}

///  Deserializes replicated component updates and applies them to the `world`.
///
/// Consumes all remaining bytes in the cursor.
fn apply_update_components(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let message_end = cursor.get_ref().len() as u64;
    while cursor.position() < message_end {
        let server_entity = deserialize_entity(cursor)?;
        let data_size: u16 = bincode::deserialize_from(&mut *cursor)?;

        let Some(client_entity) = params.entity_map.get_by_server(server_entity) else {
            // Update could arrive after a despawn from init message.
            debug!("ignoring update received for unknown server's {server_entity:?}");
            cursor.set_position(cursor.position() + data_size as u64);
            continue;
        };

        let world_cell = world.as_unsafe_world_cell();
        // SAFETY: access is unique and used to obtain `EntityMut`, which is just a wrapper over `UnsafeEntityCell`.
        let mut client_entity: EntityMut =
            unsafe { world_cell.world_mut().entity_mut(client_entity).into() };
        let mut commands = Commands::new_from_entities(params.queue, world_cell.entities());
        params
            .entity_markers
            .read(params.command_markers, &client_entity);

        let mut confirmed = client_entity
            .get_mut::<Confirmed>()
            .expect("all entities from update should have confirmed ticks");
        let new_entity = message_tick > confirmed.last_tick();
        if new_entity {
            confirmed.set_last_tick(message_tick);
        } else {
            if !params.entity_markers.need_history() {
                trace!(
                    "ignoring outdated update for client's {:?}",
                    client_entity.id()
                );
                cursor.set_position(cursor.position() + data_size as u64);
                continue;
            }

            let ago = confirmed.last_tick().get().wrapping_sub(message_tick.get());
            if ago >= u64::BITS {
                trace!(
                    "discarding update {ago} ticks old for client's {:?}",
                    client_entity.id()
                );
                cursor.set_position(cursor.position() + data_size as u64);
                continue;
            }

            confirmed.set(ago);
        }

        let end_pos = cursor.position() + data_size as u64;
        let mut components_count = 0u32;
        while cursor.position() < end_pos {
            let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            let (component_fns, rule_fns) = params.replication_fns.get(fns_id);
            let mut ctx = WriteCtx::new(&mut commands, params.entity_map, message_tick);

            // SAFETY: `rule_fns` and `component_fns` were created for the same type.
            unsafe {
                if new_entity {
                    component_fns.write(
                        &mut ctx,
                        rule_fns,
                        params.entity_markers,
                        &mut client_entity,
                        cursor,
                    )?;
                } else {
                    component_fns.consume_or_write(
                        &mut ctx,
                        rule_fns,
                        params.entity_markers,
                        params.command_markers,
                        &mut client_entity,
                        cursor,
                    )?;
                }
            }

            components_count += 1;
        }

        if let Some(stats) = &mut params.stats {
            stats.entities_changed += 1;
            stats.components_changed += components_count;
        }

        params.queue.apply(world);
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

/// Borrowed resources from the world and locals.
///
/// To avoid passing a lot of arguments into all receive functions.
struct ReceiveParams<'a> {
    queue: &'a mut CommandQueue,
    entity_markers: &'a mut EntityMarkers,
    entity_map: &'a mut ServerEntityMap,
    stats: Option<&'a mut ClientStats>,
    command_markers: &'a CommandMarkers,
    replication_fns: &'a ReplicationFns,
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

/// Last received tick for init message from server.
///
/// In other words, last [`RepliconTick`] with a removal, insertion, spawn or despawn.
/// When a component changes, this value is not updated.
#[derive(Clone, Copy, Debug, Default, Deref, Resource)]
pub struct ServerInitTick(RepliconTick);

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

    /// Inserts a new update, maintaining sorting by their message tick in descending order.
    fn insert(&mut self, update: BufferedUpdate) {
        let index = self
            .0
            .partition_point(|other_update| update.message_tick < other_update.message_tick);
        self.0.insert(index, update);
    }
}

/// Caches a partially-deserialized entity update message that is waiting for its tick to appear in an init message.
///
/// See also [`crate::server::replication_messages::UpdateMessage`].
pub(super) struct BufferedUpdate {
    /// Required tick to wait for.
    init_tick: RepliconTick,

    /// The tick this update corresponds to.
    message_tick: RepliconTick,

    /// Update data.
    message: Bytes,
}
