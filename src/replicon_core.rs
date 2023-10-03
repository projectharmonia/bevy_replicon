pub mod replication_rules;

use std::cmp::Ordering;

use bevy::prelude::*;
use bevy_renet::renet::{ChannelConfig, SendType};
use serde::{Deserialize, Serialize};

use replication_rules::ReplicationRules;

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

/// A tick that increments each time we need the server to compute and send an update.
/// This is mapped to the bevy Tick in [`crate::server::AckedTicks`].
///
/// See also [`crate::server::TickPolicy`].
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Resource, Serialize)]
pub struct NetworkTick(u32);

impl NetworkTick {
    /// Creates a new [`NetworkTick`] wrapping the given value.
    #[inline]
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    /// Gets the value of this network tick.
    #[inline]
    pub fn get(self) -> u32 {
        self.0
    }

    /// Sets the value of this network tick.
    #[inline]
    pub fn set(&mut self, value: u32) {
        self.0 = value;
    }

    /// Increments current tick and takes wrapping into account.
    #[inline]
    pub fn increment(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_comparsion() {
        assert_eq!(NetworkTick(0), NetworkTick(0));
        assert!(NetworkTick(0) < NetworkTick(1));
        assert!(NetworkTick(0) > NetworkTick(u32::MAX));
    }
}
