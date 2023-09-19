use std::{cmp::Ordering, io::Cursor, marker::PhantomData};

use bevy::{
    ecs::{component::ComponentId, world::EntityMut},
    prelude::*,
    ptr::Ptr,
    utils::HashMap,
};
use bevy_renet::renet::{Bytes, ChannelConfig, SendType};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use strum::EnumDiscriminants;

use crate::client::{ClientMapper, LastTick, NetworkEntityMap};

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
        serialize: SerializeFn,
        deserialize: DeserializeFn,
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

    fn replicate_with<C>(&mut self, serialize: SerializeFn, deserialize: DeserializeFn) -> &mut Self
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

/// Signature of serialization function stored in [`ReplicationInfo`].
pub type SerializeFn = fn(Ptr, &mut Cursor<&mut Vec<u8>>) -> Result<(), bincode::Error>;

/// Signature of deserialization function stored in [`ReplicationInfo`].
pub type DeserializeFn =
    fn(&mut EntityMut, &mut NetworkEntityMap, &mut Cursor<Bytes>) -> Result<(), bincode::Error>;

pub struct ReplicationInfo {
    /// ID of [`Ignored<T>`] component.
    pub ignored_id: ComponentId,

    /// Function that serializes component into bytes.
    pub serialize: SerializeFn,

    /// Function that deserializes component from bytes and inserts it to [`EntityMut`].
    pub deserialize: DeserializeFn,

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
fn serialize_component<C: Component + Serialize>(
    component: Ptr,
    cursor: &mut Cursor<&mut Vec<u8>>,
) -> Result<(), bincode::Error> {
    // SAFETY: Function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    bincode::serialize_into(cursor, component)
}

/// Default deserialization function.
fn deserialize_component<C: Component + DeserializeOwned>(
    entity: &mut EntityMut,
    _entity_map: &mut NetworkEntityMap,
    cursor: &mut Cursor<Bytes>,
) -> Result<(), bincode::Error> {
    let component: C = bincode::deserialize_from(cursor)?;
    entity.insert(component);

    Ok(())
}

/// Default deserialization function that also maps entities before insertion.
fn deserialize_mapped_component<C: Component + DeserializeOwned + MapNetworkEntities>(
    entity: &mut EntityMut,
    entity_map: &mut NetworkEntityMap,
    cursor: &mut Cursor<Bytes>,
) -> Result<(), bincode::Error> {
    let mut component: C = bincode::deserialize_from(cursor)?;

    entity.world_scope(|world| {
        component.map_entities(&mut ClientMapper::new(world, entity_map));
    });

    entity.insert(component);

    Ok(())
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
pub(super) struct WorldDiff<'a> {
    pub(super) tick: NetworkTick,
    pub(super) entities: HashMap<Entity, Vec<ComponentDiff<'a>>>,
    pub(super) despawns: Vec<Entity>,
}

impl WorldDiff<'_> {
    /// Creates a new [`WorldDiff`] with a tick and empty entities.
    pub(super) fn new(tick: NetworkTick) -> Self {
        Self {
            tick,
            entities: Default::default(),
            despawns: Default::default(),
        }
    }

    /// Serializes itself into a buffer.
    ///
    /// We use custom implementation because serde impls require to use generics that can't be stored in [`ReplicationInfo`].
    pub(super) fn serialize(
        &self,
        replication_rules: &ReplicationRules,
        message: &mut Vec<u8>,
    ) -> Result<(), bincode::Error> {
        let mut cursor = Cursor::new(message);

        bincode::serialize_into(&mut cursor, &self.tick)?;

        bincode::serialize_into(&mut cursor, &self.entities.len())?;
        for (entity, components) in &self.entities {
            bincode::serialize_into(&mut cursor, entity)?;
            bincode::serialize_into(&mut cursor, &components.len())?;
            for &component_diff in components {
                bincode::serialize_into(&mut cursor, &ComponentDiffKind::from(component_diff))?;
                match component_diff {
                    ComponentDiff::Changed((replication_id, ptr)) => {
                        bincode::serialize_into(&mut cursor, &replication_id)?;
                        let replication_info = replication_rules.get_info(replication_id);
                        (replication_info.serialize)(ptr, &mut cursor)?;
                    }
                    ComponentDiff::Removed(replication_id) => {
                        bincode::serialize_into(&mut cursor, &replication_id)?;
                    }
                }
            }
        }

        bincode::serialize_into(&mut cursor, &self.despawns)?;

        Ok(())
    }

