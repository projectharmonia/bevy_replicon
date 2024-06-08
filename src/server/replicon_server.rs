use bevy::prelude::*;
use bytes::Bytes;

use crate::core::ClientId;

/// Stores information about the server independent from the messaging backend.
///
/// The messaging backend is responsible for updating this resource:
/// - When the server is started or stopped, [`Self::set_running`] should be used to reflect this.
/// - For receiving messages, [`Self::insert_received`] should be used.
/// A system to forward messages from the backend to Replicon should run in [`ServerSet::ReceivePackets`](super::ServerSet::ReceivePackets).
/// - For sending messages, [`Self::drain_sent`] should be used to drain all sent messages.
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
    /// Inner [`Vec`] stores received messages since the last tick.
    received_messages: Vec<Vec<(ClientId, Bytes)>>,

    /// List of sent messages for each channel since the last tick.
    sent_messages: Vec<(ClientId, u8, Bytes)>,
}

impl RepliconServer {
    /// Changes the size of the receive messages storage according to the number of client channels.
    pub(super) fn setup_client_channels(&mut self, channels_count: usize) {
        self.received_messages.resize(channels_count, Vec::new());
    }

    /// Removes a disconnected client.
    pub(super) fn remove_client(&mut self, client_id: ClientId) {
        for receive_channel in &mut self.received_messages {
            receive_channel.retain(|&(sender_id, _)| sender_id != client_id);
        }
        self.sent_messages
            .retain(|&(sender_id, ..)| sender_id != client_id);
    }

    /// Receives all available messages from clients over a channel.
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
        let channel_messages = self
            .received_messages
            .get_mut(channel_id as usize)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        trace!(
            "received {} message(s) from channel {channel_id}",
            channel_messages.len()
        );

        channel_messages.drain(..)
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

        let channel_id: u8 = channel_id.into();
        let message: Bytes = message.into();

        trace!("sending {} bytes over channel {channel_id}", message.len());

        self.sent_messages.push((client_id, channel_id, message));
    }

    /// Marks the server as running or stopped.
    ///
    /// Should be called only from the messaging backend when the server changes its state.
    pub fn set_running(&mut self, running: bool) {
        debug!("changing `RepliconServer` running status to `{running}`");

        if !running {
            for receive_channel in &mut self.received_messages {
                receive_channel.clear();
            }
            self.sent_messages.clear();
        }

        self.running = running;
    }

    /// Returns `true` if the server is running.
    #[inline]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Retains only the messages specified by the predicate.
    ///
    /// Used for testing.
    pub(crate) fn retain_sent<F>(&mut self, f: F)
    where
        F: FnMut(&(ClientId, u8, Bytes)) -> bool,
    {
        self.sent_messages.retain(f)
    }

    /// Removes all sent messages, returning them as an iterator with client ID and channel.
    ///
    /// Should be called only from the messaging backend.
    pub fn drain_sent(&mut self) -> impl Iterator<Item = (ClientId, u8, Bytes)> + '_ {
        self.sent_messages.drain(..)
    }

    /// Adds a message from a client to the list of received messages.
    ///
    /// Should be called only from the messaging backend.
    pub fn insert_received<I: Into<u8>, B: Into<Bytes>>(
        &mut self,
        client_id: ClientId,
        channel_id: I,
        message: B,
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
