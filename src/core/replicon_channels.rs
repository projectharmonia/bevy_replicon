use std::time::Duration;

use bevy::prelude::*;

/// ID of a server replication channel.
///
/// See also [`RepliconChannels`].
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

/// A resource with channels used by Replicon.
#[derive(Clone, Resource)]
pub struct RepliconChannels {
    /// Stores settings for each server channel.
    server: Vec<RepliconChannel>,

    /// Same as [`Self::server`], but for client.
    client: Vec<RepliconChannel>,

    /// Stores the default max memory usage bytes for all channels.
    ///
    /// This value will be used instead of `None`.
    /// By default set to `5 * 1024 * 1024`.
    pub default_max_bytes: usize,
}

/// Only stores the replication channel by default.
impl Default for RepliconChannels {
    fn default() -> Self {
        Self {
            server: vec![ChannelKind::Ordered.into(), ChannelKind::Unreliable.into()],
            client: vec![ChannelKind::Ordered.into(), ChannelKind::Unreliable.into()],
            default_max_bytes: 5 * 1024 * 1024,
        }
    }
}

impl RepliconChannels {
    /// Sets the maximum usage bytes that will be used by default for all channels if not set.
    pub fn set_default_max_bytes(&mut self, max_bytes: usize) {
        self.default_max_bytes = max_bytes;
    }

    /// Creates a new client channel and returns its ID.
    ///
    /// # Panics
    ///
    /// Panics number of events exceed [`u8::MAX`].
    pub fn create_client_channel(&mut self, channel: RepliconChannel) -> u8 {
        if self.client.len() == u8::MAX.into() {
            panic!("number of client channels shouldn't exceed `u8::MAX`");
        }

        self.client.push(channel);
        self.client.len() as u8 - 1
    }

    /// Creates a new server channel and returns its ID.
    ///
    /// # Panics
    ///
    /// Panics number of events exceed [`u8::MAX`].
    pub fn create_server_channel(&mut self, channel: RepliconChannel) -> u8 {
        if self.server.len() == u8::MAX.into() {
            panic!("number of server channels shouldn't exceed `u8::MAX`");
        }

        self.server.push(channel);
        self.server.len() as u8 - 1
    }

    /// Returns the number of server channels.
    pub fn server_channels(&self) -> &[RepliconChannel] {
        &self.server
    }

    /// Returns the number of client channels.
    pub fn client_channels(&self) -> &[RepliconChannel] {
        &self.client
    }
}

/// Channel configuration.
#[derive(Clone)]
pub struct RepliconChannel {
    /// Delivery guarantee.
    pub kind: ChannelKind,

    /// Timer after which the message will be sent again if it has not been confirmed.
    ///
    /// Ignored for [`ChannelKind::Unreliable`].
    pub resend_time: Duration,

    /// Maximum usage bytes for the channel.
    ///
    /// If unset, the default value from [`RepliconChannels`] will be used.
    pub max_bytes: Option<usize>,
}

/// Channel delivery guarantee.
///
/// Can be automatically converted into [`RepliconChannel`] with zero resend time and unset max bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelKind {
    /// Unreliable and unordered.
    Unreliable,
    /// Reliable and unordered.
    Unordered,
    /// Reliable and ordered.
    Ordered,
}

impl From<ChannelKind> for RepliconChannel {
    fn from(value: ChannelKind) -> Self {
        Self {
            kind: value,
            resend_time: Duration::ZERO,
            max_bytes: None,
        }
    }
}
