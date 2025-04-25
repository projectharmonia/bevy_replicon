use bevy::prelude::*;
use log::debug;

/// A resource with all channels used by Replicon.
///
/// Channel IDs are represented by [`usize`], but backends may limit the number of channels.
///
/// The first two channels are used for replication. For more details, see [`ReplicationChannel`].
///
/// Other channels are used for events, with one channel per event. For more details, see
/// [`RemoteEventRegistry`](crate::shared::event::remote_event_registry::RemoteEventRegistry).
///
/// The backend needs to provide an API for creating its own channels. This can be done
/// by writing an extension trait for this struct. Created channels should have the defined
/// delivery guarantee or stronger.
#[derive(Clone, Resource)]
pub struct RepliconChannels {
    /// Stores settings for each server channel.
    server: Vec<Channel>,

    /// Same as [`Self::server`], but for client.
    client: Vec<Channel>,
}

/// Only stores the replication channel by default.
impl Default for RepliconChannels {
    fn default() -> Self {
        Self {
            server: vec![
                ReplicationChannel::Updates.into(),
                ReplicationChannel::Mutations.into(),
            ],
            client: vec![
                ReplicationChannel::Updates.into(),
                ReplicationChannel::Mutations.into(),
            ],
        }
    }
}

impl RepliconChannels {
    /// Creates a new server channel and returns its ID.
    pub(crate) fn create_server_channel(&mut self, channel: Channel) -> usize {
        let id = self.server.len();
        debug!("creating a server channel with ID {id}");
        self.server.push(channel);

        id
    }

    /// Creates a new client channel and returns its ID.
    pub(crate) fn create_client_channel(&mut self, channel: Channel) -> usize {
        let id = self.client.len();
        debug!("creating a client channel with ID {id}");
        self.client.push(channel);

        id
    }

    /// Returns registered server channels.
    pub fn server_channels(&self) -> &[Channel] {
        &self.server
    }

    /// Returns registered client channels.
    pub fn client_channels(&self) -> &[Channel] {
        &self.client
    }
}

/// ID of a server replication channel.
///
/// To synchronize the state, we send only changes using Bevy's change detection.
///
/// We can't use only a reliable channel because of how reliability is implement. Messages are split into packets
/// based on the MTU and are considered received only if all their packets are received. If any packet is dropped,
/// it gets resent with the same data. However, on the client, we care only about the latest data. For example:
///
/// - Tick 1, position X - received.
/// - Tick 2, position Y - missed.
/// - Tick 3, position Z - received.
///
/// By tick 3, we no longer care about the missing position from tick 2, but it will still be resent.
///
/// We also can't use only an unreliable channel. We could implement a custom acknowledgment system on top of it
/// and resend the latest data if a message is lost. However, partial updates would break the game logic.
/// For example, if a component that references an entity is lost, we can't resend it with the new entity
/// because the client might not have received the entity yet.
///
/// This is why we use a dual-channel approach to send data.
///
/// For everything except mutations, we use a reliable channel. This data can't be outdated and is sent in
/// a single update message for each tick to ensure atomic updates.
///
/// For mutations, we use an unreliable channel. This data can be outdated, so we always send the latest values
/// since the last acknowledgment. We also include a minimum required tick - the tick on which the last update
/// message was sent. Messages will be buffered until an update message for this tick is received. Mutations
/// are split into packet-size messages to allow applying them partially without waiting for all parts of the message.
/// We guarantee that all mutations for a single entity arrive won't be split across messages, even if they are larger
/// than the packet size. You can also ensure that mutations for specific entities arrive in sync by using
/// [`SyncRelatedAppExt::sync_related_entities`](crate::shared::replication::related_entities::SyncRelatedAppExt::sync_related_entities).
///
/// Server events also have minimum required tick. For details, see the documentation on
/// [`ServerEventAppExt::make_independent`](crate::shared::event::server_event::ServerEventAppExt::make_independent).
///
/// See also [`RepliconChannels`], [`Channel`] and [corresponding section](../index.html#eventual-consistency)
/// from the quick start guide.
pub enum ReplicationChannel {
    /// For sending messages with entity mappings, inserts, removals and despawns.
    ///
    /// This is an ordered reliable channel.
    Updates,
    /// For sending messages with component mutations.
    ///
    /// This is an unreliable channel.
    Mutations,
}

impl From<ReplicationChannel> for Channel {
    fn from(value: ReplicationChannel) -> Self {
        match value {
            ReplicationChannel::Updates => Channel::Ordered,
            ReplicationChannel::Mutations => Channel::Unreliable,
        }
    }
}

impl From<ReplicationChannel> for usize {
    fn from(value: ReplicationChannel) -> Self {
        value as usize
    }
}

/// Channel delivery guarantee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    /// Unreliable and unordered.
    Unreliable,
    /// Reliable and unordered.
    Unordered,
    /// Reliable and ordered.
    Ordered,
}
