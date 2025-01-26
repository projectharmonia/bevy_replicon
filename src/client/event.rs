use super::{ClientPlugin, ClientSet, ServerUpdateTick};
use crate::core::{
    common_conditions::*,
    event::{
        ctx::{ClientReceiveCtx, ClientSendCtx},
        event_registry::EventRegistry,
    },
    replicon_client::RepliconClient,
    server_entity_map::ServerEntityMap,
};
use bevy::{
    ecs::system::{FilteredResourcesMutParamBuilder, FilteredResourcesParamBuilder, ParamBuilder},
    prelude::*,
};

/// Sending events from a client to the server.
///
/// Requires [`ClientPlugin`].
/// Can be disabled for apps that act only as servers.
pub struct ClientEventPlugin;

impl Plugin for ClientEventPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // Construct systems dynamically after all plugins initialization
        // because we need to access resources by registered IDs.
        let event_registry = app
            .world_mut()
            .remove_resource::<EventRegistry>()
            .expect("event registry should be initialized on app build");

        let send = (
            FilteredResourcesParamBuilder::new(|builder| {
                for event in event_registry.iter_client_events() {
                    builder.add_read_by_id(event.events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_client_events() {
                    builder.add_write_by_id(event.reader_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(Self::send);

        let receive = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_server_events() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_server_events() {
                    builder.add_write_by_id(event.queue_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(Self::receive);

        let trigger = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for trigger in event_registry.iter_server_triggers() {
                    builder.add_write_by_id(trigger.event().events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(Self::trigger);

        let resend_locally = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_client_events() {
                    builder.add_write_by_id(event.client_events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_client_events() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(Self::resend_locally);

        let reset = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_client_events() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_server_events() {
                    builder.add_write_by_id(event.queue_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(Self::reset);

        app.insert_resource(event_registry)
            .add_systems(
                PreUpdate,
                (
                    reset.in_set(ClientSet::ResetEvents),
                    (
                        receive
                            .after(ClientPlugin::receive_replication)
                            .run_if(client_connected),
                        trigger.run_if(not(server_running)),
                    )
                        .chain()
                        .in_set(ClientSet::Receive),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    send.run_if(client_connected),
                    resend_locally.run_if(server_or_singleplayer),
                )
                    .chain()
                    .in_set(ClientSet::Send),
            );
    }
}

impl ClientEventPlugin {
    fn send(
        events: FilteredResources,
        mut readers: FilteredResourcesMut,
        mut client: ResMut<RepliconClient>,
        registry: Res<AppTypeRegistry>,
        entity_map: Res<ServerEntityMap>,
        event_registry: Res<EventRegistry>,
    ) {
        let mut ctx = ClientSendCtx {
            entity_map: &entity_map,
            registry: &registry.read(),
        };

        for event in event_registry.iter_client_events() {
            let events = events
                .get_by_id(event.events_id())
                .expect("events resource should be accessible");
            let reader = readers
                .get_mut_by_id(event.reader_id())
                .expect("event reader resource should be accessible");

            // SAFETY: passed pointers were obtained using this event data.
            unsafe {
                event.send(&mut ctx, &events, reader.into_inner(), &mut client);
            }
        }
    }

    fn receive(
        mut events: FilteredResourcesMut,
        mut queues: FilteredResourcesMut,
        mut client: ResMut<RepliconClient>,
        registry: Res<AppTypeRegistry>,
        entity_map: Res<ServerEntityMap>,
        event_registry: Res<EventRegistry>,
        update_tick: Res<ServerUpdateTick>,
    ) {
        let mut ctx = ClientReceiveCtx {
            registry: &registry.read(),
            entity_map: &entity_map,
            invalid_entities: Vec::new(),
        };

        for event in event_registry.iter_server_events() {
            let events = events
                .get_mut_by_id(event.events_id())
                .expect("events resource should be accessible");
            let queue = queues
                .get_mut_by_id(event.queue_id())
                .expect("queue resource should be accessible");

            // SAFETY: passed pointers were obtained using this event data.
            unsafe {
                event.receive(
                    &mut ctx,
                    events.into_inner(),
                    queue.into_inner(),
                    &mut client,
                    **update_tick,
                )
            };
        }
    }

    fn trigger(
        mut events: FilteredResourcesMut,
        mut commands: Commands,
        event_registry: Res<EventRegistry>,
    ) {
        for trigger in event_registry.iter_server_triggers() {
            let events = events
                .get_mut_by_id(trigger.event().events_id())
                .expect("events resource should be accessible");
            trigger.trigger(&mut commands, events.into_inner());
        }
    }

    fn resend_locally(
        mut client_events: FilteredResourcesMut,
        mut events: FilteredResourcesMut,
        event_registry: Res<EventRegistry>,
    ) {
        for event in event_registry.iter_client_events() {
            let client_events = client_events
                .get_mut_by_id(event.client_events_id())
                .expect("client events resource should be accessible");
            let events = events
                .get_mut_by_id(event.events_id())
                .expect("events resource should be accessible");

            // SAFETY: passed pointers were obtained using this event data.
            unsafe { event.resend_locally(client_events.into_inner(), events.into_inner()) };
        }
    }

    fn reset(
        mut events: FilteredResourcesMut,
        mut queues: FilteredResourcesMut,
        event_registry: Res<EventRegistry>,
    ) {
        for event in event_registry.iter_client_events() {
            let events = events
                .get_mut_by_id(event.events_id())
                .expect("events resource should be accessible");

            // SAFETY: passed pointer was obtained using this event data.
            unsafe { event.reset(events.into_inner()) };
        }

        for event in event_registry.iter_server_events() {
            let queue = queues
                .get_mut_by_id(event.queue_id())
                .expect("event queue resource should be accessible");

            // SAFETY: passed pointer was obtained using this event data.
            unsafe { event.reset(queue.into_inner()) };
        }
    }
}
