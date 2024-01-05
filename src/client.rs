pub mod client_mapper;
pub mod diagnostics;

use std::io::Cursor;

use bevy::{prelude::*, utils::EntityHashMap};
use bevy_renet::{client_connected, renet::Bytes};
use bevy_renet::{renet::RenetClient, transport::NetcodeClientPlugin, RenetClientPlugin};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintReader;

use crate::replicon_core::{
    replication_rules::{Replication, ReplicationRules},
    replicon_tick::RepliconTick,
    ReplicationChannel,
};
use client_mapper::ServerEntityMap;
use diagnostics::ClientStats;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RenetClientPlugin, NetcodeClientPlugin))
            .init_resource::<ServerEntityMap>()
            .init_resource::<ServerEntityTicks>()
            .init_resource::<BufferedUpdates>()
            .configure_sets(
                PreUpdate,
                ClientSet::Receive.after(NetcodeClientPlugin::update_system),
            )
            .configure_sets(
                PreUpdate,
                ClientSet::Reset
                    .after(NetcodeClientPlugin::update_system)
                    .run_if(bevy_renet::client_just_disconnected()),
            )
            .configure_sets(
                PostUpdate,
                ClientSet::Send.before(NetcodeClientPlugin::send_packets),
            )
            .add_systems(
                PreUpdate,
                Self::replication_receiving_system
                    .map(Result::unwrap)
                    .in_set(ClientSet::Receive)
                    .run_if(client_connected()),
            )
            .add_systems(PreUpdate, Self::reset_system.in_set(ClientSet::Reset));
    }
}

