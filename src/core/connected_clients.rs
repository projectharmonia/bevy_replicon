use bevy::prelude::*;

use crate::core::ClientId;

/// Contains all connected clients.
///
/// Inserted as resource by [`ServerPlugin`](crate::server::ServerPlugin).
///
/// See also [ReplicatedClients](super::replication::replicated_clients::ReplicatedClients).
#[derive(Resource, Default, Debug, Deref)]
pub struct ConnectedClients(Vec<ConnectedClient>);

impl ConnectedClients {
    pub(crate) fn add(&mut self, client_id: ClientId) {
        debug!("adding connected `{client_id:?}`");

        self.0.push(ConnectedClient::new(client_id));
    }

    pub(crate) fn remove(&mut self, client_id: ClientId) {
        debug!("removing disconnected `{client_id:?}`");

        let index = self
            .iter()
            .position(|client| client.id == client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be added before removal"));
        self.0.remove(index);
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ConnectedClient> {
        self.0.iter_mut()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectedClient {
    id: ClientId,
    rtt: f64,
    packet_loss: f64,
}

impl ConnectedClient {
    pub fn new(id: ClientId) -> Self {
        Self {
            id,
            rtt: 0.0,
            packet_loss: 0.0,
        }
    }

    /// Returns the associated ID.
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Returns the round-time trip for the connection.
    ///
    /// Returns zero if not provided by the backend.
    pub fn rtt(&self) -> f64 {
        self.rtt
    }

    /// Sets the round-time trip for the connection.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn set_rtt(&mut self, rtt: f64) {
        self.rtt = rtt;
    }

    /// Returns the packet loss for the connection.
    ///
    /// Returns zero if not provided by the backend.
    pub fn packet_loss(&self) -> f64 {
        self.packet_loss
    }

    /// Sets the packet loss for the connection.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn set_packet_loss(&mut self, packet_loss: f64) {
        self.packet_loss = packet_loss;
    }
}
