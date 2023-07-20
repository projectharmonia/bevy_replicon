use bevy::{
    ecs::{component::Tick, entity::EntityMap, reflect::ReflectMapEntities, system::Command},
    prelude::*,
    reflect::TypeRegistryInternal,
    utils::HashMap,
};
use bevy_renet::transport::client_connected;
use bevy_renet::{renet::RenetClient, transport::NetcodeClientPlugin, RenetClientPlugin};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeSeed, Deserialize, Serialize};

use crate::{
    world_diff::{ComponentDiff, WorldDiff, WorldDiffDeserializer},
    Replication, REPLICATION_CHANNEL_ID,
};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RenetClientPlugin, NetcodeClientPlugin))
            .add_systems(
                PreUpdate,
                Self::init_system.run_if(resource_added::<RenetClient>()),
            )
            .add_systems(
                Update,
                (
                    Self::world_diff_receiving_system,
                    Self::tick_ack_sending_system,
                )
                    .chain()
                    .run_if(client_connected()),
            );
    }
}

impl ClientPlugin {
    fn init_system(mut commands: Commands) {
        commands.insert_resource(LastTick::default());
        commands.insert_resource(NetworkEntityMap::default());
    }

    fn tick_ack_sending_system(last_tick: Res<LastTick>, mut client: ResMut<RenetClient>) {
        let message = bincode::serialize(&*last_tick)
            .unwrap_or_else(|e| panic!("client ack should be serialized: {e}"));
        client.send_message(REPLICATION_CHANNEL_ID, message);
    }

    fn world_diff_receiving_system(
        mut commands: Commands,
        mut last_tick: ResMut<LastTick>,
        mut client: ResMut<RenetClient>,
        registry: Res<AppTypeRegistry>,
    ) {
        let mut last_message = None;
        while let Some(message) = client.receive_message(REPLICATION_CHANNEL_ID) {
            last_message = Some(message);
        }

        if let Some(last_message) = last_message {
            let registry = registry.read();
            // Set options to match `bincode::serialize`.
            // https://docs.rs/bincode/latest/bincode/config/index.html#options-struct-vs-bincode-functions
            let options = DefaultOptions::new()
                .with_fixint_encoding()
                .allow_trailing_bytes();
            let mut deserializer = bincode::Deserializer::from_slice(&last_message, options);
            let world_diff = WorldDiffDeserializer::new(&registry)
                .deserialize(&mut deserializer)
                .expect("server should send only world diffs over replication channel");
            *last_tick = world_diff.tick.into();
            commands.apply_world_diff(world_diff);
        }
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
        let registry = world.resource::<AppTypeRegistry>().clone();
        let registry = registry.read();
        world.resource_scope(|world, mut entity_map: Mut<NetworkEntityMap>| {
            // Map entities non-lazily in order to correctly map components that reference server entities.
            for (entity, components) in map_entities(world, &mut entity_map, self.0.entities) {
                for component_diff in components {
                    apply_component_diff(
                        world,
                        &mut entity_map,
                        &registry,
                        entity,
                        &component_diff,
                    );
                }
            }

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
    entity_map: &mut NetworkEntityMap,
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
                // TODO 0.12: Remove mutable access.
                reflect_map_entities.map_entities(
                    world,
                    &mut entity_map.server_to_client,
                    &[client_entity],
                );
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

    fn get_by_server_or_spawn(&mut self, world: &mut World, server_entity: Entity) -> Entity {
        *self
            .server_to_client
            .entry(server_entity)
            .or_insert_with(|| {
                let client_entity = world.spawn(Replication).id();
                self.client_to_server.insert(client_entity, server_entity);
                client_entity
            })
    }

    fn remove_by_server(&mut self, server_entity: Entity) -> Option<Entity> {
        let client_entity = self.server_to_client.remove(server_entity);
        if let Some(client_entity) = client_entity {
            self.client_to_server.remove(client_entity);
        }
        client_entity
    }

    pub(crate) fn to_client(&self) -> &EntityMap {
        &self.server_to_client
    }

    pub(crate) fn to_server(&self) -> &EntityMap {
        &self.client_to_server
    }
}
