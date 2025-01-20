use bevy::prelude::*;

use super::{client_event::ClientEvent, server_event::ServerEvent};

/// Registered server and client events.
#[derive(Resource, Default)]
pub(crate) struct EventRegistry {
    server: Vec<ServerEvent>,
    client: Vec<ClientEvent>,
}

impl EventRegistry {
    pub(super) fn register_server_event(&mut self, event_data: ServerEvent) {
        self.server.push(event_data);
    }

    pub(super) fn register_client_event(&mut self, event_data: ClientEvent) {
        self.client.push(event_data);
    }

    pub(crate) fn iter_server_events_mut(&mut self) -> impl Iterator<Item = &mut ServerEvent> {
        self.server.iter_mut()
    }

    pub(crate) fn iter_server_events(&self) -> impl Iterator<Item = &ServerEvent> {
        self.server.iter()
    }

    pub(crate) fn iter_client_events(&self) -> impl Iterator<Item = &ClientEvent> {
        self.client.iter()
    }
}
