use super::{ClientPlugin, ClientSet, ServerInitTick};
use crate::core::{
    common_conditions::*,
    ctx::{ClientReceiveCtx, ClientSendCtx},
    event_registry::EventRegistry,
    replicon_client::RepliconClient,
    replicon_tick::RepliconTick,
    server_entity_map::ServerEntityMap,
};
use bevy::prelude::*;
use bytes::Bytes;
use ordered_multimap::ListOrderedMultimap;

/// Sending events from a client to the server.
///
/// Requires [`ClientPlugin`].
/// Can be disabled for apps that act only as servers.
pub struct ClientEventsPlugin;

impl Plugin for ClientEventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerEventQueue>()
            .add_systems(
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
                    Self::resend_locally.run_if(has_authority),
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
                        world.resource_scope(|world, mut queue: Mut<ServerEventQueue>| {
                            let init_tick = **world.resource::<ServerInitTick>();
                            let mut ctx = ClientReceiveCtx {
                                registry: &registry.read(),
                                entity_map: &entity_map,
                                invalid_entities: Vec::new(),
                            };

                            for event_data in event_registry.iter_server_events() {
                                let events = world
                                    .get_resource_mut_by_id(event_data.events_id())
                                    .expect("events shouldn't be removed");

                                // SAFETY: passed pointers were obtained using this event data.
                                unsafe {
                                    event_data.receive(
                                        &mut ctx,
                                        events.into_inner(),
                                        &mut queue,
                                        &mut client,
                                        init_tick,
                                    )
                                };
                            }
                        });
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

            let mut queue = world.resource_mut::<ServerEventQueue>();
            if !queue.is_empty() {
                warn!(
                    "discarding {} queued server events due to a disconnect",
                    queue.values_len()
                );
                queue.clear();
            }
        });
    }
}

/// Stores all received events from server that arrived earlier then replication message with their tick.
///
/// Stores data sorted by ticks and maintains order of arrival.
/// Needed to ensure that when an event is triggered, all the data that it affects or references already exists.
#[derive(Resource, Deref, DerefMut, Default)]
pub(crate) struct ServerEventQueue(ListOrderedMultimap<RepliconTick, Bytes>);

impl ServerEventQueue {
    /// Pops the next event that is at least as old as the specified replicon tick.
    pub(crate) fn pop_if_le(&mut self, init_tick: RepliconTick) -> Option<(RepliconTick, Bytes)> {
        let (tick, _) = self.0.front()?;
        if *tick > init_tick {
            return None;
        }
        self.0
            .pop_front()
            .map(|(tick, message)| (tick.into_owned(), message))
    }
}
