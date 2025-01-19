use std::{
    any::{self, TypeId},
    collections::HashSet,
    io::{Cursor, Write},
    marker::PhantomData,
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

use super::{
    ctx::{ClientReceiveCtx, ServerSendCtx},
    event_registry::EventRegistry,
};
use crate::core::{
    channels::{RepliconChannel, RepliconChannels},
    connected_clients::ConnectedClients,
    replication::replicated_clients::{ReplicatedClient, ReplicatedClients},
    replicon_client::RepliconClient,
    replicon_server::RepliconServer,
    replicon_tick::RepliconTick,
    ClientId,
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

    Register an event with [`Box<dyn PartialReflect>`]:

    ```
    use std::io::Cursor;

    use bevy::{
        prelude::*,
        reflect::serde::{ReflectSerializer, ReflectDeserializer},
    };
    use bevy_replicon::{
        core::event::ctx::{ClientReceiveCtx, ServerSendCtx},
        prelude::*,
    };
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_server_event_with(
        ChannelKind::Ordered,
        serialize_reflect,
        deserialize_reflect,
    );

    fn serialize_reflect(
        ctx: &mut ServerSendCtx,
        event: &ReflectEvent,
        message: &mut Vec<u8>,
    ) -> bincode::Result<()> {
        let serializer = ReflectSerializer::new(&*event.0, ctx.registry);
        DefaultOptions::new().serialize_into(message, &serializer)
    }

    fn deserialize_reflect(
        ctx: &mut ClientReceiveCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<ReflectEvent> {
        let mut deserializer = bincode::Deserializer::with_reader(cursor, DefaultOptions::new());
        let reflect = ReflectDeserializer ::new(ctx.registry).deserialize(&mut deserializer)?;
        Ok(ReflectEvent(reflect))
    }

    #[derive(Event)]
    struct ReflectEvent(Box<dyn PartialReflect>);
    ```
    */
    fn add_server_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> &mut Self;

    /// Marks the event `E` as an independent event.
    ///
    /// By default, all server events are buffered on server until server tick
    /// and queued on client until all insertions, removals and despawns
    /// (value mutations doesn't count) are replicated for the tick on which the
    /// event was triggered. This is necessary to ensure that the executed logic
    /// during the event does not affect components or entities that the client
    /// has not yet received.
    ///
    /// However, if you know your event doesn't rely on that, you can mark it
    /// as independent to always emit it immediately. For example, a chat
    /// message event - which does not hold references to any entities - may be
    /// marked as independent.
    ///
    /// <div class="warning">
    ///
    /// Use this method very carefully; it can lead to logic errors that are
    /// very difficult to debug!
    ///
    /// </div>
    fn make_independent<E: Event>(&mut self) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: SerializeFn<E>,
        deserialize: DeserializeFn<E>,
    ) -> &mut Self {
        debug!("registering event `{}`", any::type_name::<E>());

        self.add_event::<E>()
            .add_event::<ToClients<E>>()
            .init_resource::<ServerEventQueue<E>>();

        let channel_id = self
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_server_channel(channel);

        self.world_mut()
            .resource_scope(|world, mut event_registry: Mut<EventRegistry>| {
                event_registry.register_server_event(ServerEvent::new(
                    world.components(),
                    channel_id,
                    serialize,
                    deserialize,
                ));
            });

        self
    }

    fn make_independent<E: Event>(&mut self) -> &mut Self {
        self.world_mut()
            .resource_scope(|world, mut event_registry: Mut<EventRegistry>| {
                let events_id = world
                    .components()
                    .resource_id::<Events<E>>()
                    .unwrap_or_else(|| {
                        panic!(
                            "event `{}` should be previously registered",
                            any::type_name::<E>()
                        )
                    });
                event_registry.make_independent(events_id);
            });

        self
    }
}

/// Type-erased functions and metadata for a registered server event.
///
/// Needed so events of different types can be processed together.
pub(crate) struct ServerEvent {
    event_id: TypeId,
    event_name: &'static str,

    /// Whether this event depends on replication or not.
    ///
    /// Events like a chat message event do not have to wait for replication to
    /// be synced. If set to `true`, the event will always be applied
    /// immediately.
    independent: bool,

    /// ID of [`Events<E>`].
    events_id: ComponentId,

