pub mod replication_rules;
pub mod replicon_tick;

use bevy::prelude::*;
use bevy_renet::renet::{ChannelConfig, SendType};

use replication_rules::{Replication, ReplicationRules};

pub struct RepliconCorePlugin;

impl Plugin for RepliconCorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replication>()
            .init_resource::<NetworkChannels>()
            .init_resource::<ReplicationRules>();
    }
}

/// ID of the server replication channel.
///
/// See also [`NetworkChannels`].
pub const REPLICATION_CHANNEL_ID: u8 = 0;

/// A resource to configure and setup channels for [`ConnectionConfig`](bevy_renet::renet::ConnectionConfig)
#[derive(Clone, Resource)]
pub struct NetworkChannels {
    /// Stores delivery guarantee and maximum usage bytes (if set) for each server channel.
    server: Vec<(SendType, Option<usize>)>,

    /// Same as [`Self::server`], but for client.
    client: Vec<(SendType, Option<usize>)>,

    /// Stores default max memory usage bytes for all channels.
    ///
    /// This value will be used instead of `None`.
    default_max_bytes: usize,
}

/// Stores only replication channel by default.
impl Default for NetworkChannels {
    fn default() -> Self {
        Self {
            server: vec![(SendType::Unreliable, None)],
            client: vec![(SendType::Unreliable, None)],
            default_max_bytes: 5 * 1024 * 1024, // Value from `DefaultChannel::config()`.
        }
    }
}

impl NetworkChannels {
    /// Returns server channel configs that can be used to create [`ConnectionConfig`](bevy_renet::renet::ConnectionConfig).
    pub fn get_server_configs(&self) -> Vec<ChannelConfig> {
        self.get_configs(&self.server)
    }

    /// Same as [`Self::get_server_configs`], but for client.
    pub fn get_client_configs(&self) -> Vec<ChannelConfig> {
        self.get_configs(&self.client)
    }

    /// Sets maximum usage bytes for specific client channel.
    ///
    /// [`REPLICATION_CHANNEL_ID`] or [`EventChannel<T>`](crate::network_event::EventChannel) can be passed as `id`.
    /// Without calling this function, the default value will be used.
    /// See also [`Self::set_default_max_bytes`].
    pub fn set_server_max_bytes(&mut self, id: impl Into<u8>, max_bytes: usize) {
        let id = id.into() as usize;
        let (_, bytes) = self
            .server
            .get_mut(id)
            .unwrap_or_else(|| panic!("there is no server channel with id {id}"));

        *bytes = Some(max_bytes);
    }

    /// Same as [`Self::set_server_max_bytes`], but for client.
    pub fn set_client_max_bytes(&mut self, id: impl Into<u8>, max_bytes: usize) {
        let id = id.into();
        let (_, bytes) = self
            .client
            .get_mut(id as usize)
            .unwrap_or_else(|| panic!("there is no client channel with id {id}"));

        *bytes = Some(max_bytes);
    }

    /// Sets maximum usage bytes that will be used by default for all channels if not set.
    pub fn set_default_max_bytes(&mut self, max_bytes: usize) {
        self.default_max_bytes = max_bytes;
    }

    pub(super) fn create_client_channel(&mut self, send_type: SendType) -> u8 {
        if self.client.len() == u8::MAX as usize {
            panic!("number of client channels shouldn't exceed u8::MAX");
        }

        self.client.push((send_type, None));
        self.client.len() as u8 - 1
    }

    pub(super) fn create_server_channel(&mut self, send_type: SendType) -> u8 {
        if self.server.len() == u8::MAX as usize {
            panic!("number of server channels shouldn't exceed u8::MAX");
        }

        self.server.push((send_type, None));
        self.server.len() as u8 - 1
    }

    fn get_configs(&self, channels: &[(SendType, Option<usize>)]) -> Vec<ChannelConfig> {
        let mut channel_configs = Vec::with_capacity(channels.len());
        for (index, (send_type, max_bytes)) in channels.iter().enumerate() {
            channel_configs.push(ChannelConfig {
                channel_id: index as u8,
                max_memory_usage_bytes: max_bytes.unwrap_or(self.default_max_bytes),
                send_type: send_type.clone(),
            });
        }
        channel_configs
    }
}
