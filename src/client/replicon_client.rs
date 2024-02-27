use bevy::prelude::*;
use bytes::Bytes;

use crate::core::PeerId;

/// Stores information about client independent from messaging library.
///
/// Messaging library responsible for updating this resource:
/// - When messaging client changes its status (connected, connecting and disconnected),
/// [`Self::set_status`] should be used to reflect this.
/// - When [`Self::is_connected`] returns `false` while messaging client is connected,
/// it should gracefully disconnect.
/// - For sending messages [`Self::iter_sent`] should be used to drain all sent messages.
/// Corresponding system should run in [`ClientSet::SendPackets`](super::ClientSet::SendPackets).
/// - For receiving messages [`Self::insert_received`] should be to used.
/// Corresponding system should run in [`ClientSet::ReceivePackets`](super::ClientSet::ReceivePackets).
#[derive(Resource, Default)]
pub struct RepliconClient {
    /// Client connection status.
    status: RepliconClientStatus,

    /// List of sent messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores sent messages since the last tick.
    sent_messages: Vec<Vec<Bytes>>,

    /// List of received messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores received messages since the last tick.
    received_messages: Vec<Vec<Bytes>>,
}

impl RepliconClient {
    /// Changes the size of the message storage according to the number of channels.
    pub(super) fn setup_channels(
        &mut self,
        server_channels_count: usize,
        client_channels_count: usize,
    ) {
        self.sent_messages.resize(client_channels_count, Vec::new());
        self.received_messages
            .resize(server_channels_count, Vec::new());
    }

    /// Receives a message from the server over a channel.
    pub fn receive<I: Into<u8>>(&mut self, channel_id: I) -> Option<Bytes> {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("client should have a receive channel with id {channel_id}"));

        channel_messages.pop()
    }

    /// Sends a message to the server over a channel.
    pub fn send<I: Into<u8>, B: Into<Bytes>>(&mut self, channel_id: I, message: B) {
        let channel_id = channel_id.into();
        let channel_messages = self
            .sent_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("client should have a send channel with id {channel_id}"));

        channel_messages.push(message.into());
    }

    /// Sets client connection status.
    ///
    /// Should be called by messaging library when the client status changes or by user to disconnect.
    /// Cleanups all messages if the state changes from [`RepliconClientStatus::Connected`].
    /// See also [`Self::status`].
    pub fn set_status(&mut self, status: RepliconClientStatus) {
        if self.is_connected() && !matches!(status, RepliconClientStatus::Connected { .. }) {
            self.sent_messages.clear();
            self.received_messages.clear();
        }

        self.status = status;
    }

    /// Sets client connection status to [`RepliconClientStatus`].
    pub fn disconnect(&mut self) {
        self.set_status(RepliconClientStatus::NoConnection);
    }

    /// Returns current client status.
    ///
    /// See also [`Self::set_status`].
    #[inline]
    pub fn status(&self) -> RepliconClientStatus {
        self.status
    }

    /// Returns `true` if the client doesn't have a connection.
    ///
    /// See also [`Self::status`].
    #[inline]
    pub fn is_no_connection(&self) -> bool {
        matches!(self.status, RepliconClientStatus::NoConnection)
    }

    /// Returns `true` if the client is connecting.
    ///
    /// See also [`Self::status`].
    #[inline]
    pub fn is_connecting(&self) -> bool {
        matches!(self.status, RepliconClientStatus::Connecting)
    }

    /// Returns `true` if the client is connected.
    ///
    /// See also [`Self::status`].
    #[inline]
    pub fn is_connected(&self) -> bool {
        matches!(self.status, RepliconClientStatus::Connected { .. })
    }

    /// Returns client's ID.
    ///
    /// It's available only if the client state is [`RepliconClientStatus::Connected`].
    /// See also [`Self::status`].
    #[inline]
    pub fn peer_id(&self) -> Option<PeerId> {
        if let RepliconClientStatus::Connected { peer_id } = self.status {
            peer_id
        } else {
            None
        }
    }

    /// Returns iterator over all messages for each channel.
    ///
    /// Should be called only by messaging library.
    pub fn iter_sent(&mut self) -> impl Iterator<Item = (u8, &mut Vec<Bytes>)> + '_ {
        self.sent_messages
            .iter_mut()
            .enumerate()
            .map(|(channel_id, messages)| (channel_id as u8, messages))
    }

    /// Adds the message from server to the list of received.
    ///
    /// Should be called only by messaging library.
    pub fn insert_received<I: Into<u8>, B: Into<Bytes>>(&mut self, message: B, channel_id: I) {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("client should have a channel with id {channel_id}"));

        channel_messages.push(message.into());
    }
}

/// Connection status of the [`RepliconClient`].
#[derive(Clone, Copy, Default, PartialEq)]
pub enum RepliconClientStatus {
    /// Connected or disconnected.
    #[default]
    NoConnection,
    /// Trying to connect to server.
    Connecting,
    /// Connected to server.
    ///
    /// Stores the assigned ID if one was assigned by the server.
    /// Needed only for users to access ID independent from messaging library.
    Connected { peer_id: Option<PeerId> },
}
