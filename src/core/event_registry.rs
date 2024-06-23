pub(crate) mod client_event;
pub(crate) mod server_event;

use bevy::prelude::*;
use client_event::ClientEvent;
use server_event::ServerEvent;

/// Registered server and client events.
#[derive(Resource, Default)]
pub(crate) struct EventRegistry {
    server: Vec<ServerEvent>,
    client: Vec<ClientEvent>,
}

impl EventRegistry {
    pub(crate) fn register_server_event(&mut self, event_data: ServerEvent) {
        self.server.push(event_data);
    }

    pub(crate) fn register_client_event(&mut self, event_data: ClientEvent) {
        self.client.push(event_data);
    }

    pub(crate) fn iter_server_events(&self) -> impl Iterator<Item = &ServerEvent> {
        self.server.iter()
    }

    pub(crate) fn iter_client_events(&self) -> impl Iterator<Item = &ClientEvent> {
        self.client.iter()
    }
}
