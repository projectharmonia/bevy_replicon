use bevy::prelude::*;

use crate::core::ClientId;

#[derive(Resource, Default)]
pub struct ConnectedClients {
    clients: Vec<ClientId>,
    replicate_after_connect: bool,
}

impl ConnectedClients {
    pub(crate) fn new(replicate_after_connect: bool) -> Self {
        Self {
            clients: Default::default(),
            replicate_after_connect,
        }
    }

    /// Gets if clients will automatically have replication enabled for them after they connect.
    pub fn replicate_after_connect(&self) -> bool {
        self.replicate_after_connect
    }

    /// Gets an iterator over all [`ClientId`]s currently tracked.
    pub fn iter(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients.iter().copied()
    }

    pub(crate) fn add(&mut self, client_id: ClientId) {
        debug!("adding connected `{client_id:?}`");

        self.clients.push(client_id);
    }

    pub(crate) fn remove(&mut self, client_id: ClientId) {
        debug!("removing disconnected `{client_id:?}`");

        let index = self
            .clients
            .iter()
            .position(|test_id| *test_id == client_id)
            .unwrap_or_else(|| panic!("{client_id:?} should be added before removal"));
        self.clients.remove(index);
    }
}
