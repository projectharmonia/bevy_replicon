use super::{ClientPlugin, ClientSet, ServerInitTick};
use crate::core::{
    common_conditions::*,
    event_registry::{
        ctx::{ClientReceiveCtx, ClientSendCtx},
        EventRegistry,
    },
    replicon_client::RepliconClient,
    server_entity_map::ServerEntityMap,
};
use bevy::prelude::*;

/// Sending events from a client to the server.
///
/// Requires [`ClientPlugin`].
/// Can be disabled for apps that act only as servers.
pub struct ClientEventsPlugin;

impl Plugin for ClientEventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                Self::reset.in_set(ClientSet::ResetEvents),
                Self::receive
                    .after(ClientPlugin::receive_replication)
                    .in_set(ClientSet::Receive)
                    .run_if(client_connected),
            ),
        )
        .add_systems(
            PostUpdate,
            (
                Self::send.run_if(client_connected),
                Self::resend_locally.run_if(server_or_singleplayer),
            )
                .chain()
                .in_set(ClientSet::Send),
        );
    }
}

impl ClientEventsPlugin {
    fn send(world: &mut World) {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, entity_map: Mut<ServerEntityMap>| {
                    world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
                        let mut ctx = ClientSendCtx {
                            entity_map: &entity_map,
                            registry: &registry.read(),
                        };

                        let world_cell = world.as_unsafe_world_cell();
                        for event_data in event_registry.iter_client_events() {
                            // SAFETY: both resources mutably borrowed uniquely.
                            let (events, reader) = unsafe {
                                let events = world_cell
                                    .get_resource_by_id(event_data.events_id())
                                    .expect("events shouldn't be removed");
                                let reader = world_cell
                                    .get_resource_mut_by_id(event_data.reader_id())
                                    .expect("event reader shouldn't be removed");
                                (events, reader)
                            };

                            // SAFETY: passed pointers were obtained using this event data.
                            unsafe {
                                event_data.send(
                                    &mut ctx,
                                    &events,
                                    reader.into_inner(),
                                    &mut client,
                                );
                            }
                        }
                    });
                });
            });
        });
    }

    fn receive(world: &mut World) {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, entity_map: Mut<ServerEntityMap>| {
                    world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
                        let init_tick = **world.resource::<ServerInitTick>();
                        let mut ctx = ClientReceiveCtx {
                            registry: &registry.read(),
                            entity_map: &entity_map,
                            invalid_entities: Vec::new(),
                        };

                        let world_cell = world.as_unsafe_world_cell();
                        for event_data in event_registry.iter_server_events() {
                            // SAFETY: both resources mutably borrowed uniquely.
                            let (events, queue) = unsafe {
                                let events = world_cell
                                    .get_resource_mut_by_id(event_data.events_id())
                                    .expect("events shouldn't be removed");
                                let queue = world_cell
                                    .get_resource_mut_by_id(event_data.queue_id())
                                    .expect("event queue shouldn't be removed");
                                (events, queue)
                            };

                            // SAFETY: passed pointers were obtained using this event data.
                            unsafe {
                                event_data.receive(
                                    &mut ctx,
                                    events.into_inner(),
                                    queue.into_inner(),
                                    &mut client,
                                    init_tick,
                                )
                            };
                        }
                    });
                });
            });
        });
    }

    fn resend_locally(world: &mut World) {
        world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
            let world_cell = world.as_unsafe_world_cell();
            for event_data in event_registry.iter_client_events() {
                // SAFETY: both resources mutably borrowed uniquely.
                let (client_events, events) = unsafe {
                    let client_events = world_cell
                        .get_resource_mut_by_id(event_data.client_events_id())
                        .expect("client events shouldn't be removed");
                    let events = world_cell
                        .get_resource_mut_by_id(event_data.events_id())
                        .expect("events shouldn't be removed");
                    (client_events, events)
                };

                // SAFETY: passed pointers were obtained using this event data.
                unsafe {
                    event_data.resend_locally(client_events.into_inner(), events.into_inner())
                };
            }
        });
    }

    fn reset(world: &mut World) {
        world.resource_scope(|world, event_registry: Mut<EventRegistry>| {
            for event_data in event_registry.iter_client_events() {
                let events = world
                    .get_resource_mut_by_id(event_data.events_id())
                    .expect("events shouldn't be removed");

                // SAFETY: passed pointer was obtained using this event data.
                unsafe { event_data.reset(events.into_inner()) };
            }

            for event_data in event_registry.iter_server_events() {
                let queue = world
                    .get_resource_mut_by_id(event_data.queue_id())
                    .expect("event queue shouldn't be removed");

                // SAFETY: passed pointer was obtained using this event data.
                unsafe { event_data.reset(queue.into_inner()) };
            }
        });
    }
}