    /// Deserializes itself from bytes directly into the world by applying all changes.
    ///
    /// Does nothing if world already received a more recent diff.
    /// See also [`LastTick`].
    pub(super) fn deserialize_to_world(
        world: &mut World,
        message: Bytes,
    ) -> Result<(), bincode::Error> {
        let mut cursor = Cursor::new(message);

        let tick = bincode::deserialize_from(&mut cursor)?;
        let mut last_tick = world.resource_mut::<LastTick>();
        if last_tick.0 >= tick {
            return Ok(());
        }
        last_tick.0 = tick;

        world.resource_scope(|world, replication_rules: Mut<ReplicationRules>| {
            world.resource_scope(|world, mut entity_map: Mut<NetworkEntityMap>| {
                let entities_count: usize = bincode::deserialize_from(&mut cursor)?;
                for _ in 0..entities_count {
                    let entity = bincode::deserialize_from(&mut cursor)?;
                    let mut entity = entity_map.get_by_server_or_spawn(world, entity);
                    let components_count: usize = bincode::deserialize_from(&mut cursor)?;
                    for _ in 0..components_count {
                        let diff_kind = bincode::deserialize_from(&mut cursor)?;
                        let replication_id = bincode::deserialize_from(&mut cursor)?;
                        let replication_info = replication_rules.get_info(replication_id);
                        match diff_kind {
                            ComponentDiffKind::Changed => {
                                (replication_info.deserialize)(
                                    &mut entity,
                                    &mut entity_map,
                                    &mut cursor,
                                )?;
                            }
                            ComponentDiffKind::Removed => {
                                (replication_info.remove)(&mut entity);
                            }
                        }
                    }
                }

                let despawns: Vec<Entity> = bincode::deserialize_from(&mut cursor)?;
                for server_entity in despawns {
                    // The entity might have already been deleted with the last diff,
                    // but the server might not yet have received confirmation from the
                    // client and could include the deletion in the latest diff.
                    if let Some(client_entity) = entity_map.remove_by_server(server_entity) {
                        world.entity_mut(client_entity).despawn_recursive();
                    }
                }

                Ok(())
            })
        })
    }
}

/// Type of component change.
#[derive(EnumDiscriminants, Clone, Copy)]
#[strum_discriminants(name(ComponentDiffKind), derive(Deserialize, Serialize))]
pub(super) enum ComponentDiff<'a> {
    /// Indicates that a component was added or changed, contains its ID and pointer.
    Changed((ReplicationId, Ptr<'a>)),
    /// Indicates that a component was removed, contains its ID.
    Removed(ReplicationId),
}

/// Corresponds to the number of server update.
///
/// See also [`crate::server::TickPolicy`].
#[derive(Clone, Copy, Default, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct NetworkTick(u32);

impl NetworkTick {
    /// Creates a new [`NetworkTick`] wrapping the given value.
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    /// Gets the value of this network tick.
    pub fn get(self) -> u32 {
        self.0
    }

    /// Increments current tick and takes wrapping into account.
    pub(super) fn increment(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

impl PartialOrd for NetworkTick {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let difference = self.0.wrapping_sub(other.0);
        if difference == 0 {
            Some(Ordering::Equal)
        } else if difference > u32::MAX / 2 {
            Some(Ordering::Less)
        } else {
            Some(Ordering::Greater)
        }
    }
}