impl ClientPlugin {
    /// Receives and applies replication messages from the server.
    ///
    /// Tick init messages are sent over the [`ReplicationChannel::Reliable`] and are applied first to ensure valid state
    /// for entity updates.
    ///
    /// Entity update messages are sent over [`ReplicationChannel::Unreliable`], which means they may appear
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
    pub(super) fn replication_receiving_system(world: &mut World) -> bincode::Result<()> {
        world.resource_scope(|world, mut client: Mut<RenetClient>| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, mut entity_ticks: Mut<ServerEntityTicks>| {
                    world.resource_scope(|world, mut buffered_updates: Mut<BufferedUpdates>| {
                        world.resource_scope(|world, replication_rules: Mut<ReplicationRules>| {
                            let mut stats = world.remove_resource::<ClientStats>();
                            apply_replication(
                                world,
                                &mut client,
                                &mut entity_map,
                                &mut entity_ticks,
                                &mut buffered_updates,
                                stats.as_mut(),
                                &replication_rules,
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

    fn reset_system(
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
    client: &mut RenetClient,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    buffered_updates: &mut BufferedUpdates,
    mut stats: Option<&mut ClientStats>,
    replication_rules: &ReplicationRules,
) -> Result<(), Box<bincode::ErrorKind>> {
    while let Some(message) = client.receive_message(ReplicationChannel::Reliable) {
        apply_init_message(
            &message,
            world,
            entity_map,
            entity_ticks,
            stats.as_deref_mut(),
            replication_rules,
        )?;
    }

    let replicon_tick = *world.resource::<RepliconTick>();
    while let Some(message) = client.receive_message(ReplicationChannel::Unreliable) {
        let index = apply_update_message(
            message,
            world,
            entity_map,
            entity_ticks,
            buffered_updates,
            stats.as_deref_mut(),
            replication_rules,
            replicon_tick,
        )?;

        client.send_message(ReplicationChannel::Reliable, bincode::serialize(&index)?)
    }

    let mut result = Ok(());
    buffered_updates.0.retain(|update| {
        if update.last_change_tick > replicon_tick {
            return true;
        }

        trace!("applying buffered update message for {replicon_tick:?}");
        if let Err(e) = apply_update_components(
            &mut Cursor::new(&*update.message),
            world,
            entity_map,
            entity_ticks,
            stats.as_deref_mut(),
            replication_rules,
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
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    replication_rules: &ReplicationRules,
) -> bincode::Result<()> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(message);
    if let Some(stats) = &mut stats {
        stats.packets += 1;
        stats.bytes += end_pos;
    }

    let replicon_tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    trace!("applying init message for {replicon_tick:?}");
    *world.resource_mut::<RepliconTick>() = replicon_tick;
    debug_assert!(cursor.position() < end_pos, "init message can't be empty");

    apply_entity_mappings(&mut cursor, world, entity_map, stats.as_deref_mut())?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_init_components(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats.as_deref_mut(),
        ComponentsKind::Insert,
        replication_rules,
        replicon_tick,
    )?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_despawns(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats.as_deref_mut(),
        replication_rules,
        replicon_tick,
    )?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_init_components(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats,
        ComponentsKind::Removal,
        replication_rules,
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
#[allow(clippy::too_many_arguments)]
fn apply_update_message(
    message: Bytes,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    buffered_updates: &mut BufferedUpdates,
    mut stats: Option<&mut ClientStats>,
    replication_rules: &ReplicationRules,
    replicon_tick: RepliconTick,
) -> bincode::Result<u16> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(&*message);
    if let Some(stats) = &mut stats {
        stats.packets += 1;
        stats.bytes += end_pos;
    }

    let last_change_tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    let message_tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    let update_index = DefaultOptions::new().deserialize_from(&mut cursor)?;
    if last_change_tick > replicon_tick {
        trace!("buffering update message for {replicon_tick:?}");
        buffered_updates.0.push(BufferedUpdate {
            last_change_tick,
            message_tick,
            message: message.slice(cursor.position() as usize..),
        });
        return Ok(update_index);
    }

    trace!("applying update message for {replicon_tick:?}");
    apply_update_components(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats,
        replication_rules,
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
    let mappings_len: usize = DefaultOptions::new().deserialize_from(&mut *cursor)?;
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
#[allow(clippy::too_many_arguments)]
fn apply_init_components(
    cursor: &mut Cursor<&[u8]>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    components_kind: ComponentsKind,
    replication_rules: &ReplicationRules,
    replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_len: usize = DefaultOptions::new().deserialize_from(&mut *cursor)?;
    for _ in 0..entities_len {
        let entity = deserialize_entity(cursor)?;
        let data_size: usize = DefaultOptions::new().deserialize_from(&mut *cursor)?;
        let mut entity = entity_map.get_by_server_or_spawn(world, entity);
        entity_ticks.insert(entity.id(), replicon_tick);

        let end_pos = cursor.position() + data_size as u64;
        let mut components_len = 0u32;
        while cursor.position() < end_pos {
            let replication_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            // SAFETY: server and client have identical `ReplicationRules` and server always sends valid IDs.
            let replication_info = unsafe { replication_rules.get_info_unchecked(replication_id) };
            match components_kind {
                ComponentsKind::Insert => {
                    (replication_info.deserialize)(&mut entity, entity_map, cursor, replicon_tick)?
                }
                ComponentsKind::Removal => (replication_info.remove)(&mut entity, replicon_tick),
            }
            components_len += 1;
        }
        if let Some(stats) = &mut stats {
            stats.entities_changed += 1;
            stats.components_changed += components_len;
        }
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
    replication_rules: &ReplicationRules,
    replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_len: usize = DefaultOptions::new().deserialize_from(&mut *cursor)?;
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
            (replication_rules.despawn_fn)(client_entity, replicon_tick);
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
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    replication_rules: &ReplicationRules,
    message_tick: RepliconTick,
) -> bincode::Result<()> {
    let message_end = cursor.get_ref().len() as u64;
    while cursor.position() < message_end {
        let entity = deserialize_entity(cursor)?;
        let data_size: usize = DefaultOptions::new().deserialize_from(&mut *cursor)?;
        let Some(mut entity) = entity_map.get_by_server(world, entity) else {
            // Update could arrive after a despawn from init message.
            debug!("ignoring update received for unknown server's {entity:?}");
            cursor.set_position(cursor.position() + data_size as u64);
            continue;
        };
        let entity_tick = entity_ticks
            .get_mut(&entity.id())
            .expect("all entities from update should have assigned ticks");
        if message_tick <= *entity_tick {
            trace!("ignoring outdated update for client's {:?}", entity.id());
            cursor.set_position(cursor.position() + data_size as u64);
            continue;
        }
        *entity_tick = message_tick;

        let end_pos = cursor.position() + data_size as u64;
        let mut components_count = 0u32;
        while cursor.position() < end_pos {
            let replication_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            // SAFETY: server and client have identical `ReplicationRules` and server always sends valid IDs.
            let replication_info = unsafe { replication_rules.get_info_unchecked(replication_id) };
            (replication_info.deserialize)(&mut entity, entity_map, cursor, message_tick)?;
            components_count += 1;
        }
        if let Some(stats) = &mut stats {
            stats.entities_changed += 1;
            stats.components_changed += components_count;
        }
    }

    Ok(())
}

/// Deserializes `entity` from compressed index and generation.
///
/// For details see [`ReplicationBuffer::write_entity`](crate::server::replication_message::replication_buffer::write_entity).
fn deserialize_entity(cursor: &mut Cursor<&[u8]>) -> bincode::Result<Entity> {
    let flagged_index: u64 = cursor.read_u64_varint()?;
    let has_generation = (flagged_index & 1) > 0;
    let generation = if has_generation {
        cursor.read_u32_varint()?
    } else {
        0u32
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
    /// Systems that receive data.
    ///
    /// Runs in `PreUpdate`.
    Receive,
    /// Systems that send data.
    ///
    /// Runs in `PostUpdate`.
    Send,
    /// Systems that reset the client.
    ///
    /// Runs in `PreUpdate`.
    ///
    /// If this set is disabled, then you need to manually clean up the client after a disconnect or when
    /// reconnecting.
    Reset,
}

/// Last received tick for each entity.
///
/// Used to avoid applying old updates.
///
/// If [`ClientSet::Reset`] is disabled, then this needs to be cleaned up manually.
#[derive(Default, Deref, DerefMut, Resource)]
pub struct ServerEntityTicks(EntityHashMap<Entity, RepliconTick>);

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
    ///
    /// See also [`crate::server::LastChangeTick`].
    last_change_tick: RepliconTick,

    /// The tick this update corresponds to.
    message_tick: RepliconTick,

    /// Update data.
    message: Bytes,
}
