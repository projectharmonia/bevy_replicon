pub mod diagnostics;

use std::io::Cursor;

use bevy::{
    ecs::world::EntityMut,
    prelude::*,
    utils::{Entry, HashMap},
};
use bevy_renet::{renet::Bytes, transport::client_connected};
use bevy_renet::{renet::RenetClient, transport::NetcodeClientPlugin, RenetClientPlugin};
use bincode::{DefaultOptions, Options};
use varint_rs::VarintReader;

use crate::replicon_core::{
    replication_rules::{Mapper, Replication, ReplicationRules},
    replicon_tick::RepliconTick,
    REPLICATION_CHANNEL_ID,
};
use diagnostics::ClientStats;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RenetClientPlugin, NetcodeClientPlugin))
            .init_resource::<LastRepliconTick>()
            .init_resource::<ServerEntityMap>()
            .configure_set(
                PreUpdate,
                ClientSet::Receive.after(NetcodeClientPlugin::update_system),
            )
            .configure_set(
                PostUpdate,
                ClientSet::Send.before(NetcodeClientPlugin::send_packets),
            )
            .add_systems(
                PreUpdate,
                Self::replication_receiving_system
                    .pipe(unwrap)
                    .in_set(ClientSet::Receive)
                    .run_if(client_connected()),
            )
            .add_systems(
                PostUpdate,
                (
                    Self::ack_sending_system
                        .in_set(ClientSet::Send)
                        .run_if(client_connected()),
                    Self::reset_system.run_if(resource_removed::<RenetClient>()),
                ),
            );
    }
}

impl ClientPlugin {
    pub(super) fn replication_receiving_system(world: &mut World) -> Result<(), bincode::Error> {
        world.resource_scope(|world, mut client: Mut<RenetClient>| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, replication_rules: Mut<ReplicationRules>| {
                    let mut stats = world.remove_resource::<ClientStats>();
                    while let Some(message) = client.receive_message(REPLICATION_CHANNEL_ID) {
                        let end_pos: u64 = message.len().try_into().unwrap();
                        let mut cursor = Cursor::new(message);
                        if let Some(stats) = &mut stats {
                            stats.packets += 1;
                            stats.bytes += end_pos;
                        }

                        let Some(tick) = apply_tick(&mut cursor, world)? else {
                            continue;
                        };
                        if cursor.position() == end_pos {
                            continue;
                        }

                        apply_entity_mappings(&mut cursor, world, &mut entity_map, stats.as_mut())?;
                        if cursor.position() == end_pos {
                            continue;
                        }

                        apply_components(
                            &mut cursor,
                            world,
                            &mut entity_map,
                            stats.as_mut(),
                            ComponentsKind::Change,
                            &replication_rules,
                            tick,
                        )?;
                        if cursor.position() == end_pos {
                            continue;
                        }

                        apply_components(
                            &mut cursor,
                            world,
                            &mut entity_map,
                            stats.as_mut(),
                            ComponentsKind::Removal,
                            &replication_rules,
                            tick,
                        )?;
                        if cursor.position() == end_pos {
                            continue;
                        }

                        apply_despawns(
                            &mut cursor,
                            world,
                            &mut entity_map,
                            &replication_rules,
                            tick,
                            stats.as_mut(),
                        )?;
                    }

                    if let Some(stats) = stats {
                        world.insert_resource(stats);
                    }

                    Ok(())
                })
            })
        })
    }

    fn ack_sending_system(last_tick: Res<LastRepliconTick>, mut client: ResMut<RenetClient>) {
        let message = bincode::serialize(&last_tick.0)
            .unwrap_or_else(|e| panic!("client ack should be serialized: {e}"));
        client.send_message(REPLICATION_CHANNEL_ID, message);
    }

    fn reset_system(
        mut last_tick: ResMut<LastRepliconTick>,
        mut entity_map: ResMut<ServerEntityMap>,
    ) {
        last_tick.0 = Default::default();
        entity_map.clear();
    }
}

