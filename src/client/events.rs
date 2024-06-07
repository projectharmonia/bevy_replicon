mod event_data;

use std::io::Cursor;

use bevy::{
    ecs::{entity::MapEntities, event::ManualEventReader},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::{replicon_client::RepliconClient, server_entity_map::ServerEntityMap, ClientSet};
use crate::{
    core::{
        channels::{RepliconChannel, RepliconChannels},
        common_conditions::*,
        ctx::{ClientSendCtx, ServerReceiveCtx},
        ClientId,
    },
    server::{replicon_server::RepliconServer, ServerSet},
};
use event_data::ClientEventData;

/// An extension trait for [`App`] for creating client events.
pub trait ClientEventAppExt {
    /// Registers [`FromClient<E>`] and `E` events.
    ///
    /// [`FromClient<E>`] will be emitted on the server after sending `E` event on client.
    /// In listen-server mode `E` will be drained right after sending and re-emitted as
    /// [`FromClient<E>`] with [`ClientId::SERVER`](crate::core::ClientId::SERVER).
    ///
    /// Can be called for events that were registered with [add_event](bevy::app::App::add_event).
    /// A duplicate registration for `E` won't be created.
    /// But be careful, since on listen servers all events `E` are drained,
    /// which could break other Bevy or third-party plugin systems that listen for `E`.
    ///
    /// See also [`Self::add_client_event_with`] and the [corresponding section](../index.html#from-client-to-server)
    /// from the quick start guide.
    fn add_client_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with(channel, default_serialize::<E>, default_deserialize::<E>)
    }

    /// Same as [`Self::add_client_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities.
    /// See also [`Self::add_client_event`].
    fn add_mapped_client_event<E: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with(
            channel,
            default_serialize_mapped::<E>,
            default_deserialize::<E>,
        )
    }

    /**
    Same as [`Self::add_client_event`], but uses the specified functions for serialization and deserialization.

    # Examples

    Register an event with [`Box<dyn Reflect>`]:

    ```
    use std::io::Cursor;

    use bevy::{
        prelude::*,
        reflect::serde::{ReflectSerializer, ReflectDeserializer},
    };
    use bevy_replicon::{
        core::ctx::{ClientSendCtx, ServerReceiveCtx},
        prelude::*,
    };
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_client_event_with(
        ChannelKind::Ordered,
        serialize_reflect,
        deserialize_reflect,
    );

    fn serialize_reflect(
        ctx: &mut ClientSendCtx,
        event: &ReflectEvent,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        let serializer = ReflectSerializer::new(&*event.0, ctx.registry);
        DefaultOptions::new().serialize_into(cursor, &serializer)
    }

    fn deserialize_reflect(
        ctx: &mut ServerReceiveCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<ReflectEvent> {
        let mut deserializer = bincode::Deserializer::with_reader(cursor, DefaultOptions::new());
        let reflect = ReflectDeserializer::new(ctx.registry).deserialize(&mut deserializer)?;
        Ok(ReflectEvent(reflect))
    }

    #[derive(Event)]
    struct ReflectEvent(Box<dyn Reflect>);
    ```
    */
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> &mut Self {
        self.add_event::<E>()
            .add_event::<FromClient<E>>()
            .init_resource::<ClientEventReader<E>>();

        let channel_id = self
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel.into());

        self.world_mut()
            .resource_scope(|world, mut event_registry: Mut<ClientEventRegistry>| {
                event_registry.0.push(ClientEventData::new(
                    world.components(),
                    channel_id,
                    serialize,
                    deserialize,
                ));
            });

        self
    }
}

/// Sending events from a client to the server.
///
/// Requires [`ClientPlugin`](super::ClientPlugin) for clients
/// and [`ServerPlugin`](crate::server::ServerPlugin) for the server.
pub struct ClientEventsPlugin;

impl Plugin for ClientEventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientEventRegistry>()
            .add_systems(
                PreUpdate,
                (
                    Self::reset.in_set(ClientSet::ResetEvents),
                    Self::receive
                        .in_set(ServerSet::Receive)
                        .run_if(server_running),
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
                    world.resource_scope(|world, event_registry: Mut<ClientEventRegistry>| {
                        let mut ctx = ClientSendCtx {
                            entity_map: &entity_map,
                            registry: &registry.read(),
                        };

                        let world_cell = world.as_unsafe_world_cell();
                        for event_data in &event_registry.0 {
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
        world.resource_scope(|world, mut server: Mut<RepliconServer>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, event_registry: Mut<ClientEventRegistry>| {
                    let mut ctx = ServerReceiveCtx {
                        registry: &registry.read(),
                    };

                    for event_data in &event_registry.0 {
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
        world.resource_scope(|world, event_registry: Mut<ClientEventRegistry>| {
            let world_cell = world.as_unsafe_world_cell();
            for event_data in &event_registry.0 {
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
        world.resource_scope(|world, event_registry: Mut<ClientEventRegistry>| {
            for event_data in &event_registry.0 {
                let events = world
                    .get_resource_mut_by_id(event_data.events_id())
                    .expect("events shouldn't be removed");

                // SAFETY: passed pointer was obtained using this event data.
                unsafe { event_data.reset(events.into_inner()) };
            }
        });
    }
}

/// Registered client events.
#[derive(Resource, Default)]
struct ClientEventRegistry(Vec<ClientEventData>);

/// Tracks read events for [`ClientEventPlugin::send`].
///
/// Unlike with server events, we don't always drain all events in [`ClientEventPlugin::resend_locally`].
#[derive(Resource, Deref, DerefMut)]
struct ClientEventReader<E: Event>(ManualEventReader<E>);

impl<E: Event> FromWorld for ClientEventReader<E> {
    fn from_world(world: &mut World) -> Self {
        let events = world.resource::<Events<E>>();
        Self(events.get_reader())
    }
}

/// Signature of client event serialization functions.
pub type SerializeFn<E> = fn(&mut ClientSendCtx, &E, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of client event deserialization functions.
pub type DeserializeFn<E> = fn(&mut ServerReceiveCtx, &mut Cursor<&[u8]>) -> bincode::Result<E>;

/// Default event serialization function.
pub fn default_serialize<E: Event + Serialize>(
    _ctx: &mut ClientSendCtx,
    event: &E,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(cursor, event)
}

/// Like [`default_serialize`], but also maps entities.
pub fn default_serialize_mapped<E: Event + MapEntities + Clone + Serialize>(
    ctx: &mut ClientSendCtx,
    event: &E,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    let mut event = event.clone();
    event.map_entities(ctx);
    DefaultOptions::new().serialize_into(cursor, &event)
}

/// Default event deserialization function.
pub fn default_deserialize<E: Event + DeserializeOwned>(
    _ctx: &mut ServerReceiveCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<E> {
    DefaultOptions::new().deserialize_from(cursor)
}

/// An event indicating that a message from client was received.
/// Emited only on server.
#[derive(Clone, Copy, Event)]
pub struct FromClient<T> {
    pub client_id: ClientId,
    pub event: T,
}
