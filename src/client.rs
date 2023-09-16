use bevy::{
    ecs::{component::Tick, system::Command, world::EntityMut},
    prelude::*,
    utils::{Entry, HashMap},
};
use bevy_renet::transport::client_connected;
use bevy_renet::{renet::RenetClient, transport::NetcodeClientPlugin, RenetClientPlugin};
use serde::{Deserialize, Serialize};

use crate::{
    prelude::ReplicationRules,
    replicon_core::{ComponentDiff, Mapper, WorldDiff, REPLICATION_CHANNEL_ID},
    Replication,
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
    fn diff_receiving_system(
        mut commands: Commands,
        mut last_tick: ResMut<LastTick>,
        mut client: ResMut<RenetClient>,
    ) {
        let mut last_message = None;
        while let Some(message) = client.receive_message(REPLICATION_CHANNEL_ID) {
            last_message = Some(message);
        }

        if let Some(last_message) = last_message {
            let world_diff: WorldDiff = bincode::deserialize(&last_message)
                .expect("server should send only world diffs over replication channel");
            *last_tick = world_diff.tick.into();
            commands.apply_world_diff(world_diff);
        }
    }

    fn ack_sending_system(last_tick: Res<LastTick>, mut client: ResMut<RenetClient>) {
        let message = bincode::serialize(&*last_tick)
            .unwrap_or_else(|e| panic!("client ack should be serialized: {e}"));
        client.send_message(REPLICATION_CHANNEL_ID, message);
    }

    fn reset_system(mut last_tick: ResMut<LastTick>, mut entity_map: ResMut<NetworkEntityMap>) {
        last_tick.0 = 0;
        entity_map.clear();
    }
}

/// Last received tick from server.
///
/// Exists only on clients, sent to the server.
#[derive(Default, Deserialize, Resource, Serialize)]
pub(super) struct LastTick(u32);

impl From<Tick> for LastTick {
    fn from(value: Tick) -> Self {
        Self(value.get())
    }
}

impl From<LastTick> for Tick {
    fn from(value: LastTick) -> Self {
        Self::new(value.0)
    }
}

trait ApplyWorldDiffExt {
    fn apply_world_diff(&mut self, world_diff: WorldDiff);
}

impl ApplyWorldDiffExt for Commands<'_, '_> {
    fn apply_world_diff(&mut self, world_diff: WorldDiff) {
        self.add(ApplyWorldDiff(world_diff));
    }
}

struct ApplyWorldDiff(WorldDiff);

impl Command for ApplyWorldDiff {
    fn apply(self, world: &mut World) {
        world.resource_scope(|world, mut entity_map: Mut<NetworkEntityMap>| {
            world.resource_scope(|world, replication_rules: Mut<ReplicationRules>| {
                for (entity, components) in self.0.entities {
                    let mut entity = entity_map.get_by_server_or_spawn(world, entity);
                    for component_diff in components {
                        match component_diff {
                            ComponentDiff::Changed((replication_id, component)) => {
                                let replication_info = replication_rules.get_info(replication_id);
                                (replication_info.deserialize)(
                                    &mut entity,
                                    &mut entity_map,
                                    &component,
                                );
                            }
                            ComponentDiff::Removed(replication_id) => {
                                let replication_info = replication_rules.get_info(replication_id);
                                (replication_info.remove)(&mut entity);
                            }
                        }
                    }
                }
            });

            for server_entity in self.0.despawns {
                // The entity might have already been deleted with the last diff,
                // but the server might not yet have received confirmation from the
                // client and could include the deletion in the latest diff.
                if let Some(client_entity) = entity_map.remove_by_server(server_entity) {
                    world.entity_mut(client_entity).despawn_recursive();
                }
            }
        });
    }
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
///
/// Used only on client.
#[derive(Default, Resource)]
pub struct NetworkEntityMap {
    server_to_client: HashMap<Entity, Entity>,
    client_to_server: HashMap<Entity, Entity>,
}

impl NetworkEntityMap {
    pub fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        self.server_to_client.insert(server_entity, client_entity);
        self.client_to_server.insert(client_entity, server_entity);
    }

    fn get_by_server_or_spawn<'a>(
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

    fn remove_by_server(&mut self, server_entity: Entity) -> Option<Entity> {
        let client_entity = self.server_to_client.remove(&server_entity);
        if let Some(client_entity) = client_entity {
            self.client_to_server.remove(&client_entity);
        }
        client_entity
    }

    pub fn to_client(&self) -> &HashMap<Entity, Entity> {
        &self.server_to_client
    }

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