    /// ID of [`Events<ToClients<E>>`].
    server_events_id: ComponentId,

    /// ID of [`ServerEventQueue<T>`].
    queue_id: ComponentId,

    /// Used channel.
    channel_id: u8,

    send_or_buffer: SendOrBufferFn,
    receive: ReceiveFn,
    resend_locally: ResendLocallyFn,
    reset: ResetFn,
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
}

impl ServerEvent {
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

        Self {
            event_id: TypeId::of::<E>(),
            event_name: any::type_name::<E>(),
            independent: false,
            events_id,
            server_events_id,
            queue_id,
            channel_id,
            send_or_buffer: Self::send_or_buffer_typed::<E>,
            receive: Self::receive_typed::<E>,
            resend_locally: Self::resend_locally_typed::<E>,
            reset: Self::reset_typed::<E>,
            // SAFETY: these functions won't be called until the type is restored.
            serialize: unsafe { mem::transmute::<SerializeFn<E>, unsafe fn()>(serialize) },
            deserialize: unsafe { mem::transmute::<DeserializeFn<E>, unsafe fn()>(deserialize) },
        }
    }

    pub(crate) fn events_id(&self) -> ComponentId {
        self.events_id
    }

    pub(crate) fn server_events_id(&self) -> ComponentId {
        self.server_events_id
    }

    pub(crate) fn queue_id(&self) -> ComponentId {
        self.queue_id
    }

    pub(super) fn is_independent(&self) -> bool {
        self.independent
    }

    pub(super) fn make_independent(&mut self) {
        self.independent = true
    }

    /// Sends an event to client(s).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `server_events` is [`Events<ToClients<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn send_or_buffer(
        &self,
        ctx: &mut ServerSendCtx,
        server_events: &Ptr,
        server: &mut RepliconServer,
        connected_clients: &ConnectedClients,
        buffered_events: &mut BufferedServerEvents,
    ) {
        (self.send_or_buffer)(
            self,
            ctx,
            server_events,
            server,
            connected_clients,
            buffered_events,
        );
    }

    /// Typed version of [`Self::send_or_buffer`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `server_events` is [`Events<ToClients<E>>`]
    /// and this instance was created for `E`.
    unsafe fn send_or_buffer_typed<E: Event>(
        &self,
        ctx: &mut ServerSendCtx,
        server_events: &Ptr,
        server: &mut RepliconServer,
        connected_clients: &ConnectedClients,
        buffered_events: &mut BufferedServerEvents,
    ) {
        self.check_type::<E>();

        let events: &Events<ToClients<E>> = server_events.deref();
        // For server events we don't track read events because
        // all of them will always be drained in the local resending system.
        for ToClients { event, mode } in events.get_cursor().read(events) {
            debug!("sending event `{}` with `{mode:?}`", any::type_name::<E>());

            if self.is_independent() {
                self.send_independent_event(ctx, event, mode, server, connected_clients)
                    .expect("independent server event should be serializable");
            } else {
                self.buffer_event(ctx, event, *mode, buffered_events)
                    .expect("server event should be serializable");
            }
        }
    }

    /// Sends independent event `E` based on a mode.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `event_data` was created for `E`.
    ///
    /// For regular events see [`Self::buffer_event`].
    unsafe fn send_independent_event<E: Event>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
        mode: &SendMode,
        server: &mut RepliconServer,
        connected_clients: &ConnectedClients,
    ) -> bincode::Result<()> {
        let mut message = Vec::new();
        self.serialize(ctx, event, &mut message)?;
        let message: Bytes = message.into();

        match *mode {
            SendMode::Broadcast => {
                for client in connected_clients.iter() {
                    server.send(client.id(), self.channel_id, message.clone());
                }
            }
            SendMode::BroadcastExcept(id) => {
                for client in connected_clients.iter() {
                    if client.id() != id {
                        server.send(client.id(), self.channel_id, message.clone());
                    }
                }
            }
            SendMode::Direct(client_id) => {
                if client_id != ClientId::SERVER {
                    server.send(client_id, self.channel_id, message.clone());
                }
            }
        }

        Ok(())
    }

    /// Buffers event `E` based on a mode.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    ///
    /// For independent events see [`Self::send_independent_event`].
    unsafe fn buffer_event<E: Event>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
        mode: SendMode,
        buffered_events: &mut BufferedServerEvents,
    ) -> bincode::Result<()> {
        let message = self.serialize_with_padding(ctx, event)?;
        buffered_events.insert(mode, self.channel_id, message);
        Ok(())
    }

    /// Helper for serializing a server event.
    ///
    /// Will prepend padding bytes for where the update tick will be inserted to the injected message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    unsafe fn serialize_with_padding<E: Event>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
    ) -> bincode::Result<SerializedMessage> {
        let mut message = Vec::new();
        let padding = [0; mem::size_of::<RepliconTick>()];
        message.write_all(&padding)?;
        self.serialize(ctx, event, &mut message)?;
        let message = SerializedMessage::Raw(message);

        Ok(message)
    }

    /// Receives events from the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `queue` is [`ServerEventQueue<E>`],
    /// and this instance was created for `E`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ClientReceiveCtx,
        events: PtrMut,
        queue: PtrMut,
        client: &mut RepliconClient,
        update_tick: RepliconTick,
    ) {
        (self.receive)(self, ctx, events, queue, client, update_tick);
    }

    /// Typed version of [`ServerEvent::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `queue` is [`ServerEventQueue<E>`]
    /// and this instance was created for `E`.
    unsafe fn receive_typed<E: Event>(
        &self,
        ctx: &mut ClientReceiveCtx,
        events: PtrMut,
        queue: PtrMut,
        client: &mut RepliconClient,
        update_tick: RepliconTick,
    ) {
        self.check_type::<E>();

        let events: &mut Events<E> = events.deref_mut();
        let queue: &mut ServerEventQueue<E> = queue.deref_mut();

        while let Some((tick, message)) = queue.pop_if_le(update_tick) {
            let mut cursor = Cursor::new(&*message);
            match self.deserialize(ctx, &mut cursor) {
                Ok(event) => {
                    debug!(
                        "applying event `{}` from queue with `{tick:?}`",
                        any::type_name::<E>()
                    );
                    events.send(event);
                }
                Err(e) => error!(
                "ignoring event `{}` from queue with `{tick:?}` that failed to deserialize: {e}",
                any::type_name::<E>()
            ),
            }
        }

        for message in client.receive(self.channel_id) {
            let mut cursor = Cursor::new(&*message);
            if !self.is_independent() {
                let tick = match bincode::deserialize_from(&mut cursor) {
                    Ok(tick) => tick,
                    Err(e) => {
                        error!(
                            "ignoring event `{}` because it's tick failed to deserialize: {e}",
                            any::type_name::<E>()
                        );
                        continue;
                    }
                };
                if tick > update_tick {
                    debug!("queuing event `{}` with `{tick:?}`", any::type_name::<E>());
                    queue.insert(tick, message.slice(cursor.position() as usize..));
                    continue;
                } else {
                    debug!(
                        "receiving event `{}` with `{tick:?}`",
                        any::type_name::<E>()
                    );
                }
            }

            match self.deserialize(ctx, &mut cursor) {
                Ok(event) => {
                    debug!("applying event `{}`", any::type_name::<E>());
                    events.send(event);
                }
                Err(e) => error!(
                    "ignoring event `{}` that failed to deserialize: {e}",
                    any::type_name::<E>()
                ),
            }
        }
    }

    /// Drains events [`ToClients<E>`] and re-emits them as `E` if the server is in the list of the event recipients.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `server_events` is [`Events<ToClients<E>>`],
    /// and this instance was created for `E`.
    pub(crate) unsafe fn resend_locally(&self, server_events: PtrMut, events: PtrMut) {
        (self.resend_locally)(server_events, events);
    }

    /// Typed version of [`Self::resend_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`] and `server_events` is [`Events<ToClients<E>>`].
    unsafe fn resend_locally_typed<E: Event>(server_events: PtrMut, events: PtrMut) {
        let server_events: &mut Events<ToClients<E>> = server_events.deref_mut();
        let events: &mut Events<E> = events.deref_mut();
        for ToClients { event, mode } in server_events.drain() {
            debug!("resending event `{}` locally", any::type_name::<E>());
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

    /// Clears queued events.
    ///
    /// We clear events while waiting for a connection to ensure clean reconnects.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`Events<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn reset(&self, queue: PtrMut) {
        (self.reset)(queue);
    }

    /// Typed version of [`Self::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`Events<E>`].
    unsafe fn reset_typed<E: Event>(queue: PtrMut) {
        let queue: &mut ServerEventQueue<E> = queue.deref_mut();
        if !queue.is_empty() {
            warn!(
                "discarding {} queued events due to a disconnect",
                queue.values_len()
            );
        }
        queue.clear();
    }

    /// Serializes an event.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    unsafe fn serialize<E: Event>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
        message: &mut Vec<u8>,
    ) -> bincode::Result<()> {
        let serialize: SerializeFn<E> = std::mem::transmute(self.serialize);
        (serialize)(ctx, event, message)
    }

    /// Deserializes an event from a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    unsafe fn deserialize<E: Event>(
        &self,
        ctx: &mut ClientReceiveCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<E> {
        let deserialize: DeserializeFn<E> = std::mem::transmute(self.deserialize);
        let event = (deserialize)(ctx, cursor);
        if ctx.invalid_entities.is_empty() {
            event
        } else {
            let message = format!(
                "unable to map entities `{:?}` from server, \
                make sure that the event references visible entities for the client",
                ctx.invalid_entities,
            );
            ctx.invalid_entities.clear();
            Err(bincode::ErrorKind::Custom(message).into())
        }
    }

    fn check_type<C: Event>(&self) {
        debug_assert_eq!(
            self.event_id,
            TypeId::of::<C>(),
            "trying to call event functions with `{}`, but they were created with `{}`",
            any::type_name::<C>(),
            self.event_name,
        );
    }
}

