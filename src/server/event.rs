use bevy::prelude::*;

use super::{server_tick::ServerTick, ServerPlugin, ServerSet};
use crate::core::{
    common_conditions::*,
    connected_clients::ConnectedClients,
    event::{
        ctx::{ServerReceiveCtx, ServerSendCtx},
        event_registry::EventRegistry,
        server_event::BufferedServerEvents,
    },
    replication::replicated_clients::ReplicatedClients,
    replicon_server::RepliconServer,
};

/// Sending events from the server to clients.
///
/// Requires [`ServerPlugin`].
/// Can be disabled for apps that act only as clients.
pub struct ServerEventPlugin;

impl Plugin for ServerEventPlugin {
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
                Self::send_or_buffer.run_if(server_running),
                Self::send_buffered
                    .run_if(server_running)
                    .run_if(resource_changed::<ServerTick>),
                Self::resend_locally.run_if(server_or_singleplayer),
            )
                .chain()
                .after(ServerPlugin::send_replication)
                .in_set(ServerSet::Send),
        );
    }
}

impl ServerEventPlugin {
    fn send_or_buffer(world: &mut World) {
        world.resource_scope(|world, mut server: Mut<RepliconServer>| {
            world.resource_scope(|world, mut buffered_events: Mut<BufferedServerEvents>| {
                let registry = world.resource::<AppTypeRegistry>();
                let mut ctx = ServerSendCtx {
                    registry: &registry.read(),
                };
                let connected_clients = world.resource::<ConnectedClients>();
                let event_registry = world.resource::<EventRegistry>();

                buffered_events.start_tick();

                for event_data in event_registry.iter_server_events() {
                    let server_events = world
                        .get_resource_by_id(event_data.server_events_id())
                        .expect("server events shouldn't be removed");

                    // SAFETY: passed pointer was obtained using this event data.
                    unsafe {
                        event_data.send_or_buffer(
                            &mut ctx,
                            &server_events,
                            &mut server,
                            connected_clients,
                            &mut buffered_events,
                        );
                    }
                }
            });
        });
    }

    fn send_buffered(world: &mut World) {
        world.resource_scope(|world, mut server: Mut<RepliconServer>| {
            world.resource_scope(|world, mut buffered_events: Mut<BufferedServerEvents>| {
                let replicated_clients = world.resource::<ReplicatedClients>();
                buffered_events
                    .send_all(&mut server, replicated_clients)
                    .expect("buffered server events should send");
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
