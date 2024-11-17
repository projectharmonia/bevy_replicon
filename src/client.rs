pub mod confirm_history;
#[cfg(feature = "client_diagnostics")]
pub mod diagnostics;
pub mod events;

use std::{io::Cursor, mem};

use bevy::{ecs::world::CommandQueue, prelude::*};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;
use integer_encoding::{FixedIntReader, VarIntReader, VarIntWriter};

use crate::core::{
    channels::{ReplicationChannel, RepliconChannels},
    common_conditions::{client_connected, client_just_connected, client_just_disconnected},
    replication::{
        change_message_arrays::ChangeMessageArrays,
        command_markers::{CommandMarkers, EntityMarkers},
        deferred_entity::DeferredEntity,
        replication_registry::{
            ctx::{DespawnCtx, RemoveCtx, WriteCtx},
            ReplicationRegistry,
        },
        Replicated,
    },
    replicon_client::RepliconClient,
    replicon_tick::RepliconTick,
    server_entity_map::ServerEntityMap,
};
use confirm_history::ConfirmHistory;

/// Client functionality and replication receiving.
///
/// Can be disabled for server-only apps.
pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RepliconClient>()
            .init_resource::<ServerEntityMap>()
            .init_resource::<ServerChangeTick>()
            .init_resource::<BufferedMutations>()
            .configure_sets(
                PreUpdate,
                (
                    ClientSet::ReceivePackets,
                    (
                        ClientSet::ResetEvents.run_if(client_just_connected),
                        ClientSet::Reset.run_if(client_just_disconnected),
                    ),
                    ClientSet::Receive,
                    ClientSet::SyncHierarchy,
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
    /// Change messages are sent over the [`ReplicationChannel::Changes`] and are applied first to ensure valid state
    /// for component mutations.
    ///
    /// Mutate messages are sent over [`ReplicationChannel::Mutations`], which means they may appear
    /// ahead-of or behind change messages from the same server tick. A mutation will only be applied if its
    /// change tick has already appeared in an change message, otherwise it will be buffered while waiting.
    /// Since component mutations can arrive in any order, they will only be applied if correspond to a more
    /// recent server tick than the last acked server tick for each entity.
    ///
    /// Buffered mutate messages are processed last.
    ///
    /// Acknowledgments for received mutate messages are sent back to the server.
    ///
    /// See also [`ReplicationMessages`](crate::server::replication_messages::ReplicationMessages).
    pub(super) fn receive_replication(
        world: &mut World,
        mut queue: Local<CommandQueue>,
        mut entity_markers: Local<EntityMarkers>,
    ) -> bincode::Result<()> {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, mut buffered_mutations: Mut<BufferedMutations>| {
                    world.resource_scope(|world, command_markers: Mut<CommandMarkers>| {
                        world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                            let mut stats = world.remove_resource::<ClientStats>();
                            let mut params = ReceiveParams {
                                queue: &mut queue,
                                entity_markers: &mut entity_markers,
                                entity_map: &mut entity_map,
                                stats: stats.as_mut(),
                                command_markers: &command_markers,
                                registry: &registry,
                            };

                            apply_replication(
                                world,
                                &mut params,
                                &mut client,
                                &mut buffered_mutations,
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
        mut change_tick: ResMut<ServerChangeTick>,
        mut entity_map: ResMut<ServerEntityMap>,
        mut buffered_mutations: ResMut<BufferedMutations>,
    ) {
        *change_tick = Default::default();
        entity_map.clear();
        buffered_mutations.clear();
    }
}

/// Reads all received messages and applies them.
///
/// Sends acknowledgments for mutate messages back.
fn apply_replication(
    world: &mut World,
    params: &mut ReceiveParams,
    client: &mut RepliconClient,
    buffered_mutations: &mut BufferedMutations,
) -> bincode::Result<()> {
    for message in client.receive(ReplicationChannel::Changes) {
        apply_change_message(world, params, &message)?;
    }

    // Unlike change messages, we read all mutate messages first, sort them by tick
    // in descending order to ensure that the last mutation will be applied first.
    // Since mutate messages manually split by packet size, we apply all messages,
    // but skip outdated data per-entity by checking last received tick for it
    // (unless user requested history via marker).
    let change_tick = *world.resource::<ServerChangeTick>();
    let acks_size = mem::size_of::<u16>() * client.received_count(ReplicationChannel::Mutations);
    if acks_size != 0 {
        let mut acks = Vec::with_capacity(acks_size);
        for message in client.receive(ReplicationChannel::Mutations) {
            let mutate_index = buffer_mutate_message(params, buffered_mutations, message)?;
            acks.write_varint(mutate_index)?;
        }
        client.send(ReplicationChannel::Changes, acks);
    }

    apply_mutate_messages(world, params, buffered_mutations, change_tick)
}

/// Reads and applies change message.
///
/// For details see [`replication_messages`](crate::server::replication_messages).
fn apply_change_message(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &[u8],
) -> bincode::Result<()> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(message);
    if let Some(stats) = &mut params.stats {
        stats.messages += 1;
        stats.bytes += end_pos;
    }

    let arrays = ChangeMessageArrays::from_bits_retain(cursor.read_fixedint()?);
    debug_assert!(!arrays.is_empty(), "message can't be empty");

    let message_tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    trace!("applying change message for {message_tick:?}");
    world.resource_mut::<ServerChangeTick>().0 = message_tick;

    let last_array = arrays.last();
    for (_, array) in arrays.iter_names() {
        match array {
            ChangeMessageArrays::MAPPINGS => {
                let len = if array != last_array {
                    apply_sized_array(&mut cursor, |cursor| {
                        apply_entity_mapping(world, params, cursor)
                    })?
                } else {
                    apply_dyn_array(&mut cursor, |cursor| {
                        apply_entity_mapping(world, params, cursor)
                    })?
                };
                if let Some(stats) = &mut params.stats {
                    stats.mappings += len as u32;
                }
            }
            ChangeMessageArrays::DESPAWNS => {
                let len = if array != last_array {
                    apply_sized_array(&mut cursor, |cursor| {
                        apply_despawn(world, params, cursor, message_tick)
                    })?
                } else {
                    apply_dyn_array(&mut cursor, |cursor| {
                        apply_despawn(world, params, cursor, message_tick)
                    })?
                };
                if let Some(stats) = &mut params.stats {
                    stats.despawns += len as u32;
                }
            }
            ChangeMessageArrays::REMOVALS => {
                let len = if array != last_array {
                    apply_sized_array(&mut cursor, |cursor| {
                        apply_removals(world, params, cursor, message_tick)
                    })?
                } else {
                    apply_dyn_array(&mut cursor, |cursor| {
                        apply_removals(world, params, cursor, message_tick)
                    })?
                };
                if let Some(stats) = &mut params.stats {
                    stats.entities_changed += len as u32;
                }
            }
            ChangeMessageArrays::CHANGES => {
                let len = apply_dyn_array(&mut cursor, |cursor| {
                    apply_changes(world, params, cursor, message_tick)
                })?;
                if let Some(stats) = &mut params.stats {
                    stats.entities_changed += len as u32;
                }
            }
            _ => unreachable!("iteration should yield only named flags"),
        }
    }

    Ok(())
}

/// Reads and buffers mutate message.
///
/// For details see [`replication_messages`](crate::server::replication_messages).
///
/// Returns mutate index to be used for acknowledgment.
fn buffer_mutate_message(
    params: &mut ReceiveParams,
    buffered_mutations: &mut BufferedMutations,
    message: Bytes,
) -> bincode::Result<u16> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(&*message);
    if let Some(stats) = &mut params.stats {
        stats.messages += 1;
        stats.bytes += end_pos;
    }

    let change_tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    let message_tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    let mutate_index = cursor.read_varint()?;
    trace!("received mutate message for {message_tick:?}");
    buffered_mutations.insert(BufferedMutate {
        change_tick,
        message_tick,
        message: message.slice(cursor.position() as usize..),
    });

    Ok(mutate_index)
}

/// Applies mutations from [`BufferedMutations`].
///
/// If the mutate message can't be applied yet (because the change message with the
/// corresponding tick hasn't arrived), it will be kept in the buffer.
fn apply_mutate_messages(
    world: &mut World,
    params: &mut ReceiveParams,
    buffered_mutations: &mut BufferedMutations,
    change_tick: ServerChangeTick,
) -> bincode::Result<()> {
    let mut result = Ok(());
    buffered_mutations.0.retain(|mutate| {
        if mutate.change_tick > *change_tick {
            return true;
        }

        trace!("applying mutate message for {:?}", mutate.message_tick);
        let len = apply_dyn_array(&mut Cursor::new(&*mutate.message), |cursor| {
            apply_mutations(world, params, cursor, mutate.message_tick)
        });

        match len {
            Ok(len) => {
                if let Some(stats) = &mut params.stats {
                    stats.entities_changed += len as u32;
                }
            }
            Err(e) => result = Err(e),
        }

        false
    });

    result
}

/// Deserializes and applies server mapping from client's pre-spawned entities.
fn apply_entity_mapping(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
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

    Ok(())
}

/// Deserializes and applies entity despawn from change message.
fn apply_despawn(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
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
        (params.registry.despawn)(&ctx, client_entity);
    }

    Ok(())
}

/// Deserializes and applies component removals for an entity.
fn apply_removals(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let server_entity = deserialize_entity(cursor)?;

    let client_entity = params
        .entity_map
        .get_by_server_or_insert(server_entity, || world.spawn(Replicated).id());

    let (mut client_entity, mut commands) = read_entity(world, params.queue, client_entity);
    params
        .entity_markers
        .read(params.command_markers, &*client_entity);

    if let Some(mut history) = client_entity.get_mut::<ConfirmHistory>() {
        history.set_last_tick(message_tick);
    } else {
        commands
            .entity(client_entity.id())
            .insert(ConfirmHistory::new(message_tick));
    }

    let len = apply_sized_array(cursor, |cursor| {
        let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
        let (component_id, component_fns, _) = params.registry.get(fns_id);
        let mut ctx = RemoveCtx {
            commands: &mut commands,
            message_tick,
            component_id,
        };
        component_fns.remove(&mut ctx, params.entity_markers, &mut client_entity);

        Ok(())
    })?;

    if let Some(stats) = &mut params.stats {
        stats.components_changed += len as u32;
    }

    params.queue.apply(world);

    Ok(())
}

/// Deserializes and applies component insertions or mutations for an entity.
fn apply_changes(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let server_entity = deserialize_entity(cursor)?;

    let client_entity = params
        .entity_map
        .get_by_server_or_insert(server_entity, || world.spawn(Replicated).id());

    let (mut client_entity, mut commands) = read_entity(world, params.queue, client_entity);
    params
        .entity_markers
        .read(params.command_markers, &*client_entity);

    if let Some(mut history) = client_entity.get_mut::<ConfirmHistory>() {
        history.set_last_tick(message_tick);
    } else {
        commands
            .entity(client_entity.id())
            .insert(ConfirmHistory::new(message_tick));
    }

    let len = apply_sized_array(cursor, |cursor| {
        let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
        let (component_id, component_fns, rule_fns) = params.registry.get(fns_id);
        let mut ctx = WriteCtx::new(&mut commands, params.entity_map, component_id, message_tick);

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

        Ok(())
    })?;

    if let Some(stats) = &mut params.stats {
        stats.components_changed += len as u32;
    }

    params.queue.apply(world);

    Ok(())
}

/// Reads a received array with its size and applies `f` to all members.
fn apply_sized_array(
    cursor: &mut Cursor<&[u8]>,
    mut f: impl FnMut(&mut Cursor<&[u8]>) -> bincode::Result<()>,
) -> bincode::Result<usize> {
    let len = cursor.read_varint()?;
    for _ in 0..len {
        (f)(cursor)?;
    }

    Ok(len)
}

/// Like [`apply_sized_array`], but instead of relying on the size, applies `f`
/// until the cursor reaches the end.
fn apply_dyn_array(
    cursor: &mut Cursor<&[u8]>,
    mut f: impl FnMut(&mut Cursor<&[u8]>) -> bincode::Result<()>,
) -> bincode::Result<usize> {
    let mut len = 0;
    let end = cursor.get_ref().len() as u64;
    while cursor.position() < end {
        (f)(cursor)?;
        len += 1;
    }

    Ok(len)
}

/// Deserializes and applies component mutations for all entities.
///
/// Consumes all remaining bytes in the cursor.
fn apply_mutations(
    world: &mut World,
    params: &mut ReceiveParams,
    cursor: &mut Cursor<&[u8]>,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let server_entity = deserialize_entity(cursor)?;
    let data_size: usize = cursor.read_varint()?;

    let Some(client_entity) = params.entity_map.get_by_server(server_entity) else {
        // Mutation could arrive after a despawn from change message.
        debug!("ignoring mutations received for unknown server's {server_entity:?}");
        cursor.set_position(cursor.position() + data_size as u64);
        return Ok(());
    };

    let (mut client_entity, mut commands) = read_entity(world, params.queue, client_entity);
    params
        .entity_markers
        .read(params.command_markers, &*client_entity);

    let mut history = client_entity
        .get_mut::<ConfirmHistory>()
        .expect("all entities from mutate message should have confirmed ticks");
    let new_entity = message_tick > history.last_tick();
    if new_entity {
        history.set_last_tick(message_tick);
    } else {
        if !params.entity_markers.need_history() {
            trace!(
                "ignoring outdated mutations for client's {:?}",
                client_entity.id()
            );
            cursor.set_position(cursor.position() + data_size as u64);
            return Ok(());
        }

        let ago = history.last_tick().get().wrapping_sub(message_tick.get());
        if ago >= u64::BITS {
            trace!(
                "discarding {ago} ticks old mutations for client's {:?}",
                client_entity.id()
            );
            cursor.set_position(cursor.position() + data_size as u64);
            return Ok(());
        }

        history.set(ago);
    }

    let end_pos = cursor.position() + data_size as u64;
    let mut components_count = 0;
    while cursor.position() < end_pos {
        let fns_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
        let (component_id, component_fns, rule_fns) = params.registry.get(fns_id);
        let mut ctx = WriteCtx::new(&mut commands, params.entity_map, component_id, message_tick);

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
        stats.components_changed += components_count;
    }

    params.queue.apply(world);

    Ok(())
}

/// Splits world access into entity that disallows structural ECS changes and commands.
fn read_entity<'w, 's>(
    world: &'w mut World,
    queue: &'s mut CommandQueue,
    client_entity: Entity,
) -> (DeferredEntity<'w>, Commands<'w, 's>) {
    let world_cell = world.as_unsafe_world_cell();
    // SAFETY: have write access and the cell used only to get entities.
    let client_entity = unsafe { DeferredEntity::new(world_cell, client_entity) };
    let commands = Commands::new_from_entities(queue, world_cell.entities());

    (client_entity, commands)
}

/// Deserializes `entity` from compressed index and generation.
///
/// For details see
/// [`ReplicationBuffer::write_entity`](crate::server::replication_message::replication_buffer::write_entity).
fn deserialize_entity(cursor: &mut Cursor<&[u8]>) -> bincode::Result<Entity> {
    let flagged_index: u64 = cursor.read_varint()?;
    let has_generation = (flagged_index & 1) > 0;
    let generation = if has_generation {
        cursor.read_varint::<u32>()? + 1
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
    registry: &'a ReplicationRegistry,
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
    /// Systems that synchronize hierarchy changes in [`ParentSync`](super::parent_sync::ParentSync).
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PreUpdate`].
    SyncHierarchy,
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

/// Last received tick for change message from server.
///
/// In other words, last [`RepliconTick`] with a removal, insertion, spawn or despawn.
/// When a component mutates, this value is not updated.
#[derive(Clone, Copy, Debug, Default, Deref, Resource)]
pub struct ServerChangeTick(RepliconTick);

/// Cached buffered mutate messages, used by the replicon client to align them with change messages.
///
/// If [`ClientSet::Reset`] is disabled, then this needs to be cleaned up manually with [`Self::clear`].
#[derive(Default, Resource)]
pub struct BufferedMutations(Vec<BufferedMutate>);

impl BufferedMutations {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Inserts a new buffered message, maintaining sorting by their message tick in descending order.
    fn insert(&mut self, mutation: BufferedMutate) {
        let index = self
            .0
            .partition_point(|other_mutation| mutation.message_tick < other_mutation.message_tick);
        self.0.insert(index, mutation);
    }
}

/// Partially-deserialized mutate message that is waiting for its tick to appear in an change message.
///
/// See also [`crate::server::replication_messages`].
pub(super) struct BufferedMutate {
    /// Required tick to wait for.
    change_tick: RepliconTick,

    /// The tick this mutations corresponds to.
    message_tick: RepliconTick,

    /// Mutations data.
    message: Bytes,
}

/// Replication stats during message processing.
///
/// Statistic will be collected only if the resource is present.
/// The resource is not added by default.
///
/// See also [`ClientDiagnosticsPlugin`](diagnostics::ClientDiagnosticsPlugin)
/// for automatic integration with Bevy diagnostics.
#[derive(Default, Resource, Debug)]
pub struct ClientStats {
    /// Incremented per entity that changes.
    pub entities_changed: u32,
    /// Incremented for every component that changes.
    pub components_changed: u32,
    /// Incremented per client mapping added.
    pub mappings: u32,
    /// Incremented per entity despawn.
    pub despawns: u32,
    /// Replication messages received.
    pub messages: u32,
    /// Replication bytes received in message payloads (without internal messaging plugin data).
    pub bytes: u64,
}
