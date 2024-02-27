use bevy::{prelude::*, utils::HashMap};
use bytes::Bytes;

use crate::core::PeerId;

/// Stores information about server independent from messaging library(-ies).
///
/// Messaging library(-ies) responsible for updating this resource:
/// - When server is activated or deactivated, [`Self::set_active`] should be used to reflect this.
/// - When [`Self::is_active`] returns `false` while messaging server is still active,
/// the server should stop.
/// - For sending messages [`Self::iter_sent`] should be used to drain all sent messages.
/// Corresponding system should run in [`ServerSet::SendPackets`](super::ServerSet::SendPackets).
/// - For receiving messages [`Self::insert_received`] should be to used.
/// Corresponding system should run in [`ServerSet::ReceivePackets`](super::ServerSet::ReceivePackets).
#[derive(Resource, Default)]
pub struct RepliconServer {
    /// `true` if server is open for connections.
    ///
    /// By default set to `false`.
    active: bool,

    /// List of sent messages for each channel where top index is ID.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores sent messages since the last tick.
    sent_messages: Vec<Vec<(PeerId, Bytes)>>,

    /// List of received messages for each channel where top index is ID.
    ///
    /// Top index is channel ID.
    /// Inner hash map stores sent messages since the last tick.
    ///
    /// Unlike in `sent_messages`, we use a hash map here for quick access
    /// to messages for a client from other system.
    received_messages: Vec<HashMap<PeerId, Vec<Bytes>>>,

    /// [`Vec`]'s from disconnected clients.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    client_buffer: Vec<Vec<Bytes>>,
}

impl RepliconServer {
    /// Changes the size of the message storage according to the number of channels.
    pub(super) fn setup_channels(
        &mut self,
        server_channels_count: usize,
        client_channels_count: usize,
    ) {
        self.sent_messages.resize(server_channels_count, Vec::new());
        self.received_messages
            .resize(client_channels_count, HashMap::new());
    }

    /// Initializes a message storage for a client.
    ///
    /// Reuses the memory from previously disconnected clients if available.
    pub(super) fn add_client(&mut self, peer_id: PeerId) {
        for channel_messages in &mut self.received_messages {
            let client_messages = self.client_buffer.pop().unwrap_or_default();
            channel_messages.insert(peer_id, client_messages);
        }
    }

    /// Removes a connected client.
    ///
    /// Keeps allocated memory for reuse.
    pub(super) fn remove_client(&mut self, peer_id: PeerId) {
        for channel_messages in &mut self.received_messages {
            let client_messages = channel_messages
                .remove(&peer_id)
                .unwrap_or_else(|| panic!("{peer_id:?} should be added before removal"));
            self.client_buffer.push(client_messages);
        }
    }

    /// Creates a new instance and marks it as active.
    pub fn active() -> Self {
        Self {
            active: true,
            ..Default::default()
        }
    }

    /// Receives a message from a client over a channel.
    pub fn receive<I: Into<u8>>(&mut self, peer_id: PeerId, channel_id: I) -> Option<Bytes> {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        channel_messages.get_mut(&peer_id)?.pop()
    }

    /// Sends a message to a client over a channel.
    pub fn send<I: Into<u8>, B: Into<Bytes>>(
        &mut self,
        peer_id: PeerId,
        channel_id: I,
        message: B,
    ) {
        let channel_id = channel_id.into();
        let channel_messages = self
            .sent_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a send channel with id {channel_id}"));

        channel_messages.push((peer_id, message.into()));
    }

    /// Marks server as active or inactive.
    ///
    /// Should be called from library(-ies) when the server changes its status
    /// or by user to deactivate the server.
    pub fn set_active(&mut self, active: bool) {
        if !active {
            self.sent_messages.clear();
            self.received_messages.clear();
        }

        self.active = active;
    }

    /// Returns `true` if the server is active.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Returns iterator over all messages for each channel.
    ///
    /// Should be called only by library(-ies).
    pub fn iter_sent(&mut self) -> impl Iterator<Item = (u8, &mut Vec<(PeerId, Bytes)>)> + '_ {
        self.sent_messages
            .iter_mut()
            .enumerate()
            .map(|(channel_id, messages)| (channel_id as u8, messages))
    }

    /// Adds the message from the client to the list of received.
    ///
    /// Should be called only by library(-ies).
    pub fn insert_received<I: Into<u8>, B: Into<Bytes>>(
        &mut self,
        peer_id: PeerId,
        message: B,
        channel_id: I,
    ) {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        let client_messages = channel_messages
            .get_mut(&peer_id)
            .unwrap_or_else(|| panic!("{peer_id:?} should be connected to send messages"));
        client_messages.push(message.into());
    }
}
