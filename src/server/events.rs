use bevy::prelude::*;

use super::{ServerPlugin, ServerSet};
use crate::core::{
    common_conditions::*,
    connected_clients::ConnectedClients,
    ctx::{ServerReceiveCtx, ServerSendCtx},
    event_registry::EventRegistry,
    replicon_server::RepliconServer,
};

/// Sending events from the server to clients.
///
/// Requires [`ServerPlugin`].
/// Can be disabled for apps that act only as clients.
pub struct ServerEventsPlugin;

impl Plugin for ServerEventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            Self::receive
                .in_set(ServerSet::Receive)
                .run_if(server_running),
        )
        .add_systems(
            PostUpdate,
            (
                Self::send.run_if(server_running),
                Self::resend_locally.run_if(has_authority),
            )
                .chain()
                .after(ServerPlugin::send_replication)
                .in_set(ServerSet::Send),
        );
    }
}

impl ServerEventsPlugin {
    fn send(world: &mut World) {
        world.resource_scope(|world, mut server: Mut<RepliconServer>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, connected_clients: Mut<ConnectedClients>| {
                    world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
                        let mut ctx = ServerSendCtx {
                            registry: &registry.read(),
                        };

                        for event_data in event_registry.iter_server_events() {
                            let server_events = world
                                .get_resource_by_id(event_data.server_events_id())
                                .expect("server events shouldn't be removed");

                            // SAFETY: passed pointer was obtained using this event data.
                            unsafe {
                                event_data.send(
                                    &mut ctx,
                                    &server_events,
                                    &mut server,
                                    &connected_clients,
                                );
                            }
                        }
                    });
                });
            });
        });
    }

    fn receive(world: &mut World) {
        world.resource_scope(|world, mut server: Mut<RepliconServer>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
                    let mut ctx = ServerReceiveCtx {
                        registry: &registry.read(),
                    };

                    for event_data in event_registry.iter_client_events() {
                        let client_events = world
                            .get_resource_mut_by_id(event_data.client_events_id())
                            .expect("client events shouldn't be removed");

                        // SAFETY: passed pointer was obtained using this event data.
                        unsafe {
                            event_data.receive(&mut ctx, client_events.into_inner(), &mut server)
                        };
                    }
                });
            });
        });
    }

    fn resend_locally(world: &mut World) {
        world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
            let world_cell = world.as_unsafe_world_cell();
            for event_data in event_registry.iter_server_events() {
                // SAFETY: both resources mutably borrowed uniquely.
                let (server_events, events) = unsafe {
                    let server_events = world_cell
                        .get_resource_mut_by_id(event_data.server_events_id())
                        .expect("server events shouldn't be removed");
                    let events = world_cell
                        .get_resource_mut_by_id(event_data.events_id())
                        .expect("events shouldn't be removed");
                    (server_events, events)
                };

                // SAFETY: passed pointers were obtained using this event data.
                unsafe {
                    event_data.resend_locally(server_events.into_inner(), events.into_inner())
                };
            }
        });
    }
}
