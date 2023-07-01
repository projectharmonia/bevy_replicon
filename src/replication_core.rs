use bevy::{
    ecs::{archetype::Archetype, component::ComponentId},
    prelude::*,
    reflect::GetTypeRegistration,
    utils::{HashMap, HashSet},
};
use bevy_renet::renet::{ChannelConfig, SendType};

use crate::REPLICATION_CHANNEL_ID;

pub struct ReplicationCorePlugin;

impl Plugin for ReplicationCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replication>()
            .init_resource::<NetworkChannels>()
            .init_resource::<ReplicationRules>();
    }
}

/// A resource to create channels for [`bevy_renet::renet::ConnectionConfig`]
/// based on number of added server and client events.
#[derive(Clone, Resource, Default)]
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
    /// Also registers the type in [`AppTypeRegistry`].
    /// The component should implement [`Reflect`] and have `#[reflect(Component)]`.
    fn replicate<T: Component + GetTypeRegistration>(&mut self) -> &mut Self;

    /// Ignores component `T` replication if component `U` is present on the same entity.
    ///
    /// Component `T` should be marked for replication.
    /// Could be called multiple times for the same component to disable replication
    /// for different presented components.
    fn not_replicate_if_present<T: Component, U: Component>(&mut self) -> &mut Self;
}

impl AppReplicationExt for App {
    fn replicate<T: Component + GetTypeRegistration>(&mut self) -> &mut Self {
        self.register_type::<T>();
        let component_id = self.world.init_component::<T>();
        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules.replicated.insert(component_id);
        self
    }

    fn not_replicate_if_present<T: Component, U: Component>(&mut self) -> &mut Self {
        let ignore_id = self.world.init_component::<T>();
        let present_id = self.world.init_component::<U>();
        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules
            .ignored_if_present
            .entry(ignore_id)
            .or_default()
            .push(present_id);
        self
    }
}

/// Contains [`ComponentId`]'s that used to decide
/// if a component should be replicated.
#[derive(Resource)]
pub struct ReplicationRules {
    /// Components that should be replicated.
    pub(super) replicated: HashSet<ComponentId>,

    /// Ignore a key component if any of its value components are present in an archetype.
    ignored_if_present: HashMap<ComponentId, Vec<ComponentId>>,

    /// ID of [`Replication`] component, only entities with this components should be replicated.
    replication_id: ComponentId,
}

impl ReplicationRules {
    /// Returns `true` if an entity of an archetype should be replicated.
    pub fn is_replicated_archetype(&self, archetype: &Archetype) -> bool {
        archetype.contains(self.replication_id)
    }

    /// Returns `true` if a component of an archetype should be replicated.
    pub fn is_replicated_component(
        &self,
        archetype: &Archetype,
        component_id: ComponentId,
    ) -> bool {
        if self.replicated.contains(&component_id) {
            if let Some(ignore_ids) = self.ignored_if_present.get(&component_id) {
                for &ignore_id in ignore_ids {
                    if archetype.contains(ignore_id) {
                        return false;
                    }
                }
            }
            return true;
        }

        false
    }
}

impl FromWorld for ReplicationRules {
    fn from_world(world: &mut World) -> Self {
        Self {
            replicated: Default::default(),
            ignored_if_present: Default::default(),
            replication_id: world.init_component::<Replication>(),
        }
    }
}

/// Marks entity for replication.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct Replication;
