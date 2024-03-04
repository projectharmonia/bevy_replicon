use bevy::prelude::*;
use bytes::Bytes;

use crate::core::ClientId;

/// Stores information about the server independent from the messaging backend.
///
/// The messaging backend is responsible for updating this resource:
/// - When the server is started or stopped, [`Self::set_running`] should be used to reflect this.
/// - For receiving messages, [`Self::insert_received`] should be used.
/// A system to forward messages from the backend to Replicon should run in [`ServerSet::ReceivePackets`](super::ServerSet::ReceivePackets).
/// - For sending messages, [`Self::iter_sent_mut`] should be used to drain all sent messages.
/// A system to forward messages from Replicon to the backend should run in [`ServerSet::SendPackets`](super::ServerSet::SendPackets).
#[derive(Resource, Default)]
pub struct RepliconServer {
    /// Indicates if the server is open for connections.
    ///
    /// By default set to `false`.
    running: bool,

    /// List of received messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner hash map stores sent messages since the last tick.
    received_messages: Vec<Vec<(ClientId, Bytes)>>,

    /// List of sent messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores sent messages since the last tick.
    sent_messages: Vec<Vec<(ClientId, Bytes)>>,
}

impl RepliconServer {
    /// Changes the size of the message storage according to the number of channels.
    pub(super) fn setup_channels(
        &mut self,
        server_channels_count: usize,
        client_channels_count: usize,
    ) {
        self.received_messages
            .resize(client_channels_count, Vec::new());
        self.sent_messages.resize(server_channels_count, Vec::new());
    }

    /// Removes a disconnected client.
    ///
    /// Keeps allocated memory for reuse.
    pub(super) fn remove_client(&mut self, client_id: ClientId) {
        for receive_channel in &mut self.received_messages {
            receive_channel.retain(|&(sender_id, _)| sender_id != client_id);
        }
        for send_channel in &mut self.sent_messages {
            send_channel.retain(|&(sender_id, _)| sender_id != client_id);
        }
    }

    /// Receives all available messages from over a channel.
    ///
    /// All messages will be drained.
    pub fn receive<I: Into<u8>>(
        &mut self,
        channel_id: I,
    ) -> impl Iterator<Item = (ClientId, Bytes)> + '_ {
        if !self.running {
            // We can't return here because we need to return an empty iterator.
            warn!("trying to receive a message when the server is not running");
        }

        let channel_id = channel_id.into();
        let receive_channel = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        receive_channel.drain(..)
    }

    /// Sends a message to a client over a channel.
    pub fn send<I: Into<u8>, B: Into<Bytes>>(
        &mut self,
        client_id: ClientId,
        channel_id: I,
        message: B,
    ) {
        if !self.running {
            warn!("trying to send a message when the server is not running");
            return;
        }

        let channel_id = channel_id.into();
        let send_channel = self
            .sent_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a send channel with id {channel_id}"));

        send_channel.push((client_id, message.into()));
    }

    /// Marks the server as running or stopped.
    ///
    /// Should be called only from the messaging backend when the server changes its state.
    pub fn set_running(&mut self, running: bool) {
        if !running {
            for receive_channel in &mut self.received_messages {
                receive_channel.clear();
            }
            for send_channel in &mut self.sent_messages {
                send_channel.clear();
            }
        }

        self.running = running;
    }

    /// Returns `true` if the server is running.
    #[inline]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Returns iterator over all messages for each channel.
    ///
    /// Should be called only from messaging library.
    pub fn iter_sent_mut(
        &mut self,
    ) -> impl Iterator<Item = (u8, &mut Vec<(ClientId, Bytes)>)> + '_ {
        self.sent_messages
            .iter_mut()
            .enumerate()
            .map(|(channel_id, messages)| (channel_id as u8, messages))
    }

    /// Adds a message from a client to the list of received messages.
    ///
    /// Should be called only from the messaging backend.
    pub fn insert_received<I: Into<u8>, B: Into<Bytes>>(
        &mut self,
        client_id: ClientId,
        message: B,
        channel_id: I,
    ) {
        if !self.running {
            warn!("trying to insert a received message when the server is not running");
            return;
        }

        let channel_id = channel_id.into();
        let receive_channel = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        receive_channel.push((client_id, message.into()));
    }
}