/// Signature of server event serialization functions.
pub type SerializeFn<E> = fn(&mut ServerSendCtx, &E, &mut Vec<u8>) -> bincode::Result<()>;

/// Signature of server event deserialization functions.
pub type DeserializeFn<E> = fn(&mut ClientReceiveCtx, &mut Cursor<&[u8]>) -> bincode::Result<E>;

/// Signature of server event sending functions.
type SendOrBufferFn = unsafe fn(
    &ServerEvent,
    &mut ServerSendCtx,
    &Ptr,
    &mut RepliconServer,
    &ConnectedClients,
    &mut BufferedServerEvents,
);

/// Signature of server event receiving functions.
type ReceiveFn = unsafe fn(
    &ServerEvent,
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

/// Cached message for use in [`BufferedServerEvents`].
enum SerializedMessage {
    /// A message without serialized tick.
    ///
    /// `padding | message`
    ///
    /// The padding length equals to serialized bytes of [`RepliconTick`]. It should be overwritten before sending
    /// to clients.
    Raw(Vec<u8>),
    /// A message with serialized tick.
    ///
    /// `tick | message`
    Resolved { tick: RepliconTick, bytes: Bytes },
}

impl SerializedMessage {
    /// Optimized to avoid reallocations when clients have the same update tick as other clients receiving the
    /// same message.
    fn get_bytes(&mut self, update_tick: RepliconTick) -> bincode::Result<Bytes> {
        match self {
            // Resolve the raw value into a message with serialized tick.
            Self::Raw(raw) => {
                let mut bytes = mem::take(raw);
                bincode::serialize_into(
                    &mut bytes[..mem::size_of::<RepliconTick>()],
                    &update_tick,
                )?;
                let bytes = Bytes::from(bytes);
                *self = Self::Resolved {
                    tick: update_tick,
                    bytes: bytes.clone(),
                };
                Ok(bytes)
            }
            // Get the already-resolved value or reserialize with a different tick.
            Self::Resolved { tick, bytes } => {
                if *tick == update_tick {
                    return Ok(bytes.clone());
                }

                let mut new_bytes = Vec::with_capacity(bytes.len());
                bincode::serialize_into(&mut new_bytes, &update_tick)?;
                new_bytes.extend_from_slice(&bytes[mem::size_of::<RepliconTick>()..]);
                Ok(new_bytes.into())
            }
        }
    }
}

struct BufferedServerEvent {
    mode: SendMode,
    channel: u8,
    message: SerializedMessage,
}

impl BufferedServerEvent {
    fn send(
        &mut self,
        server: &mut RepliconServer,
        client: &ReplicatedClient,
    ) -> bincode::Result<()> {
        let message = self.message.get_bytes(client.update_tick())?;
        server.send(client.id(), self.channel, message);
        Ok(())
    }
}

#[derive(Default)]
struct BufferedServerEventSet {
    events: Vec<BufferedServerEvent>,
    /// Client ids excluded from receiving events in this set because they connected after the events were sent.
    excluded: HashSet<ClientId>,
}

impl BufferedServerEventSet {
    fn clear(&mut self) {
        self.events.clear();
        self.excluded.clear();
    }
}

/// Caches synchronization-dependent server events until they can be sent with an accurate update tick.
///
/// This exists because replication does not scan the world every tick. If a server event is sent in the same
/// tick as a spawn and the event references that spawn, then the server event's update tick needs to be synchronized
/// with that spawn on the client. We buffer the event until the spawn can be detected.
#[derive(Resource, Default)]
pub(crate) struct BufferedServerEvents {
    buffer: Vec<BufferedServerEventSet>,

    /// Caches unused sets to avoid reallocations when pushing into the buffer.
    ///
    /// These are cleared before insertion.
    cache: Vec<BufferedServerEventSet>,
}

impl BufferedServerEvents {
    pub(crate) fn start_tick(&mut self) {
        self.buffer.push(self.cache.pop().unwrap_or_default());
    }

    fn active_tick(&mut self) -> Option<&mut BufferedServerEventSet> {
        self.buffer.last_mut()
    }

    fn insert(&mut self, mode: SendMode, channel: u8, message: SerializedMessage) {
        let buffer = self
            .active_tick()
            .expect("`BufferedServerEvents::start_tick` should be called before buffering");

        buffer.events.push(BufferedServerEvent {
            mode,
            channel,
            message,
        });
    }

    /// Used to prevent newly-connected clients from receiving old events.
    pub(crate) fn exclude_client(&mut self, client: ClientId) {
        for set in self.buffer.iter_mut() {
            set.excluded.insert(client);
        }
    }

    pub(crate) fn send_all(
        &mut self,
        server: &mut RepliconServer,
        replicated_clients: &ReplicatedClients,
    ) -> bincode::Result<()> {
        for mut set in self.buffer.drain(..) {
            for mut event in set.events.drain(..) {
                match event.mode {
                    SendMode::Broadcast => {
                        for client in replicated_clients
                            .iter()
                            .filter(|c| !set.excluded.contains(&c.id()))
                        {
                            event.send(server, client)?;
                        }
                    }
                    SendMode::BroadcastExcept(client_id) => {
                        for client in replicated_clients
                            .iter()
                            .filter(|c| !set.excluded.contains(&c.id()))
                        {
                            if client.id() == client_id {
                                continue;
                            }
                            event.send(server, client)?;
                        }
                    }
                    SendMode::Direct(client_id) => {
                        if client_id != ClientId::SERVER && !set.excluded.contains(&client_id) {
                            if let Some(client) = replicated_clients.get_client(client_id) {
                                event.send(server, client)?;
                            }
                        }
                    }
                }
            }
            set.clear();
            self.cache.push(set);
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        for mut set in self.buffer.drain(..) {
            set.clear();
            self.cache.push(set);
        }
    }
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
#[derive(Resource, Deref, DerefMut)]
struct ServerEventQueue<E> {
    #[deref]
    list: ListOrderedMultimap<RepliconTick, Bytes>,
    marker: PhantomData<E>,
}

impl<E> ServerEventQueue<E> {
    /// Pops the next event that is at least as old as the specified replicon tick.
    fn pop_if_le(&mut self, update_tick: RepliconTick) -> Option<(RepliconTick, Bytes)> {
        let (tick, _) = self.list.front()?;
        if *tick > update_tick {
            return None;
        }
        self.list
            .pop_front()
            .map(|(tick, message)| (tick.into_owned(), message))
    }
}

impl<E> Default for ServerEventQueue<E> {
    fn default() -> Self {
        Self {
            list: Default::default(),
            marker: PhantomData,
        }
    }
}

/// Default event serialization function.
pub fn default_serialize<E: Event + Serialize>(
    _ctx: &mut ServerSendCtx,
    event: &E,
    message: &mut Vec<u8>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(message, event)
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
