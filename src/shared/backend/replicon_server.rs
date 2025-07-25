use bevy::prelude::*;
use bytes::Bytes;
use log::{debug, trace, warn};

/// Stores information about the server independent from the messaging backend.
///
/// The messaging backend is responsible for updating this resource:
/// - When the server starts or stops, [`Self::set_running`] should be used to update its status:
///   - Set to `true` in [`ServerSet::ReceivePackets`](crate::server::ServerSet::ReceivePackets) to
///     allow immediate processing of received messages.
///   - Set to `false` before [`ServerSet::Send`](crate::server::ServerSet::Send) to ensure
///     the server stops immediately on the same frame.
/// - For receiving messages, [`Self::insert_received`] should be used.
///   A system to forward messages from the backend to Replicon should run in [`ServerSet::ReceivePackets`](crate::server::ServerSet::ReceivePackets).
/// - For sending messages, [`Self::drain_sent`] should be used to drain all sent messages.
///   A system to forward messages from Replicon to the backend should run in [`ServerSet::SendPackets`](crate::server::ServerSet::SendPackets).
///
/// Inserted as resource by [`ServerPlugin`](crate::server::ServerPlugin).
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
    received_messages: Vec<Vec<(Entity, Bytes)>>,

    /// List of sent messages for each channel since the last tick.
    sent_messages: Vec<(Entity, usize, Bytes)>,
}

impl RepliconServer {
    /// Changes the size of the receive messages storage according to the number of client channels.
    pub(crate) fn setup_client_channels(&mut self, channels_count: usize) {
        self.received_messages.resize(channels_count, Vec::new());
    }

    /// Removes a disconnected client.
    pub(crate) fn remove_client(&mut self, client: Entity) {
        for receive_channel in &mut self.received_messages {
            receive_channel.retain(|&(entity, _)| entity != client);
        }
        self.sent_messages.retain(|&(entity, ..)| entity != client);
    }

    /// Receives all available messages from clients over a channel.
    ///
    /// All messages will be drained.
    pub(crate) fn receive<I: Into<usize>>(
        &mut self,
        channel_id: I,
    ) -> impl Iterator<Item = (Entity, Bytes)> + '_ {
        if !self.running {
            // We can't return here because we need to return an empty iterator.
            warn!("trying to receive a message when the server is not running");
        }

        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        if !channel_messages.is_empty() {
            trace!(
                "received {} message(s) totaling {} bytes from channel {channel_id}",
                channel_messages.len(),
                channel_messages
                    .iter()
                    .map(|(_, bytes)| bytes.len())
                    .sum::<usize>()
            );
        }

        channel_messages.drain(..)
    }

    /// Sends a message to a client over a channel.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn send<I: Into<usize>, B: Into<Bytes>>(
        &mut self,
        client: Entity,
        channel_id: I,
        message: B,
    ) {
        if !self.running {
            warn!("trying to send a message when the server is not running");
            return;
        }

        let channel_id = channel_id.into();
        let message: Bytes = message.into();

        trace!("sending {} bytes over channel {channel_id}", message.len());

        self.sent_messages.push((client, channel_id, message));
    }

    /// Marks the server as running or stopped.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend when the server changes its state.
    ///
    /// </div>
    pub fn set_running(&mut self, running: bool) {
        debug!("changing running status to `{running}`");

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
        F: FnMut(&(Entity, usize, Bytes)) -> bool,
    {
        self.sent_messages.retain(f)
    }

    /// Removes all sent messages, returning them as an iterator with client entity and channel.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn drain_sent(&mut self) -> impl Iterator<Item = (Entity, usize, Bytes)> + '_ {
        self.sent_messages.drain(..)
    }

    /// Adds a message from a client to the list of received messages.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn insert_received<I: Into<usize>, B: Into<Bytes>>(
        &mut self,
        client: Entity,
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
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        receive_channel.push((client, message.into()));
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::{
        prelude::*,
        shared::backend::channels::{ClientChannel, ServerChannel},
    };

    #[test]
    fn cleanup_on_stop() {
        let channels = RepliconChannels::default();
        let mut server = RepliconServer::default();
        server.setup_client_channels(channels.client_channels().len());
        server.set_running(true);

        server.send(Entity::PLACEHOLDER, ServerChannel::Mutations, Vec::new());
        server.insert_received(Entity::PLACEHOLDER, ClientChannel::MutationAcks, Vec::new());

        server.set_running(false);

        assert_eq!(server.drain_sent().count(), 0);
        assert_eq!(server.receive(ClientChannel::MutationAcks).count(), 0);
    }

    #[test]
    fn inactive() {
        let channels = RepliconChannels::default();
        let mut server = RepliconServer::default();
        server.setup_client_channels(channels.client_channels().len());

        server.send(Entity::PLACEHOLDER, ServerChannel::Mutations, Vec::new());
        server.insert_received(Entity::PLACEHOLDER, ClientChannel::MutationAcks, Vec::new());

        assert_eq!(server.drain_sent().count(), 0);
        assert_eq!(server.receive(ClientChannel::MutationAcks).count(), 0);
    }
}
