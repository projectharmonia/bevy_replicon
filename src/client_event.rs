use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{
    ecs::{
        component::{ComponentId, Components},
        entity::MapEntities,
        event::ManualEventReader,
    },
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    client::{replicon_client::RepliconClient, server_entity_map::ServerEntityMap, ClientSet},
    core::{
        common_conditions::*,
        ctx::{ClientSendCtx, ServerReceiveCtx},
        replicon_channels::{RepliconChannel, RepliconChannels},
        ClientId,
    },
    server::{replicon_server::RepliconServer, ServerSet},
};

/// An extension trait for [`App`] for creating client events.
pub trait ClientEventAppExt {
    /// Registers [`FromClient<E>`] and `E` events.
    ///
    /// [`FromClient<E>`] will be emitted on the server after sending `E` event on client.
    /// In listen-server mode `E` will be drained right after sending and re-emitted as
    /// [`FromClient<E>`] with [`ClientId::SERVER`](crate::core::ClientId::SERVER).
    ///
    /// Can be called for already existing regular events, a duplicate registration for
    /// `E` won't be created. But be careful, since on listen server all events `E` will be drained,
    /// it could break other Bevy or third-party plugin systems that listen for `E`.
    ///
    /// It can be registered for an already existing events. It won't create a duplicate event, but
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
        reflect::serde::{ReflectSerializer, UntypedReflectDeserializer},
    };
    use bevy_replicon::{
        core::ctx::{ClientSendCtx, ServerReceiveCtx},
        prelude::*,
    };
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_client_event_with::<ReflectEvent>(
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
        let reflect =
            UntypedReflectDeserializer::new(ctx.registry).deserialize(&mut deserializer)?;
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
            .world
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel.into());

        self.world
            .resource_scope(|world, mut client_events: Mut<ClientEvents>| {
                client_events.0.push(ClientEvent::new(
                    world.components(),
                    channel_id,
                    serialize,
                    deserialize,
                ));
            });

        self
    }
}

pub struct ClientEventPlugin;

impl Plugin for ClientEventPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientEvents>()
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

