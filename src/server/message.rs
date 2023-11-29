pub(super) mod replication_buffer;

use bevy::{ecs::component::Tick, prelude::*};
use bevy_renet::renet::{Bytes, ClientId, RenetServer};

use crate::replicon_core::{replicon_tick::RepliconTick, REPLICATION_CHANNEL_ID};
use replication_buffer::ReplicationBuffer;

/// A reusable message with replicated data for a client.
///
/// See also [Limits](../index.html#limits)
#[derive(Deref, DerefMut)]
pub(crate) struct ReplicationMessage {
    /// ID of a client for which this message is written.
    pub(super) client_id: ClientId,

    /// Last system tick acknowledged by the client.
    ///
    /// Used for changes preparation.
    pub(super) system_tick: Tick,

    /// Send message even if it doesn't contain replication data.
    ///
    /// See also [`Self::send_to`]
    send_empty: bool,

    /// Message data.
    #[deref]
    buffer: ReplicationBuffer,
}

impl ReplicationMessage {
    /// Creates a new message with assigned client ID.
    ///
    /// `replicon_tick` is the current tick that will be written into
    ///  the message to read by client on receive.
    ///
    /// `system_tick` is the last acknowledged system tick for this client.
    ///  Changes since this tick should be written into the message.
    ///
    /// If `send_empty` is set to `true`, then [`Self::send_to`]
    /// will send the message even if it doesn't contain any data.
    pub(super) fn new(
        replicon_tick: RepliconTick,
        client_id: ClientId,
        system_tick: Tick,
        send_empty: bool,
    ) -> bincode::Result<Self> {
        Ok(Self {
            client_id,
            system_tick,
            send_empty,
            buffer: ReplicationBuffer::new(replicon_tick)?,
        })
    }

    /// Clears the message and assigns it to a different client ID.
    ///
    /// Keeps allocated capacity of the buffer.
    pub(super) fn reset(
        &mut self,
        replicon_tick: RepliconTick,
        client_id: ClientId,
        system_tick: Tick,
        send_empty: bool,
    ) -> bincode::Result<()> {
        self.client_id = client_id;
        self.system_tick = system_tick;
        self.send_empty = send_empty;
        self.buffer.reset(replicon_tick)
    }

    /// Sends the message to the designated client.
    pub(super) fn send(&mut self, server: &mut RenetServer) {
        if !self.buffer.contains_data() && !self.send_empty {
            trace!("no changes to send for client {}", self.client_id);
            return;
        }

        self.buffer.trim_empty_arrays();

        trace!("sending replication message to client {}", self.client_id);
        server.send_message(
            self.client_id,
            REPLICATION_CHANNEL_ID,
            Bytes::copy_from_slice(self.buffer.as_slice()),
        );
    }
}
