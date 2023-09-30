use std::io::Cursor;

use bevy::{
    ecs::world::EntityMut,
    prelude::*,
    utils::{Entry, HashMap},
};
use bevy_renet::{renet::Bytes, transport::client_connected};
use bevy_renet::{renet::RenetClient, transport::NetcodeClientPlugin, RenetClientPlugin};
use bincode::{DefaultOptions, Options};

use crate::replicon_core::{
    replication_rules::{Mapper, Replication, ReplicationRules},
    NetworkTick, REPLICATION_CHANNEL_ID,
};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RenetClientPlugin, NetcodeClientPlugin))
            .init_resource::<LastTick>()
            .init_resource::<NetworkEntityMap>()
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
                Self::diff_receiving_system
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
    fn diff_receiving_system(world: &mut World) -> Result<(), bincode::Error> {
        world.resource_scope(|world, mut client: Mut<RenetClient>| {
            world.resource_scope(|world, mut entity_map: Mut<NetworkEntityMap>| {
                world.resource_scope(|world, replication_rules: Mut<ReplicationRules>| {
                    while let Some(message) = client.receive_message(REPLICATION_CHANNEL_ID) {
                        let end_pos = message.len().try_into().unwrap();
                        let mut cursor = Cursor::new(message);

                        if !deserialize_tick(&mut cursor, world)? {
                            continue;
                        }
                        if cursor.position() == end_pos {
                            continue;
                        }

                        deserialize_component_diffs(
                            &mut cursor,
                            world,
                            &mut entity_map,
                            &replication_rules,
                            DiffKind::Change,
                        )?;
                        if cursor.position() == end_pos {
                            continue;
                        }

                        deserialize_component_diffs(
                            &mut cursor,
                            world,
                            &mut entity_map,
                            &replication_rules,
                            DiffKind::Removal,
                        )?;
                        if cursor.position() == end_pos {
                            continue;
                        }

                        deserialize_despawns(&mut cursor, world, &mut entity_map)?;
                    }

                    Ok(())
                })
            })
        })
    }

    fn ack_sending_system(last_tick: Res<LastTick>, mut client: ResMut<RenetClient>) {
        let message = bincode::serialize(&last_tick.0)
            .unwrap_or_else(|e| panic!("client ack should be serialized: {e}"));
        client.send_message(REPLICATION_CHANNEL_ID, message);
    }

    fn reset_system(mut last_tick: ResMut<LastTick>, mut entity_map: ResMut<NetworkEntityMap>) {
        last_tick.0 = Default::default();
        entity_map.clear();
    }
}

/// Deserializes server tick and applies it to [`LastTick`] if it is newer.
///
/// Returns true if [`LastTick`] has been updated.
fn deserialize_tick(cursor: &mut Cursor<Bytes>, world: &mut World) -> Result<bool, bincode::Error> {
    let tick = bincode::deserialize_from(cursor)?;

    let mut last_tick = world.resource_mut::<LastTick>();
    if last_tick.0 < tick {
        last_tick.0 = tick;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Deserializes component diffs of `diff_kind` and applies them to the `world`.
fn deserialize_component_diffs(
    cursor: &mut Cursor<Bytes>,
    world: &mut World,
    entity_map: &mut NetworkEntityMap,
    replication_rules: &ReplicationRules,
    diff_kind: DiffKind,
) -> Result<(), bincode::Error> {
    let entities_count: u16 = bincode::deserialize_from(&mut *cursor)?;
    for _ in 0..entities_count {
        let entity = deserialize_entity(&mut *cursor)?;
        let mut entity = entity_map.get_by_server_or_spawn(world, entity);
        let components_count: u8 = bincode::deserialize_from(&mut *cursor)?;
        for _ in 0..components_count {
            let replication_id = DefaultOptions::new().deserialize_from(&mut *cursor)?;
            let replication_info = replication_rules.get_info(replication_id);
            match diff_kind {
                DiffKind::Change => {
                    (replication_info.deserialize)(&mut entity, entity_map, cursor)?
                }
                DiffKind::Removal => (replication_info.remove)(&mut entity),
            }
        }
    }

    Ok(())
}

/// Deserializes despawns and applies them to the `world`.
fn deserialize_despawns(
    cursor: &mut Cursor<Bytes>,
    world: &mut World,
    entity_map: &mut NetworkEntityMap,
) -> Result<(), bincode::Error> {
    let entities_count: u16 = bincode::deserialize_from(&mut *cursor)?;
    for _ in 0..entities_count {
        // The entity might have already been despawned because of hierarchy or
        // with the last diff, but the server might not yet have received confirmation
        // from the client and could include the deletion in the latest diff.
        let server_entity = deserialize_entity(&mut *cursor)?;
        if let Some(client_entity) = entity_map
            .remove_by_server(server_entity)
            .and_then(|entity| world.get_entity_mut(entity))
        {
            client_entity.despawn_recursive();
        }
    }

    Ok(())
}

/// Deserializes `entity` from compressed index and generation, for details see [`ReplicationBuffer::write_entity()`].
fn deserialize_entity(cursor: &mut Cursor<Bytes>) -> Result<Entity, bincode::Error> {
    let flagged_index: u64 = DefaultOptions::new().deserialize_from(&mut *cursor)?;
    let has_generation = (flagged_index & 1) > 0;
    let generation = if has_generation {
        DefaultOptions::new().deserialize_from(&mut *cursor)?
    } else {
        0u32
    };

    let bits = (generation as u64) << 32 | (flagged_index >> 1);

    Ok(Entity::from_bits(bits))
}

/// Type of component change.
///
/// Parameter for [`deserialize_component_diffs`].
enum DiffKind {
    Change,
    Removal,
}

/// Last received tick from server.
///
/// Exists only on clients, sent to the server.
#[derive(Default, Resource, Deref)]
pub struct LastTick(pub(super) NetworkTick);

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
pub struct NetworkEntityMap {
    server_to_client: HashMap<Entity, Entity>,
    client_to_server: HashMap<Entity, Entity>,
}

impl NetworkEntityMap {
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
    pub fn new(world: &'a mut World, entity_map: &'a mut NetworkEntityMap) -> Self {
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