/// Deserializes server tick and applies it to [`LastTick`] if it is newer.
///
/// Returns the tick if [`LastTick`] has been updated.
fn apply_tick(
    cursor: &mut Cursor<Bytes>,
    world: &mut World,
) -> Result<Option<RepliconTick>, bincode::Error> {
    let tick = bincode::deserialize_from(cursor)?;

    let mut last_tick = world.resource_mut::<LastRepliconTick>();
    if last_tick.0 < tick {
        last_tick.0 = tick;
        Ok(Some(tick))
    } else {
        Ok(None)
    }
}

/// Applies received server mappings from client's pre-spawned entities.
fn apply_entity_mappings(
    cursor: &mut Cursor<Bytes>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    stats: Option<&mut ClientStats>,
) -> Result<(), bincode::Error> {
    let array_len: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = stats {
        stats.mappings += array_len as u32;
    }
    for _ in 0..array_len {
        let server_entity = read_entity(cursor)?;
        let client_entity = read_entity(cursor)?;

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
fn apply_components(
    cursor: &mut Cursor<Bytes>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    mut stats: Option<&mut ClientStats>,
    components_kind: ComponentsKind,
    replication_rules: &ReplicationRules,
    tick: RepliconTick,
) -> Result<(), bincode::Error> {
    let entities_count: u16 = bincode::deserialize_from(&mut *cursor)?;
    for _ in 0..entities_count {
        let entity = read_entity(cursor)?;
        let mut entity = entity_map.get_by_server_or_spawn(world, entity);
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
                ComponentsKind::Change => {
                    (replication_info.deserialize)(&mut entity, entity_map, cursor, tick)?
                }
                ComponentsKind::Removal => (replication_info.remove)(&mut entity, tick),
            }
        }
    }

    Ok(())
}

/// Deserializes despawns and applies them to the `world`.
fn apply_despawns(
    cursor: &mut Cursor<Bytes>,
    world: &mut World,
    entity_map: &mut ServerEntityMap,
    replication_rules: &ReplicationRules,
    tick: RepliconTick,
    stats: Option<&mut ClientStats>,
) -> Result<(), bincode::Error> {
    let entities_count: u16 = bincode::deserialize_from(&mut *cursor)?;
    if let Some(stats) = stats {
        stats.despawns += entities_count as u32;
    }
    for _ in 0..entities_count {
        // The entity might have already been despawned because of hierarchy or
        // with the last replication message, but the server might not yet have received confirmation
        // from the client and could include the deletion in the this message.
        let server_entity = read_entity(cursor)?;
        if let Some(client_entity) = entity_map
            .remove_by_server(server_entity)
            .and_then(|entity| world.get_entity_mut(entity))
        {
            (replication_rules.despawn_fn)(client_entity, tick);
        }
    }

    Ok(())
}

/// Deserializes `entity` from compressed index and generation.
///
/// For details see [`ReplicationBuffer::write_entity`](crate::server::replication_buffer::ReplicationBuffer::write_entity).
fn read_entity(cursor: &mut Cursor<Bytes>) -> Result<Entity, bincode::Error> {
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
    Change,
    Removal,
}

/// Last received tick from server.
///
/// Used only on clients, sent to the server in last replicon ack message.
#[derive(Default, Resource, Deref)]
pub struct LastRepliconTick(pub(super) RepliconTick);

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
///
/// Used only on client.
#[derive(Default, Resource)]
pub struct ServerEntityMap {
    server_to_client: HashMap<Entity, Entity>,
    client_to_server: HashMap<Entity, Entity>,
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
    ) -> EntityMut<'a> {
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
    pub fn to_client(&self) -> &HashMap<Entity, Entity> {
        &self.server_to_client
    }

    #[inline]
    pub fn to_server(&self) -> &HashMap<Entity, Entity> {
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
    server_to_client: &'a mut HashMap<Entity, Entity>,
    client_to_server: &'a mut HashMap<Entity, Entity>,
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
