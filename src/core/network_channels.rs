use std::time::Duration;

use bevy::prelude::*;
use bevy_renet::renet::{ChannelConfig, SendType};

/// ID of a server replication channel.
///
/// See also [`NetworkChannels`].
#[repr(u8)]
pub enum ReplicationChannel {
    /// For sending messages with entity mappings, inserts, removals and despawns.
    Reliable,
    /// For sending messages with component updates.
    Unreliable,
}

impl From<ReplicationChannel> for u8 {
    fn from(value: ReplicationChannel) -> Self {
        value as u8
    }
}

/// A resource to configure and setup channels for [`ConnectionConfig`](bevy_renet::renet::ConnectionConfig).
#[derive(Clone, Resource)]
pub struct NetworkChannels {
    /// Stores settings for each server channel.
    server: Vec<ChannelSettings>,

    /// Same as [`Self::server`], but for client.
    client: Vec<ChannelSettings>,

    /// Stores the default max memory usage bytes for all channels.
    ///
    /// This value will be used instead of `None`.
    default_max_bytes: usize,
}

/// Only stores the replication channel by default.
impl Default for NetworkChannels {
    fn default() -> Self {
        let replication_channels = vec![
            ChannelSettings {
                send_type: SendType::ReliableOrdered {
                    resend_time: Duration::ZERO,
                },
                max_bytes: None,
            },
            ChannelSettings {
                send_type: SendType::Unreliable,
                max_bytes: None,
            },
        ];

        Self {
            server: replication_channels.clone(),
            client: replication_channels,
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

    /// Sets the maximum usage bytes for a specific server channel.
    ///
    /// [`ReplicationChannel`] or [`ServerEventChannel<T>`](crate::network_event::server_event::ServerEventChannel)
    /// can be passed as `id`.
    /// Without calling this function, the default value will be used.
    /// See also [`Self::set_default_max_bytes`].
    pub fn set_server_max_bytes(&mut self, id: impl Into<u8>, max_bytes: usize) {
        let id = id.into() as usize;
        let settings = self
            .server
            .get_mut(id)
            .unwrap_or_else(|| panic!("there is no server channel with id {id}"));

        settings.max_bytes = Some(max_bytes);
    }

    /// Same as [`Self::set_server_max_bytes`], but for a client channel.
    pub fn set_client_max_bytes(&mut self, id: impl Into<u8>, max_bytes: usize) {
        let id = id.into();
        let settings = self
            .client
            .get_mut(id as usize)
            .unwrap_or_else(|| panic!("there is no client channel with id {id}"));

        settings.max_bytes = Some(max_bytes);
    }

    /// Sets the maximum usage bytes that will be used by default for all channels if not set.
    pub fn set_default_max_bytes(&mut self, max_bytes: usize) {
        self.default_max_bytes = max_bytes;
    }

    /// Creates a new client channel with the specified send type.
    pub fn create_client_channel(&mut self, send_type: SendType) -> u8 {
        if self.client.len() == u8::MAX as usize {
            panic!("number of client channels shouldn't exceed u8::MAX");
        }

        self.client.push(ChannelSettings {
            send_type,
            max_bytes: None,
        });
        self.client.len() as u8 - 1
    }

    /// Creates a new server channel with the specified send type.
    pub fn create_server_channel(&mut self, send_type: SendType) -> u8 {
        if self.server.len() == u8::MAX as usize {
            panic!("number of server channels shouldn't exceed u8::MAX");
        }

        self.server.push(ChannelSettings {
            send_type,
            max_bytes: None,
        });
        self.server.len() as u8 - 1
    }

    fn get_configs(&self, channels: &[ChannelSettings]) -> Vec<ChannelConfig> {
        let mut channel_configs = Vec::with_capacity(channels.len());
        for (index, settings) in channels.iter().enumerate() {
            channel_configs.push(ChannelConfig {
                channel_id: index as u8,
                max_memory_usage_bytes: settings.max_bytes.unwrap_or(self.default_max_bytes),
                send_type: settings.send_type.clone(),
            });
        }
        channel_configs
    }
}

/// Channel configuration.
#[derive(Clone)]
struct ChannelSettings {
    /// Delivery guarantee.
    send_type: SendType,

    /// Maximum usage bytes (if set) for each server channel.
    max_bytes: Option<usize>,
}
