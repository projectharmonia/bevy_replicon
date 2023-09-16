use std::{any, marker::PhantomData};

use bevy::{
    ecs::{
        component::{ComponentId, Tick},
        world::EntityMut,
    },
    prelude::*,
    ptr::Ptr,
    utils::HashMap,
};
use bevy_renet::renet::{ChannelConfig, SendType};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};

use crate::client::{ClientMapper, NetworkEntityMap};

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetworkChannels>()
            .init_resource::<ReplicationRules>();
    }
}

pub(super) const REPLICATION_CHANNEL_ID: u8 = 0;

/// A resource to create channels for [`bevy_renet::renet::ConnectionConfig`]
/// based on number of added server and client events.
#[derive(Clone, Default, Resource)]
pub struct NetworkChannels {
    /// Grows with each server event registration.
    server: Vec<SendType>,
    /// Grows with each client event registration.
    client: Vec<SendType>,
}

impl NetworkChannels {
    pub fn server_channels(&self) -> Vec<ChannelConfig> {
        channel_configs(&self.server)
    }

    pub fn client_channels(&self) -> Vec<ChannelConfig> {
        channel_configs(&self.client)
    }

    pub(super) fn create_client_channel(&mut self, send_type: SendType) -> u8 {
        if self.client.len() == REPLICATION_CHANNEL_ID as usize + u8::MAX as usize {
            panic!("max client channels exceeded u8::MAX");
        }
        self.client.push(send_type);
        self.client.len() as u8 + REPLICATION_CHANNEL_ID
    }

    pub(super) fn create_server_channel(&mut self, send_type: SendType) -> u8 {
        if self.server.len() == REPLICATION_CHANNEL_ID as usize + u8::MAX as usize {
            panic!("max server channels exceeded u8::MAX");
        }
        self.server.push(send_type);
        self.server.len() as u8 + REPLICATION_CHANNEL_ID
    }
}

fn channel_configs(channels: &[SendType]) -> Vec<ChannelConfig> {
    let mut channel_configs = Vec::with_capacity(channels.len() + 1);
    // TODO: Make it configurable.
    // Values from `DefaultChannel::config()`.
    channel_configs.push(ChannelConfig {
        channel_id: REPLICATION_CHANNEL_ID,
        max_memory_usage_bytes: 5 * 1024 * 1024,
        send_type: SendType::Unreliable,
    });
    for (idx, send_type) in channels.iter().enumerate() {
        channel_configs.push(ChannelConfig {
            channel_id: REPLICATION_CHANNEL_ID + 1 + idx as u8,
            max_memory_usage_bytes: 5 * 1024 * 1024,
            send_type: send_type.clone(),
        });
    }
    channel_configs
}

pub trait AppReplicationExt {
    /// Marks component for replication.
    ///
    /// Component will be serialized as is using bincode.
    /// It also registers [`Ignored<T>`] that can be used to exclude the component from replication.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned;

    /// Same as [`Self::replicate`], but maps component entities using [`MapNetworkEntities`] trait.
    ///
    /// Always use it for components that contains entities.
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapNetworkEntities;

    /// Same as [`Self::replicate`], but uses the specified functions for serialization and deserialization.
    fn replicate_with<C>(
        &mut self,
        serialize: fn(Ptr) -> Vec<u8>,
        deserialize: fn(&mut EntityMut, &mut NetworkEntityMap, &[u8]),
    ) -> &mut Self
    where
        C: Component;
}

impl AppReplicationExt for App {
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.replicate_with::<C>(serialize_component::<C>, deserialize_component::<C>)
    }

    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapNetworkEntities,
    {
        self.replicate_with::<C>(serialize_component::<C>, deserialize_mapped_component::<C>)
    }

    fn replicate_with<C>(
        &mut self,
        serialize: fn(Ptr) -> Vec<u8>,
        deserialize: fn(&mut EntityMut, &mut NetworkEntityMap, &[u8]),
    ) -> &mut Self
    where
        C: Component,
    {
        let component_id = self.world.init_component::<C>();
        let ignored_id = self.world.init_component::<Ignored<C>>();
        let replicated_component = ReplicationInfo {
            ignored_id,
            serialize,
            deserialize,
            remove: remove_component::<C>,
        };

        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules.info.push(replicated_component);

        let replication_id = ReplicationId(replication_rules.info.len() - 1);
        replication_rules
            .components
            .insert(component_id, replication_id);

        self
    }
}

/// Stores information about which components will be serialized and how.
#[derive(Resource)]
pub struct ReplicationRules {
    /// Maps component IDs to their replication IDs.
    components: HashMap<ComponentId, ReplicationId>,

    /// Meta information about components that should be replicated.
    info: Vec<ReplicationInfo>,

    /// ID of [`Replication`] component.
    replication_id: ComponentId,
}

