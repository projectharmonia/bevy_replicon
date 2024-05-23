use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{
    ecs::{
        component::{ComponentId, Components},
        entity::MapEntities,
    },
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;
use ordered_multimap::ListOrderedMultimap;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    client::{server_entity_map::ServerEntityMap, ClientSet, ServerInitTick},
    core::{
        common_conditions::*,
        ctx::{ClientReceiveCtx, ServerSendCtx},
        replicon_tick::RepliconTick,
        ClientId,
    },
    server::{replicon_server::RepliconServer, ServerSet},
    ClientPlugin, ConnectedClient, ConnectedClients, RepliconChannel, RepliconChannels,
    RepliconClient, ServerPlugin,
};

/// An extension trait for [`App`] for creating client events.
pub trait ServerEventAppExt {
    /// Registers `E` and [`ToClients<E>`] events.
    ///
    /// `E` will be emitted on client after sending [`ToClients<E>`] on the server.
    /// If [`ClientId::SERVER`] is a recipient of the event, then [`ToClients<E>`] will be drained
    /// after sending to clients and `E` events will be emitted on the server.
    ///
    /// Can be called for already existing regular events, a duplicate registration
    /// for `E` won't be created.
    ///
    /// See also [`Self::add_server_event_with`] and the [corresponding section](../index.html#from-server-to-client)
    /// from the quick start guide.
    fn add_server_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_server_event_with(channel, default_serialize::<E>, default_deserialize::<E>)
    }

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client inside the event after receiving.
    ///
    /// Always use it for events that contain entities.
    /// See also [`Self::add_server_event`].
    fn add_mapped_server_event<E: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_server_event_with(
            channel,
            default_serialize::<E>,
            default_deserialize_mapped::<E>,
        )
    }

    /**
    Same as [`Self::add_server_event`], but uses the specified functions for serialization and deserialization.

    # Examples

    Register an event with [`Box<dyn Reflect>`]:

    ```
    use std::io::Cursor;

    use bevy::{
        prelude::*,
        reflect::serde::{ReflectSerializer, UntypedReflectDeserializer},
    };
    use bevy_replicon::{
        core::ctx::{ClientReceiveCtx, ServerSendCtx},
        prelude::*,
    };
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_server_event_with::<ReflectEvent>(
        ChannelKind::Ordered,
        serialize_reflect,
        deserialize_reflect,
    );

    fn serialize_reflect(
        ctx: &mut ServerSendCtx,
        event: &ReflectEvent,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        let serializer = ReflectSerializer::new(&*event.0, ctx.registry);
        DefaultOptions::new().serialize_into(cursor, &serializer)
    }

    fn deserialize_reflect(
        ctx: &mut ClientReceiveCtx,
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
    fn add_server_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> &mut Self {
        self.add_event::<E>()
            .add_event::<ToClients<E>>()
            .init_resource::<ServerEventQueue<E>>();

        let channel_id = self
            .world
            .resource_mut::<RepliconChannels>()
            .create_server_channel(channel.into());

        self.world
            .resource_scope(|world, mut event_registry: Mut<ServerEventRegistry>| {
                event_registry.0.push(ServerEventData::new(
                    world.components(),
                    channel_id,
                    serialize,
                    deserialize,
                ));
            });

        self
    }
}

pub struct ServerEventPlugin;

impl Plugin for ServerEventPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerEventRegistry>()
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
                    Self::send.run_if(server_running),
                    Self::resend_locally.run_if(has_authority),
                )
                    .chain()
                    .after(ServerPlugin::send_replication)
                    .in_set(ServerSet::Send),
            );
    }
}

impl ServerEventPlugin {
    fn send(world: &mut World) {
        world.resource_scope(|world, mut server: Mut<RepliconServer>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, connected_clients: Mut<ConnectedClients>| {
                    world.resource_scope(|world, event_registry: Mut<ServerEventRegistry>| {
                        let mut ctx = ServerSendCtx {
                            registry: &registry.read(),
                        };

                        for event_data in &event_registry.0 {
                            let server_events = world
                                .get_resource_by_id(event_data.server_events_id)
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
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
                world.resource_scope(|world, entity_map: Mut<ServerEntityMap>| {
                    world.resource_scope(|world, event_registry: Mut<ServerEventRegistry>| {
                        let init_tick = **world.resource::<ServerInitTick>();
                        let mut ctx = ClientReceiveCtx {
                            registry: &registry.read(),
                            entity_map: &entity_map,
                        };

                        let world_cell = world.as_unsafe_world_cell();
                        for event_data in &event_registry.0 {
                            // SAFETY: both resources mutably borrowed uniquely.
                            let (events, queue) = unsafe {
                                let events = world_cell
                                    .get_resource_mut_by_id(event_data.events_id)
                                    .expect("events shouldn't be removed");
                                let queue = world_cell
                                    .get_resource_mut_by_id(event_data.queue_id)
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
        world.resource_scope(|world, event_registry: Mut<ServerEventRegistry>| {
            let world_cell = world.as_unsafe_world_cell();
            for event_data in &event_registry.0 {
                // SAFETY: both resources mutably borrowed uniquely.
                let (server_events, events) = unsafe {
                    let server_events = world_cell
                        .get_resource_mut_by_id(event_data.server_events_id)
                        .expect("server events shouldn't be removed");
                    let events = world_cell
                        .get_resource_mut_by_id(event_data.events_id)
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

    fn reset(world: &mut World) {
        world.resource_scope(|world, event_registry: Mut<ServerEventRegistry>| {
            for event_data in &event_registry.0 {
                let events = world
                    .get_resource_mut_by_id(event_data.events_id)
                    .expect("events shouldn't be removed");

                // SAFETY: passed pointer was obtained using this event data.
                unsafe { event_data.reset(events.into_inner()) };
            }
        });
    }
}

/// Registered server events.
#[derive(Resource, Default)]
struct ServerEventRegistry(Vec<ServerEventData>);

/// Type-erased functions and metadata for a registered server event.
///
/// Needed so events of different types can be processed together.
struct ServerEventData {
    type_id: TypeId,
    type_name: &'static str,

    /// ID of [`Events<E>`].
    events_id: ComponentId,

    /// ID of [`Events<ToClients<E>>`].
    server_events_id: ComponentId,

    /// ID of [`ServerEventQueue<T>`].
    queue_id: ComponentId,

    /// Used channel.
    channel_id: u8,

    send: SendFn,
    receive: ReceiveFn,
    resend_locally: ResendLocallyFn,
    reset: ResetFn,
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
}

impl ServerEventData {
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
        let server_events_id = components
            .resource_id::<Events<ToClients<E>>>()
            .unwrap_or_else(|| {
                panic!(
                    "event `{}` should be previously registered",
                    any::type_name::<ToClients<E>>()
                )
            });
        let queue_id = components
            .resource_id::<ServerEventQueue<E>>()
            .unwrap_or_else(|| {
                panic!(
                    "resource `{}` should be previously inserted",
                    any::type_name::<ServerEventQueue<E>>()
                )
            });

        // SAFETY: these functions won't be called until the type is restored.
        Self {
            type_id: TypeId::of::<E>(),
            type_name: any::type_name::<E>(),
            events_id,
            server_events_id,
            queue_id,
            channel_id,
            send: send::<E>,
            receive: receive::<E>,
            resend_locally: resend_locally::<E>,
            reset: reset::<E>,
            serialize: unsafe { mem::transmute(serialize) },
            deserialize: unsafe { mem::transmute(deserialize) },
        }
    }

    /// Sends an event to client(s).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<ToClients<E>>`]
    /// and this instance was created for `E`.
    unsafe fn send(
        &self,
        ctx: &mut ServerSendCtx,
        server_events: &Ptr,
        server: &mut RepliconServer,
        connected_clients: &ConnectedClients,
    ) {
        (self.send)(self, ctx, server_events, server, connected_clients);
    }

    /// Receives an event from the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `queue` is [`ServerEventQueue<E>`],
    /// and this instance was created for `E`.
    unsafe fn receive(
        &self,
        ctx: &mut ClientReceiveCtx,
        events: PtrMut,
        queue: PtrMut,
        client: &mut RepliconClient,
        init_tick: RepliconTick,
    ) {
        (self.receive)(self, ctx, events, queue, client, init_tick);
    }

    /// Drains events [`ToClients<E>`] and re-emits them as `E` if the server is in the list of the event recipients.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `server_events` is [`Events<ToClients<E>>`],
    /// and this instance was created for `E`.
    unsafe fn resend_locally(&self, server_events: PtrMut, events: PtrMut) {
        (self.resend_locally)(server_events, events);
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
        ctx: &mut ServerSendCtx,
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
        ctx: &mut ClientReceiveCtx,
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

/// Signature of server event sending functions.
type SendFn =
    unsafe fn(&ServerEventData, &mut ServerSendCtx, &Ptr, &mut RepliconServer, &ConnectedClients);

/// Signature of server event receiving functions.
type ReceiveFn = unsafe fn(
    &ServerEventData,
    &mut ClientReceiveCtx,
    PtrMut,
    PtrMut,
    &mut RepliconClient,
    RepliconTick,
);

/// Signature of server event resending functions.
type ResendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of server event reset functions.
type ResetFn = unsafe fn(PtrMut);

/// Signature of server event serialization functions.
pub type SerializeFn<E> = fn(&mut ServerSendCtx, &E, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of server event deserialization functions.
pub type DeserializeFn<E> = fn(&mut ClientReceiveCtx, &mut Cursor<&[u8]>) -> bincode::Result<E>;

/// Typed version of [`ServerEvent::send`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<ToClients<E>>`]
/// and `event_data` was created for `E`.
unsafe fn send<E: Event>(
    event_data: &ServerEventData,
    ctx: &mut ServerSendCtx,
    server_events: &Ptr,
    server: &mut RepliconServer,
    connected_clients: &ConnectedClients,
) {
    let events: &Events<ToClients<E>> = server_events.deref();
    // For server events we don't track read events because
    // all of them will always be drained in the local resending system.
    for ToClients { event, mode } in events.get_reader().read(events) {
        trace!("sending event `{}` with `{mode:?}`", any::type_name::<E>());
        send_with(event_data, ctx, event, mode, server, connected_clients)
            .expect("server event should be serializable");
    }
}

/// Typed version of [`ServerEvent::receive`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<E>`], `queue` is [`ServerEventQueue<E>`]
/// and `event_data` was created for `E`.
unsafe fn receive<E: Event>(
    event_data: &ServerEventData,
    ctx: &mut ClientReceiveCtx,
    events: PtrMut,
    queue: PtrMut,
    client: &mut RepliconClient,
    init_tick: RepliconTick,
) {
    let events: &mut Events<E> = events.deref_mut();
    let queue: &mut ServerEventQueue<E> = queue.deref_mut();

    while let Some((tick, event)) = queue.pop_if_le(init_tick) {
        trace!(
            "applying event `{}` from queue with `{tick:?}`",
            any::type_name::<E>()
        );
        events.send(event);
    }

    for message in client.receive(event_data.channel_id) {
        let mut cursor = Cursor::new(&*message);
        let (tick, event) = deserialize_with(ctx, event_data, &mut cursor)
            .expect("server should send valid events");

        if tick <= init_tick {
            trace!("applying event `{}` with `{tick:?}`", any::type_name::<E>());
            events.send(event);
        } else {
            trace!("queuing event `{}` with `{tick:?}`", any::type_name::<E>());
            queue.insert(tick, event);
        }
    }
}

/// Typed version of [`ServerEvent::resend_locally`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<E>`] and `server_events` is [`Events<ToClients<E>>`].
unsafe fn resend_locally<E: Event>(server_events: PtrMut, events: PtrMut) {
    let server_events: &mut Events<ToClients<E>> = server_events.deref_mut();
    let events: &mut Events<E> = events.deref_mut();
    for ToClients { event, mode } in server_events.drain() {
        match mode {
            SendMode::Broadcast => {
                events.send(event);
            }
            SendMode::BroadcastExcept(client_id) => {
                if client_id != ClientId::SERVER {
                    events.send(event);
                }
            }
            SendMode::Direct(client_id) => {
                if client_id == ClientId::SERVER {
                    events.send(event);
                }
            }
        }
    }
}

/// Typed version of [`ServerEvent::reset`].
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

/// Sends event `E` based on a mode.
///
/// # Safety
///
/// The caller must ensure that `event_data` was created for `E`.
unsafe fn send_with<E: Event>(
    event_data: &ServerEventData,
    ctx: &mut ServerSendCtx,
    event: &E,
    mode: &SendMode,
    server: &mut RepliconServer,
    connected_clients: &ConnectedClients,
) -> bincode::Result<()> {
    match *mode {
        SendMode::Broadcast => {
            let mut previous_message = None;
            for client in connected_clients.iter() {
                let message = serialize_with(event_data, ctx, event, client, previous_message)?;
                server.send(client.id(), event_data.channel_id, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::BroadcastExcept(client_id) => {
            let mut previous_message = None;
            for client in connected_clients.iter() {
                if client.id() == client_id {
                    continue;
                }
                let message = serialize_with(event_data, ctx, event, client, previous_message)?;
                server.send(client.id(), event_data.channel_id, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::Direct(client_id) => {
            if client_id != ClientId::SERVER {
                if let Some(client) = connected_clients.get_client(client_id) {
                    let message = serialize_with(event_data, ctx, event, client, None)?;
                    server.send(client.id(), event_data.channel_id, message.bytes);
                }
            }
        }
    }

    Ok(())
}

/// Helper for serializing a server event.
///
/// Will prepend the client's change tick to the injected message.
/// Optimized to avoid reallocations when consecutive clients have the same change tick.
///
/// # Safety
///
/// The caller must ensure that `event_data` was created for `E`.
unsafe fn serialize_with<E: Event>(
    event_data: &ServerEventData,
    ctx: &mut ServerSendCtx,
    event: &E,
    client: &ConnectedClient,
    previous_message: Option<SerializedMessage>,
) -> bincode::Result<SerializedMessage> {
    if let Some(previous_message) = previous_message {
        if previous_message.tick == client.change_tick() {
            return Ok(previous_message);
        }

        let tick_size = DefaultOptions::new().serialized_size(&client.change_tick())? as usize;
        let mut bytes = Vec::with_capacity(tick_size + previous_message.event_bytes().len());
        DefaultOptions::new().serialize_into(&mut bytes, &client.change_tick())?;
        bytes.extend_from_slice(previous_message.event_bytes());
        let message = SerializedMessage {
            tick: client.change_tick(),
            tick_size,
            bytes: bytes.into(),
        };

        Ok(message)
    } else {
        let mut cursor = Cursor::new(Vec::new());
        DefaultOptions::new().serialize_into(&mut cursor, &client.change_tick())?;
        let tick_size = cursor.get_ref().len();
        event_data.serialize(ctx, event, &mut cursor)?;
        let message = SerializedMessage {
            tick: client.change_tick(),
            tick_size,
            bytes: cursor.into_inner().into(),
        };

        Ok(message)
    }
}

/// Deserializes event change tick first and then calls the specified deserialization function to get the event itself.
///
/// # Safety
///
/// The caller must ensure that `event_data` was created for `E`.
unsafe fn deserialize_with<E: Event>(
    ctx: &mut ClientReceiveCtx,
    event_data: &ServerEventData,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<(RepliconTick, E)> {
    let tick = DefaultOptions::new().deserialize_from(&mut *cursor)?;
    let event = event_data.deserialize(ctx, cursor)?;

    Ok((tick, event))
}

/// Default event serialization function.
pub fn default_serialize<E: Event + Serialize>(
    _ctx: &mut ServerSendCtx,
    event: &E,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(cursor, event)
}

/// Default event deserialization function.
pub fn default_deserialize<E: Event + DeserializeOwned>(
    _ctx: &mut ClientReceiveCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<E> {
    DefaultOptions::new().deserialize_from(cursor)
}

/// Default event deserialization function.
pub fn default_deserialize_mapped<E: Event + DeserializeOwned + MapEntities>(
    ctx: &mut ClientReceiveCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<E> {
    let mut event: E = DefaultOptions::new().deserialize_from(cursor)?;
    event.map_entities(ctx);

    Ok(event)
}

/// An event that will be send to client(s).
#[derive(Clone, Copy, Debug, Event)]
pub struct ToClients<T> {
    pub mode: SendMode,
    pub event: T,
}

/// Type of server message sending.
#[derive(Clone, Copy, Debug)]
pub enum SendMode {
    Broadcast,
    BroadcastExcept(ClientId),
    Direct(ClientId),
}

/// Stores all received events from server that arrived earlier then replication message with their tick.
///
/// Stores data sorted by ticks and maintains order of arrival.
/// Needed to ensure that when an event is triggered, all the data that it affects or references already exists.
#[derive(Resource)]
struct ServerEventQueue<T>(ListOrderedMultimap<RepliconTick, T>);

impl<T> ServerEventQueue<T> {
    /// Inserts a new event.
    ///
    /// The event will be queued until [`RepliconTick`] is bigger or equal to the tick specified here.
    fn insert(&mut self, tick: RepliconTick, event: T) {
        self.0.insert(tick, event);
    }

    /// Pops the next event that is at least as old as the specified replicon tick.
    fn pop_if_le(&mut self, init_tick: RepliconTick) -> Option<(RepliconTick, T)> {
        let (tick, _) = self.0.front()?;
        if *tick > init_tick {
            return None;
        }
        self.0
            .pop_front()
            .map(|(tick, event)| (tick.into_owned(), event))
    }
}

impl<T> Default for ServerEventQueue<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

/// Cached message for use in [`serialize_with`].
struct SerializedMessage {
    tick: RepliconTick,
    tick_size: usize,
    bytes: Bytes,
}

impl SerializedMessage {
    fn event_bytes(&self) -> &[u8] {
        &self.bytes[self.tick_size..]
    }
}
