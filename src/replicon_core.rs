use std::{cmp::Ordering, io::Cursor, io::Write, marker::PhantomData};

use bevy::{
    ecs::{
        component::{ComponentId, Tick},
        world::EntityMut,
    },
    prelude::*,
    ptr::Ptr,
    utils::HashMap,
};
use bevy_renet::renet::{Bytes, ChannelConfig, SendType};
use bincode::Options;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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
    pub(super) fn get_info(&self, replication_id: ReplicationId) -> &ReplicationInfo {
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

/// Buffer for an entity's replicated components.
/// - An entity may replicate up to 127 components.
#[derive(Default)]
struct ComponentData {
    /// Number of updated components.
    /// - Only 7 bits can be used. We reserve one bit to indicate if components were removed.
    updated_count: u8,
    updated_data: Vec<u8>,

    /// Number of removed components.
    removed_count: u8,
    removed_ids: Vec<u8>,
}

/// Buffer for a client's replicated entities.
/// - Up to 65535 entities may be replicated.
//todo: save up to N ComponentData buffers for removed entities and re-use them for newly added entities
#[derive(Default)]
pub(super) struct ReplicationBuffer {
    /// [ entity : component data ]
    data: HashMap<Entity, ComponentData>,

    /// Number of despawned entities
    despawn_count: u16,
    /// Despawned entities
    despawned: Vec<u8>,

    /// Total buffer size
    total_size: usize,

    /// The current server tick
    server_tick: NetworkTick,
    /// Last system tick acked by the client
    acked_system_tick: Option<Tick>,
}

impl ReplicationBuffer {
    /// Update ticks
    pub(super) fn refresh_ticks(&mut self, server_tick: NetworkTick, acked_system_tick: Tick) {
        self.server_tick = server_tick;
        self.acked_system_tick = Some(acked_system_tick);
    }

    /// Get the client's last acked system tick
    pub(super) fn last_acked_system_tick(&self) -> Tick {
        self.acked_system_tick.unwrap_or(Tick::new(0u32))
    }

    /// Add an updated component to the buffer.
    pub(super) fn append_updated_component(
        &mut self,
        replication_rules: &ReplicationRules,
        entity: Entity,
        replication_id: ReplicationId,
        component_diff: Ptr<'_>,
    ) -> Result<(), bincode::Error> {
        // get component data entry
        let entry = self.data.entry(entity).or_default();

        // write component diff into buffer
        let prev_len = entry.updated_data.len();
        {
            let len = entry.updated_data.len();
            let mut cursor = Cursor::new(&mut entry.updated_data);
            cursor.set_position(len as u64);
            bincode::config::DefaultOptions::new().serialize_into(&mut cursor, &replication_id)?;
            let replication_info = replication_rules.get_info(replication_id);
            (replication_info.serialize)(component_diff, &mut cursor)?;
        }
        let post_len = entry.updated_data.len();

        // update trackers
        entry.updated_count += 1;
        if entry.updated_count > 127 {
            error!("entity has too many components being updated");
        }
        self.total_size += post_len.saturating_sub(prev_len);

        Ok(())
    }

    /// Add a removed component to the buffer.
    pub(super) fn append_removed_component(
        &mut self,
        entity: Entity,
        replication_id: ReplicationId,
    ) -> Result<(), bincode::Error> {
        // get component data entry
        let entry = self.data.entry(entity).or_default();

        // write removed component's id into buffer
        let prev_len = entry.removed_ids.len();
        bincode::config::DefaultOptions::new()
            .serialize_into(&mut entry.removed_ids, &replication_id)?;
        let post_len = entry.removed_ids.len();

        // update trackers
        entry.removed_count = entry
            .removed_count
            .checked_add(1)
            .expect("entity has removed too many components");
        self.total_size += post_len.saturating_sub(prev_len);

        Ok(())
    }

    /// Add a despawned entity to the buffer.
    /// - Will clean up any internal entries for the despawned entity.
    pub(super) fn despawn_entity(&mut self, entity: Entity) -> Result<(), bincode::Error> {
        // write despawned entity into buffer
        let prev_len = self.despawned.len();
        bincode::config::DefaultOptions::new().serialize_into(&mut self.despawned, &entity)?;
        let post_len = self.despawned.len();

        // update trackers
        self.despawn_count = self
            .despawn_count
            .checked_add(1)
            .expect("entity has despawned too many entities");
        self.total_size += post_len.saturating_sub(prev_len);

        // clean up entity entry
        let _ = self.data.remove(&entity);

        Ok(())
    }

    /// Build a replication message and reset the buffer's internal state (without deallocating internal buffers).
    pub(super) fn consume(&mut self) -> Result<Vec<u8>, bincode::Error> {
        let mut message = Vec::with_capacity(self.estimate_len());

        // current server tick
        bincode::serialize_into(&mut message, &self.server_tick)?;

        // number of entities
        if self.data.len() > u16::MAX as usize {
            error!("replication buffer has too many entities");
        }
        bincode::serialize_into(&mut message, &(self.data.len() as u16))?;

        // entities
        for (entity, component_data) in self.data.iter_mut() {
            // check if any components were removed from the entity
            // - set most significant bit of updated count to indicate there are removed components
            if component_data.updated_count > 127u8 {
                error!("entity has too many components to replicate");
            }
            if component_data.removed_count > 0 {
                component_data.updated_count += 128u8
            }

            // write entity
            bincode::config::DefaultOptions::new().serialize_into(&mut message, &entity)?;
            bincode::serialize_into(&mut message, &component_data.updated_count)?;
            message
                .write(&component_data.updated_data[..])
                .map_err(|e| Box::new(bincode::ErrorKind::Io(e)))?;

            if component_data.removed_count > 0 {
                bincode::serialize_into(&mut message, &component_data.removed_count)?;
                message
                    .write(&component_data.removed_ids[..])
                    .map_err(|e| Box::new(bincode::ErrorKind::Io(e)))?;
            }

            // reset entity
            component_data.updated_count = 0;
            component_data.updated_data.clear();
            component_data.removed_count = 0;
            component_data.removed_ids.clear();
        }

        // despawned entities
        bincode::serialize_into(&mut message, &self.despawn_count)?;
        message
            .write(&self.despawned[..])
            .map_err(|e| Box::new(bincode::ErrorKind::Io(e)))?;

        // final resets
        self.despawn_count = 0;
        self.despawned.clear();
        self.total_size = 0;

        Ok(message)
    }

    fn estimate_len(&self) -> usize {
        // buffered data + entity ids + entity count + despawn count + network tick
        self.total_size + self.data.len() * 8 + 2 + 2 + 4 + 20
    }
}

/// Deserializes a replication buffer package from bytes directly into the world by applying all changes.
///
/// Does nothing if world already received a more recent diff.
/// See also [`LastTick`].
pub(super) fn deserialize_to_world(
    world: &mut World,
    message: Bytes,
) -> Result<(), bincode::Error> {
    let mut cursor = Cursor::new(message);

    // tick
    let tick = bincode::deserialize_from(&mut cursor)?;
    let mut last_tick = world.resource_mut::<LastTick>();
    if last_tick.0 >= tick {
        return Ok(());
    }
    last_tick.0 = tick;

    // prep
    let replication_rules = world.remove_resource::<ReplicationRules>().unwrap();
    let mut entity_map = world.remove_resource::<NetworkEntityMap>().unwrap();

    // entities
    let entities_count: u16 = bincode::deserialize_from(&mut cursor)?;
    for _ in 0..entities_count {
        // entity
        let entity = bincode::config::DefaultOptions::new().deserialize_from(&mut cursor)?;
        let mut entity = entity_map.get_by_server_or_spawn(world, entity);

        // updated components
        let mut components_count: u8 = bincode::deserialize_from(&mut cursor)?;
        let has_removed = components_count > 127;
        components_count %= 128u8;

        for _ in 0..components_count {
            let replication_id =
                bincode::config::DefaultOptions::new().deserialize_from(&mut cursor)?;
            let replication_info = replication_rules.get_info(replication_id);
            (replication_info.deserialize)(&mut entity, &mut entity_map, &mut cursor)?;
        }

        // removed components
        if !has_removed {
            continue;
        }
        let removed_count: u8 = bincode::deserialize_from(&mut cursor)?;

        for _ in 0..removed_count {
            let replication_id =
                bincode::config::DefaultOptions::new().deserialize_from(&mut cursor)?;
            let replication_info = replication_rules.get_info(replication_id);
            (replication_info.remove)(&mut entity);
        }
    }

    // despawns
    let despawn_count: u16 = bincode::deserialize_from(&mut cursor)?;
    for _ in 0..despawn_count {
        let server_entity: Entity =
            bincode::config::DefaultOptions::new().deserialize_from(&mut cursor)?;
        // The entity might have already been deleted with the last diff,
        // but the server might not yet have received confirmation from the
        // client and could include the deletion in the latest diff.
        if let Some(client_entity) = entity_map.remove_by_server(server_entity) {
            world.entity_mut(client_entity).despawn_recursive();
        }
    }

    // cleanup
    world.insert_resource(replication_rules);
    world.insert_resource(entity_map);

    Ok(())
}