impl ReplicationRules {
    /// ID of [`Replication`] component, only entities with this components will be replicated.
    pub fn replication_id(&self) -> ComponentId {
        self.replication_id
    }

    /// Returns mapping of replicated components to their replication IDs.
    pub fn components(&self) -> &HashMap<ComponentId, ReplicationId> {
        &self.components
    }

    /// Returns meta information about replicated component.
    pub fn get_info(&self, replication_id: ReplicationId) -> &ReplicationInfo {
        // SAFETY: `ReplicationId` always corresponds to a valid index.
        unsafe { self.info.get_unchecked(replication_id.0) }
    }

    /// Returns ID for component that will be consistent between clients and server.
    pub fn get_id(&self, component_id: ComponentId) -> Option<ReplicationId> {
        self.components.get(&component_id).copied()
    }
}

impl FromWorld for ReplicationRules {
    fn from_world(world: &mut World) -> Self {
        Self {
            info: Default::default(),
            components: Default::default(),
            replication_id: world.init_component::<Replication>(),
        }
    }
}

pub struct ReplicationInfo {
    /// ID of [`Ignored<T>`] component.
    pub ignored_id: ComponentId,

    /// Function that serializes component into bytes.
    pub serialize: fn(Ptr) -> Vec<u8>,

    /// Function that deserializes component from bytes and inserts it to [`EntityMut`].
    pub deserialize: fn(&mut EntityMut, &mut NetworkEntityMap, &[u8]),

    /// Function that removes specific component from [`EntityMut`].
    pub remove: fn(&mut EntityMut),
}

/// Replication will be ignored for `T` if this component is present on the same entity.
#[derive(Component)]
pub struct Ignored<T>(PhantomData<T>);

impl<T> Default for Ignored<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Same as [`ComponentId`], but consistent between server and clients.
///
/// Internally represents index of [`ReplicationInfo`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ReplicationId(usize);

/// Default serialization function.
fn serialize_component<C: Component + Serialize>(component: Ptr) -> Vec<u8> {
    // SAFETY: Function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    bincode::serialize(component)
        .unwrap_or_else(|e| panic!("{} should be serialzable: {e}", any::type_name::<C>()))
}

/// Default deserialization function.
fn deserialize_component<C: Component + DeserializeOwned>(
    entity: &mut EntityMut,
    _entity_map: &mut NetworkEntityMap,
    component: &[u8],
) {
    let component: C = bincode::deserialize(component)
        .unwrap_or_else(|e| panic!("bytes from server should be {}: {e}", any::type_name::<C>()));
    entity.insert(component);
}

/// Default deserialization function that also maps entities before insertion.
fn deserialize_mapped_component<C: Component + DeserializeOwned + MapNetworkEntities>(
    entity: &mut EntityMut,
    entity_map: &mut NetworkEntityMap,
    component: &[u8],
) {
    let mut component: C = bincode::deserialize(component)
        .unwrap_or_else(|e| panic!("bytes from server should be {}: {e}", any::type_name::<C>()));

    entity.world_scope(|world| {
        component.map_entities(&mut ClientMapper::new(world, entity_map));
    });

    entity.insert(component);
}

/// Removes specified component from entity.
fn remove_component<C: Component>(entity: &mut EntityMut) {
    entity.remove::<C>();
}

/// Maps entities inside component.
///
/// The same as [`bevy::ecs::entity::MapEntities`], but never creates new entities on mapping error.
pub trait MapNetworkEntities {
    /// Maps stored entities using specified map.
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T);
}

pub trait Mapper {
    fn map(&mut self, entity: Entity) -> Entity;
}

/// Marks entity for replication.
#[derive(Component, Clone, Copy)]
pub struct Replication;

/// Changed world data and current tick from server.
///
/// Sent from server to clients.
#[derive(Serialize, Deserialize)]
pub(super) struct WorldDiff {
    #[serde(
        serialize_with = "serialize_tick",
        deserialize_with = "deserialize_tick"
    )]
    pub(super) tick: Tick,
    pub(super) entities: HashMap<Entity, Vec<ComponentDiff>>,
    pub(super) despawns: Vec<Entity>,
}

impl WorldDiff {
    /// Creates a new [`WorldDiff`] with a tick and empty entities.
    pub(super) fn new(tick: Tick) -> Self {
        Self {
            tick,
            entities: Default::default(),
            despawns: Default::default(),
        }
    }
}

/// Type of component change.
#[derive(Serialize, Deserialize)]
pub(super) enum ComponentDiff {
    /// Indicates that a component was added or changed, contains its ID and serialized bytes.
    Changed((ReplicationId, Vec<u8>)),
    /// Indicates that a component was removed, contains its ID.
    Removed(ReplicationId),
}

fn serialize_tick<S: Serializer>(tick: &Tick, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u32(tick.get())
}

fn deserialize_tick<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Tick, D::Error> {
    let tick = u32::deserialize(deserializer)?;
    Ok(Tick::new(tick))
}
