pub mod diagnostics;

use std::io::Cursor;

use bevy::{
    prelude::*,
    utils::{hashbrown::hash_map::Entry, EntityHashMap},
};
use bevy_renet::{client_connected, renet::Bytes};
use bevy_renet::{renet::RenetClient, transport::NetcodeClientPlugin, RenetClientPlugin};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintReader;

use crate::replicon_core::{
    replication_rules::{Mapper, Replication, ReplicationRules},
    replicon_tick::RepliconTick,
    ReplicationChannel,
};
use diagnostics::ClientStats;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RenetClientPlugin, NetcodeClientPlugin))
            .init_resource::<ServerEntityMap>()
            .init_resource::<ServerEntityTicks>()
            .configure_sets(
                PreUpdate,
                ClientSet::Receive.after(NetcodeClientPlugin::update_system),
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
            .add_systems(
                PostUpdate,
                Self::reset_system.run_if(resource_removed::<RenetClient>()),
            );
    }
}

impl ClientPlugin {
    /// Receives and applies replication messages from the server.
    ///
    /// Init messages applied first to ensure the valid state.
    ///
    /// Then update messages will be applied only if an init message with their tick has arrived.
    /// If an update message received before the required init message, then they will be buffered until then.
    /// Since they could arrive in any order, entities will be updated only if the received update requires a more recent tick.
    ///
    /// And then the buffered messages from the last run are processed.
    ///
    /// Sends acknowledgments back for update messages.
    ///
    /// See also [`ReplicationMessages`](crate::server::replication_messages::ReplicationMessages).
    pub(super) fn replication_receiving_system(
        world: &mut World,
        mut buffered_updates: Local<Vec<BufferedUpdate>>,
        mut retain_buffer: Local<Vec<bool>>,
    ) -> bincode::Result<()> {
        world.resource_scope(|world, mut client: Mut<RenetClient>| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, mut entity_ticks: Mut<ServerEntityTicks>| {
                    world.resource_scope(|world, replication_rules: Mut<ReplicationRules>| {
                        let mut stats = world.remove_resource::<ClientStats>();
                        while let Some(message) =
                            client.receive_message(ReplicationChannel::Reliable)
                        {
                            apply_init_message(
                                &message,
                                world,
                                &mut entity_map,
                                &mut entity_ticks,
                                stats.as_mut(),
                                &replication_rules,
                            )?;
                        }

                        let old_buffers = buffered_updates.len();
                        let replicon_tick = *world.resource::<RepliconTick>();
                        while let Some(message) =
                            client.receive_message(ReplicationChannel::Unreliable)
                        {
                            let index = apply_update_message(
                                message,
                                world,
                                &mut entity_map,
                                &mut entity_ticks,
                                &mut buffered_updates,
                                stats.as_mut(),
                                &replication_rules,
                                replicon_tick,
                            )?;

                            client.send_message(
                                ReplicationChannel::Reliable,
                                bincode::serialize(&index)?,
                            )
                        }

                        retain_buffer.clear();
                        retain_buffer.reserve(old_buffers);
                        for update in buffered_updates.iter().take(old_buffers) {
                            let retain = update.tick < replicon_tick;
                            if retain {
                                let mut cursor = Cursor::new(&*update.message);
                                cursor.set_position(update.position);

                                apply_components(
                                    &mut cursor,
                                    world,
                                    &mut entity_map,
                                    &mut entity_ticks,
                                    stats.as_mut(),
                                    ComponentsKind::Update,
                                    &replication_rules,
                                    replicon_tick,
                                )?;
                            }
                            retain_buffer.push(retain);
                        }
                        let mut iter = retain_buffer.iter();
                        buffered_updates.retain(|_| *iter.next().unwrap_or(&true));

                        if let Some(stats) = stats {
                            world.insert_resource(stats);
                        }

                        Ok(())
                    })
                })
            })
        })
    }

    fn reset_system(
        mut replicon_tick: ResMut<RepliconTick>,
        mut entity_map: ResMut<ServerEntityMap>,
        mut entity_ticks: ResMut<ServerEntityTicks>,
    ) {
        *replicon_tick = Default::default();
        entity_map.clear();
        entity_ticks.0.clear();
    }
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
        stats.init_messages += 1;
        stats.bytes += end_pos;
    }

    let replicon_tick = bincode::deserialize_from(&mut cursor)?;
    trace!("applying {replicon_tick:?}");
    *world.resource_mut::<RepliconTick>() = replicon_tick;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_entity_mappings(&mut cursor, world, entity_map, stats.as_deref_mut())?;
    if cursor.position() == end_pos {
        return Ok(());
    }

    apply_components(
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

    apply_components(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats.as_deref_mut(),
        ComponentsKind::Removal,
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
        replication_rules,
        replicon_tick,
        stats,
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
    buffered_updates: &mut Vec<BufferedUpdate>,
    mut stats: Option<&mut ClientStats>,
    replication_rules: &ReplicationRules,
    replicon_tick: RepliconTick,
) -> bincode::Result<u16> {
    let end_pos: u64 = message.len().try_into().unwrap();
    let mut cursor = Cursor::new(&*message);
    if let Some(stats) = &mut stats {
        stats.update_messages += 1;
        stats.bytes += end_pos;
    }

    let (tick, update_index) = bincode::deserialize_from(&mut cursor)?;
    if tick > replicon_tick {
        buffered_updates.push(BufferedUpdate {
            tick,
            position: cursor.position(),
            message,
        });
        return Ok(update_index);
    }

    apply_components(
        &mut cursor,
        world,
        entity_map,
        entity_ticks,
        stats,
        ComponentsKind::Update,
        replication_rules,
        tick,
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
    let array_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = stats {
        stats.mappings += array_len as u32;
    }
    for _ in 0..array_len {
        let server_entity = deserialize_entity(cursor)?;
        let client_entity = deserialize_entity(cursor)?;

        if let Some(entry) = entity_map.to_client().get(&server_entity) {
            // It's possible to receive the same mappings in multiple packets if the server has not
            // yet received an ack from the client for the tick when the mapping was created.
            if *entry != client_entity {
                panic!("received mapping from {server_entity:?} to {client_entity:?}, but already mapped to {entry:?}");
            }
        }

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
fn apply_components(
    cursor: &mut Cursor<&[u8]>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    entity_ticks: &mut ServerEntityTicks,
    mut stats: Option<&mut ClientStats>,
    components_kind: ComponentsKind,
    replication_rules: &ReplicationRules,
    replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let entities_count: u16 = bincode::deserialize_from(&mut *cursor)?;
    for _ in 0..entities_count {
        let entity = deserialize_entity(cursor)?;
        let mut entity = entity_map.get_by_server_or_spawn(world, entity);
        match components_kind {
            ComponentsKind::Update => {
                let Some(tick) = entity_ticks.0.get_mut(&entity.id()) else {
                    continue; // Update arrived arrive after a despawn from init message.
                };
                if *tick > replicon_tick {
                    continue; // Update for this entity is outdated.
                }
                *tick = replicon_tick;
            }
            ComponentsKind::Insert | ComponentsKind::Removal => {
                entity_ticks.0.insert(entity.id(), replicon_tick);
            }
        }

        let components_count: u8 = bincode::deserialize_from(&mut *cursor)?;
        if let Some(stats) = &mut stats {
            stats.entities_changed += 1;
            stats.components_changed += components_count as u32;
        }
        for _ in 0..components_count {
            let replication_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            // SAFETY: server and client have identical `ReplicationRules` and server always sends valid IDs.
            let replication_info = unsafe { replication_rules.get_info_unchecked(replication_id) };
            match components_kind {
                ComponentsKind::Insert | ComponentsKind::Update => {
                    (replication_info.deserialize)(&mut entity, entity_map, cursor, replicon_tick)?
                }
                ComponentsKind::Removal => (replication_info.remove)(&mut entity, replicon_tick),
            }
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
    replication_rules: &ReplicationRules,
    replicon_tick: RepliconTick,
    stats: Option<&mut ClientStats>,
) -> bincode::Result<()> {
    let entities_count: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = stats {
        stats.despawns += entities_count as u32;
    }
    for _ in 0..entities_count {
        // The entity might have already been despawned because of hierarchy or
        // with the last replication message, but the server might not yet have received confirmation
        // from the client and could include the deletion in the this message.
        let server_entity = deserialize_entity(cursor)?;
        if let Some(client_entity) = entity_map
            .remove_by_server(server_entity)
            .and_then(|entity| world.get_entity_mut(entity))
        {
            entity_ticks.0.remove(&client_entity.id());
            (replication_rules.despawn_fn)(client_entity, replicon_tick);
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
    Update,
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
}

/// Maps server entities to client entities and vice versa.
#[derive(Default, Resource)]
pub struct ServerEntityMap {
    server_to_client: EntityHashMap<Entity, Entity>,
    client_to_server: EntityHashMap<Entity, Entity>,
}

impl ServerEntityMap {
    #[inline]
    pub fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        self.server_to_client.insert(server_entity, client_entity);
        self.client_to_server.insert(client_entity, server_entity);
    }

    pub(super) fn get_by_server_or_spawn<'a>(
        &mut self,
        world: &'a mut World,
        server_entity: Entity,
    ) -> EntityWorldMut<'a> {
        match self.server_to_client.entry(server_entity) {
            Entry::Occupied(entry) => world.entity_mut(*entry.get()),
            Entry::Vacant(entry) => {
                let client_entity = world.spawn(Replication);
                entry.insert(client_entity.id());
                self.client_to_server
                    .insert(client_entity.id(), server_entity);
                client_entity
            }
        }
    }

    pub(super) fn remove_by_server(&mut self, server_entity: Entity) -> Option<Entity> {
        let client_entity = self.server_to_client.remove(&server_entity);
        if let Some(client_entity) = client_entity {
            self.client_to_server.remove(&client_entity);
        }
        client_entity
    }

    #[inline]
    pub fn to_client(&self) -> &EntityHashMap<Entity, Entity> {
        &self.server_to_client
    }

    #[inline]
    pub fn to_server(&self) -> &EntityHashMap<Entity, Entity> {
        &self.client_to_server
    }

    fn clear(&mut self) {
        self.client_to_server.clear();
        self.server_to_client.clear();
    }
}

/// Maps server entities into client entities inside components.
///
/// Spawns new client entity if a mapping doesn't exists.
pub struct ClientMapper<'a> {
    world: &'a mut World,
    server_to_client: &'a mut EntityHashMap<Entity, Entity>,
    client_to_server: &'a mut EntityHashMap<Entity, Entity>,
}

impl<'a> ClientMapper<'a> {
    #[inline]
    pub fn new(world: &'a mut World, entity_map: &'a mut ServerEntityMap) -> Self {
        Self {
            world,
            server_to_client: &mut entity_map.server_to_client,
            client_to_server: &mut entity_map.client_to_server,
        }
    }
}

impl Mapper for ClientMapper<'_> {
    fn map(&mut self, entity: Entity) -> Entity {
        *self.server_to_client.entry(entity).or_insert_with(|| {
            let client_entity = self.world.spawn(Replication).id();
            self.client_to_server.insert(client_entity, entity);
            client_entity
        })
    }
}

/// Last received tick for each entity.
///
/// Used to avoid applying old updates.
#[derive(Default, Deref, Resource)]
pub struct ServerEntityTicks(EntityHashMap<Entity, RepliconTick>);

/// Caches buffer with deserialized tick that we received earlier
/// then the required init message with this tick.
pub(super) struct BufferedUpdate {
    /// Required tick to wait for.
    tick: RepliconTick,
    /// Position of the message data.
    position: u64,
    /// Update data.
    message: Bytes,
}
