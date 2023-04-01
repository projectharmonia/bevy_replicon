use bevy::{
    ecs::{
        entity::{EntityMap, MapEntitiesError},
        reflect::ReflectMapEntities,
        system::{Command, SystemChangeTick},
    },
    prelude::*,
    reflect::TypeRegistryInternal,
    utils::HashMap,
};
use bevy_renet::{renet::RenetClient, RenetClientPlugin};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeSeed, Deserialize, Serialize};

use crate::{
    tick::Tick,
    world_diff::{ComponentDiff, WorldDiff, WorldDiffDeserializer},
    Replication, REPLICATION_CHANNEL_ID,
};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(RenetClientPlugin::default())
            .add_state::<ClientState>()
            .init_resource::<LastTick>()
            .init_resource::<NetworkEntityMap>()
            .add_systems((
                Self::no_connection_state_system.run_if(resource_removed::<RenetClient>()),
                Self::connecting_state_system
                    .run_if(bevy_renet::client_connecting)
                    .in_set(OnUpdate(ClientState::NoConnection)),
                Self::connected_state_system
                    .run_if(bevy_renet::client_connected)
                    .in_set(OnUpdate(ClientState::Connecting)),
                Self::client_reset_system.in_schedule(OnExit(ClientState::Connected)),
            ))
            .add_systems(
                (
                    Self::tick_ack_sending_system,
                    Self::world_diff_receiving_system,
                )
                    .in_set(OnUpdate(ClientState::Connected)),
            );
    }
}

impl ClientPlugin {
    fn no_connection_state_system(mut client_state: ResMut<NextState<ClientState>>) {
        client_state.set(ClientState::NoConnection);
    }

    fn connecting_state_system(mut client_state: ResMut<NextState<ClientState>>) {
        client_state.set(ClientState::Connecting);
    }

    fn connected_state_system(mut client_state: ResMut<NextState<ClientState>>) {
        client_state.set(ClientState::Connected);
    }

    fn tick_ack_sending_system(last_tick: Res<LastTick>, mut client: ResMut<RenetClient>) {
        let message = bincode::serialize(&*last_tick)
            .unwrap_or_else(|e| panic!("client ack should be serialized: {e}"));
        client.send_message(REPLICATION_CHANNEL_ID, message);
    }

    fn world_diff_receiving_system(
        mut commands: Commands,
        change_tick: SystemChangeTick,
        mut last_tick: ResMut<LastTick>,
        mut client: ResMut<RenetClient>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        let mut received_diffs = Vec::<WorldDiff>::new();
        while let Some(message) = client.receive_message(REPLICATION_CHANNEL_ID) {
            // Set options to match `bincode::serialize`.
            // https://docs.rs/bincode/latest/bincode/config/index.html#options-struct-vs-bincode-functions
            let options = DefaultOptions::new()
                .with_fixint_encoding()
                .allow_trailing_bytes();
            let mut deserializer = bincode::Deserializer::from_slice(&message, options);
            let world_diff = WorldDiffDeserializer::new(&registry)
                .deserialize(&mut deserializer)
                .expect("server should send only world diffs over replication channel");
            received_diffs.push(world_diff);
        }

        if let Some(world_diff) = received_diffs
            .into_iter()
            .max_by_key(|world_diff| world_diff.tick.get())
            .filter(|world_diff| {
                world_diff
                    .tick
                    .is_newer_than(last_tick.0, Tick::new(change_tick.change_tick()))
            })
        {
            last_tick.0 = world_diff.tick;
            commands.apply_world_diff(world_diff);
        }
    }

    fn client_reset_system(mut commands: Commands) {
        commands.insert_resource(LastTick::default());
        commands.insert_resource(NetworkEntityMap::default());
    }
}

#[derive(States, Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
pub enum ClientState {
    #[default]
    NoConnection,
    Connecting,
    Connected,
}

/// Last received tick from server.
///
/// Exists only on clients, sent to the server.
#[derive(Resource, Serialize, Deserialize, Deref, DerefMut)]
pub(super) struct LastTick(pub(super) Tick);