impl ClientEventPlugin {
    fn send(world: &mut World) {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, entity_map: Mut<ServerEntityMap>| {
                    world.resource_scope(|world, client_events: Mut<ClientEvents>| {
                        let mut ctx = ClientSendCtx {
                            entity_map: &entity_map,
                            registry: &registry.read(),
                        };

                        let world_cell = world.as_unsafe_world_cell();
                        for client_event in &client_events.0 {
                            // SAFETY: both resources mutably borrowed uniquely.
                            let (events, reader) = unsafe {
                                let events = world_cell
                                    .get_resource_by_id(client_event.events_id)
                                    .expect("events shouldn't be removed");
                                let reader = world_cell
                                    .get_resource_mut_by_id(client_event.reader_id)
                                    .expect("event reader shouldn't be removed");
                                (events, reader)
                            };

                            // SAFETY: passed pointers were obtained from this event IDs.
                            unsafe {
                                client_event.send(
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
                world.resource_scope(|world, client_events: Mut<ClientEvents>| {
                    let mut ctx = ServerReceiveCtx {
                        registry: &registry.read(),
                    };

                    for client_event in &client_events.0 {
                        let client_events = world
                            .get_resource_mut_by_id(client_event.client_events_id)
                            .expect("client events shouldn't be removed");

                        // SAFETY: passed pointer was obtained from this event ID.
                        unsafe {
                            client_event.receive(&mut ctx, client_events.into_inner(), &mut server)
                        };
                    }
                });
            });
        });
    }

    fn resend_locally(world: &mut World) {
        world.resource_scope(|world, client_events: Mut<ClientEvents>| {
            let world_cell = world.as_unsafe_world_cell();
            for client_event in &client_events.0 {
                // SAFETY: both resources mutably borrowed uniquely.
                let (events, client_events) = unsafe {
                    let events = world_cell
                        .get_resource_mut_by_id(client_event.events_id)
                        .expect("events shouldn't be removed");
                    let client_events = world_cell
                        .get_resource_mut_by_id(client_event.client_events_id)
                        .expect("client events shouldn't be removed");
                    (events, client_events)
                };

                // SAFETY: passed pointers were obtained from this event IDs.
                unsafe {
                    client_event.resend_locally(events.into_inner(), client_events.into_inner())
                };
            }
        });
    }

    fn reset(world: &mut World) {
        world.resource_scope(|world, client_events: Mut<ClientEvents>| {
            for client_event in &client_events.0 {
                let events = world
                    .get_resource_mut_by_id(client_event.events_id)
                    .expect("events shouldn't be removed");

                // SAFETY: passed pointer was obtained from this event ID.
                unsafe { client_event.reset(events.into_inner()) };
            }
        });
    }
}

/// Registered client events.
#[derive(Resource, Default)]
struct ClientEvents(Vec<ClientEvent>);

/// Type-erased functions and metadata for a registered client event.
///
/// Needed to process all events in a single system and call its functions without knowing the type.
struct ClientEvent {
    type_id: TypeId,
    type_name: &'static str,

    /// ID of [`Events<E>`].
    events_id: ComponentId,

    /// ID of [`ClientEventReader<E>`].
    reader_id: ComponentId,

    /// ID of [`Events<ToClients<E>>`].
    client_events_id: ComponentId,

    /// Used channel.
    channel_id: u8,

    send: SendFn,
    receive: ReceiveFn,
    resend_locally: ResendLocallyFn,
    reset: ResetFn,
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
}

impl ClientEvent {
    fn new<E: Event>(
        components: &Components,
        channel_id: u8,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> Self {
        let events_id = components.resource_id::<Events<E>>().unwrap_or_else(|| {
            panic!(
                "event `{}` should be previously registered",
                any::type_name::<E>()
            )
        });
        let client_events_id = components
            .resource_id::<Events<FromClient<E>>>()
            .unwrap_or_else(|| {
                panic!(
                    "event `{}` should be previously registered",
                    any::type_name::<FromClient<E>>()
                )
            });
        let reader_id = components
            .resource_id::<ClientEventReader<E>>()
            .unwrap_or_else(|| {
                panic!(
                    "resource `{}` should be previously inserted",
                    any::type_name::<ClientEventReader<E>>()
                )
            });

        // SAFETY: these functions won't be called until the type is restored.
        Self {
            type_id: TypeId::of::<E>(),
            type_name: any::type_name::<E>(),
            events_id,
            reader_id,
            client_events_id,
            channel_id,
            send: send::<E>,
            receive: receive::<E>,
            resend_locally: resend_locally::<E>,
            reset: reset::<E>,
            serialize: unsafe { mem::transmute(serialize) },
            deserialize: unsafe { mem::transmute(deserialize) },
        }
    }

    /// Sends an event to the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `reader` is [`ClientEventReader<E>`]
    /// and this instance was created for `E`.
    unsafe fn send(
        &self,
        ctx: &mut ClientSendCtx,
        events: &Ptr,
        reader: PtrMut,
        client: &mut RepliconClient,
    ) {
        (self.send)(self, ctx, events, reader, client);
    }

    /// Receives an event from a client.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E`.
    unsafe fn receive(
        &self,
        ctx: &mut ServerReceiveCtx,
        client_events: PtrMut,
        server: &mut RepliconServer,
    ) {
        (self.receive)(self, ctx, client_events, server);
    }

    /// Drains events `E` and re-emits them as [`FromClient<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E`.
    unsafe fn resend_locally(&self, events: PtrMut, client_events: PtrMut) {
        (self.resend_locally)(events, client_events);
    }

    /// Drains all events.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`]
    /// and this instance was created for `E`.
    unsafe fn reset(&self, events: PtrMut) {
        (self.reset)(events);
    }

    /// Serializes an event into a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    unsafe fn serialize<E: Event>(
        &self,
        ctx: &mut ClientSendCtx,
        event: &E,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        self.check_type::<E>();
        let serialize: SerializeFn<E> = std::mem::transmute(self.serialize);
        (serialize)(ctx, event, cursor)
    }

    /// Deserializes an event into a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    unsafe fn deserialize<E: Event>(
        &self,
        ctx: &mut ServerReceiveCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<E> {
        self.check_type::<E>();
        let deserialize: DeserializeFn<E> = std::mem::transmute(self.deserialize);
        (deserialize)(ctx, cursor)
    }

    fn check_type<C: Event>(&self) {
        debug_assert_eq!(
            self.type_id,
            TypeId::of::<C>(),
            "trying to call event functions with {}, but they were created with {}",
            any::type_name::<C>(),
            self.type_name,
        );
    }
}

/// Tracks read events for [`ClientEventPlugin::send`].
///
/// Unlike with server events, we not always drain all events in [`ClientEventPlugin::resend_locally`].
#[derive(Resource, Deref, DerefMut)]
struct ClientEventReader<E: Event>(ManualEventReader<E>);

impl<E: Event> FromWorld for ClientEventReader<E> {
    fn from_world(world: &mut World) -> Self {
        let events = world.resource::<Events<E>>();
        Self(events.get_reader())
    }
}

/// Signature of client event sending functions.
type SendFn = unsafe fn(&ClientEvent, &mut ClientSendCtx, &Ptr, PtrMut, &mut RepliconClient);

/// Signature of client event receiving functions.
type ReceiveFn = unsafe fn(&ClientEvent, &mut ServerReceiveCtx, PtrMut, &mut RepliconServer);

/// Signature of client event resending functions.
type ResendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of client event reset functions.
type ResetFn = unsafe fn(PtrMut);

/// Signature of client event serialization functions.
pub type SerializeFn<E> = fn(&mut ClientSendCtx, &E, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of client event deserialization functions.
pub type DeserializeFn<E> = fn(&mut ServerReceiveCtx, &mut Cursor<&[u8]>) -> bincode::Result<E>;

/// Typed version of [`ClientEvent::send`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<FromClient<E>>`], `reader` is [`ClientEventReader<E>`]
/// and `client_event` was created for `E`.
unsafe fn send<E: Event>(
    client_event: &ClientEvent,
    ctx: &mut ClientSendCtx,
    events: &Ptr,
    reader: PtrMut,
    client: &mut RepliconClient,
) {
    let reader: &mut ClientEventReader<E> = reader.deref_mut();
    for event in reader.read(events.deref()) {
        let mut cursor = Default::default();
        client_event
            .serialize::<E>(ctx, event, &mut cursor)
            .expect("client event should be serializable");

        trace!("sending event `{}`", any::type_name::<E>());
        client.send(client_event.channel_id, cursor.into_inner());
    }
}

/// Typed version of [`ClientEvent::receive`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<E>`]
/// and `client_event` was created for `E`.
unsafe fn receive<E: Event>(
    client_event: &ClientEvent,
    ctx: &mut ServerReceiveCtx,
    events: PtrMut,
    server: &mut RepliconServer,
) {
    let events: &mut Events<FromClient<E>> = events.deref_mut();
    for (client_id, message) in server.receive(client_event.channel_id) {
        let mut cursor = Cursor::new(&*message);
        match client_event.deserialize::<E>(ctx, &mut cursor) {
            Ok(event) => {
                trace!(
                    "applying event `{}` from `{client_id:?}`",
                    any::type_name::<E>()
                );
                events.send(FromClient { client_id, event });
            }
            Err(e) => debug!("unable to deserialize event from {client_id:?}: {e}"),
        }
    }
}

/// Typed version of [`ClientEvent::resend_locally`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<E>`] and `server_events` is [`Events<ToClients<E>>`].
unsafe fn resend_locally<E: Event>(events: PtrMut, client_events: PtrMut) {
    let client_events: &mut Events<FromClient<E>> = client_events.deref_mut();
    let events: &mut Events<E> = events.deref_mut();
    client_events.send_batch(events.drain().map(|event| FromClient {
        client_id: ClientId::SERVER,
        event,
    }));
}

/// Typed version of [`ClientEvent::reset`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<E>`].
unsafe fn reset<E: Event>(events: PtrMut) {
    let events: &mut Events<E> = events.deref_mut();
    let drained_count = events.drain().count();
    if drained_count > 0 {
        warn!("discarded {drained_count} client events due to a disconnect");
    }
}

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
