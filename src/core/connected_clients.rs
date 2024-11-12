use bevy::prelude::*;

use crate::core::ClientId;

/// Contains all connected clients.
///
/// Inserted as resource by [`ServerPlugin`](crate::server::ServerPlugin).
///
/// See also [ReplicatedClients](super::replication::replicated_clients::ReplicatedClients).
#[derive(Resource, Default, Deref)]
pub struct ConnectedClients(Vec<ClientId>);

impl ConnectedClients {
    pub(crate) fn add(&mut self, client_id: ClientId) {
        debug!("adding connected `{client_id:?}`");

        self.0.push(client_id);
    }

    pub(crate) fn remove(&mut self, client_id: ClientId) {
        debug!("removing disconnected `{client_id:?}`");

        let index = self
            .iter()
            .position(|test_id| *test_id == client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be added before removal"));
        self.0.remove(index);
    }
}