impl Default for LastTick {
    fn default() -> Self {
        Self(Tick::new(0))
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
    fn write(self, world: &mut World) {
        let registry = world.resource::<AppTypeRegistry>().clone();
        let registry = registry.read();
        world.resource_scope(|world, mut entity_map: Mut<NetworkEntityMap>| {
            // Map entities non-lazily in order to correctly map components that reference server entities.
            for (entity, components) in map_entities(world, &mut entity_map, self.0.entities) {
                for component_diff in components {
                    apply_component_diff(world, &entity_map, &registry, entity, &component_diff);
                }
            }

            for server_entity in self.0.despawns {
                let client_entity = entity_map
                    .remove_by_server(server_entity)
                    .expect("server should send valid entities to despawn");
                world.entity_mut(client_entity).despawn_recursive();
            }
        });
    }
}

/// Maps entities received from server into client entities.
fn map_entities(
    world: &mut World,
    entity_map: &mut NetworkEntityMap,
    entities: HashMap<Entity, Vec<ComponentDiff>>,
) -> Vec<(Entity, Vec<ComponentDiff>)> {
    let mut mapped_entities = Vec::with_capacity(entities.len());
    for (server_entity, components) in entities {
        let client_entity = entity_map.get_by_server_or_spawn(world, server_entity);
        mapped_entities.push((client_entity, components));
    }
    mapped_entities
}

fn apply_component_diff(
    world: &mut World,
    entity_map: &NetworkEntityMap,
    registry: &TypeRegistryInternal,
    client_entity: Entity,
    component_diff: &ComponentDiff,
) {
    let type_name = component_diff.type_name();
    let registration = registry
        .get_with_name(type_name)
        .unwrap_or_else(|| panic!("{type_name} should be registered"));

    let reflect_component = registration
        .data::<ReflectComponent>()
        .unwrap_or_else(|| panic!("{type_name} should have reflect(Component)"));

    match component_diff {
        ComponentDiff::Changed(component) => {
            reflect_component.apply_or_insert(&mut world.entity_mut(client_entity), &**component);
            if let Some(reflect_map_entities) = registration.data::<ReflectMapEntities>() {
                reflect_map_entities
                    .map_specific_entities(world, entity_map.to_client(), &[client_entity])
                    .unwrap_or_else(|e| panic!("entities in {type_name} should be mappable: {e}"));
            }
        }
        ComponentDiff::Removed(_) => reflect_component.remove(&mut world.entity_mut(client_entity)),
    }
}

/// Maps server entities to client entities and vice versa.
///
/// Used only on client.
#[derive(Default, Resource)]
pub(crate) struct NetworkEntityMap {
    server_to_client: EntityMap,
    client_to_server: EntityMap,
}

impl NetworkEntityMap {
    #[cfg(test)]
    pub(crate) fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        self.server_to_client.insert(server_entity, client_entity);
        self.client_to_server.insert(client_entity, server_entity);
    }

    pub(crate) fn get_by_server_or_spawn(
        &mut self,
        world: &mut World,
        server_entity: Entity,
    ) -> Entity {
        *self
            .server_to_client
            .entry(server_entity)
            .or_insert_with(|| {
                let client_entity = world.spawn(Replication).id();
                self.client_to_server.insert(client_entity, server_entity);
                client_entity
            })
    }

    pub(crate) fn remove_by_server(
        &mut self,
        server_entity: Entity,
    ) -> Result<Entity, MapEntitiesError> {
        let client_entity = self.server_to_client.remove(server_entity);
        if let Some(client_entity) = client_entity {
            self.client_to_server.remove(client_entity);
        }
        client_entity.ok_or(MapEntitiesError::EntityNotFound(server_entity))
    }

    pub(crate) fn to_client(&self) -> &EntityMap {
        &self.server_to_client
    }

    pub(crate) fn to_server(&self) -> &EntityMap {
        &self.client_to_server
    }
}
