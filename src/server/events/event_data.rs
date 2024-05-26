use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{
    ecs::component::{ComponentId, Components},
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;

use super::{DeserializeFn, SendMode, SerializeFn, ServerEventQueue, ToClients};
use crate::{
    client::replicon_client::RepliconClient,
    core::{
        ctx::{ClientReceiveCtx, ServerSendCtx},
        replicon_tick::RepliconTick,
        ClientId,
    },
    server::{
        connected_clients::{ConnectedClient, ConnectedClients},
        replicon_server::RepliconServer,
    },
};

/// Type-erased functions and metadata for a registered server event.
///
/// Needed so events of different types can be processed together.
pub(super) struct ServerEventData {
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
    pub(super) fn new<E: Event>(
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

    pub(super) fn events_id(&self) -> ComponentId {
        self.events_id
    }

    pub(super) fn server_events_id(&self) -> ComponentId {
        self.server_events_id
    }

    pub(super) fn queue_id(&self) -> ComponentId {
        self.queue_id
    }

    /// Sends an event to client(s).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<ToClients<E>>`]
    /// and this instance was created for `E`.
    pub(super) unsafe fn send(
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
    pub(super) unsafe fn receive(
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
    pub(super) unsafe fn resend_locally(&self, server_events: PtrMut, events: PtrMut) {
        (self.resend_locally)(server_events, events);
    }

    /// Clears queued events.
    ///
    /// We clear events while waiting for a connection to ensure clean reconnects.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`Events<E>`]
    /// and this instance was created for `E`.
    pub(super) unsafe fn reset(&self, queue: PtrMut) {
        (self.reset)(queue);
    }

    /// Serializes an event into a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    pub(super) unsafe fn serialize<E: Event>(
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
    pub(super) unsafe fn deserialize<E: Event>(
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
/// The caller must ensure that `queue` is [`Events<E>`].
unsafe fn reset<E: Event>(queue: PtrMut) {
    let queue: &mut ServerEventQueue<E> = queue.deref_mut();
    if !queue.0.is_empty() {
        warn!(
            "discarding {} queued server events due to a disconnect",
            queue.0.values_len()
        );
    }
    queue.0.clear();
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
        if previous_message.tick == client.init_tick() {
            return Ok(previous_message);
        }

        let tick_size = DefaultOptions::new().serialized_size(&client.init_tick())? as usize;
        let mut bytes = Vec::with_capacity(tick_size + previous_message.event_bytes().len());
        DefaultOptions::new().serialize_into(&mut bytes, &client.init_tick())?;
        bytes.extend_from_slice(previous_message.event_bytes());
        let message = SerializedMessage {
            tick: client.init_tick(),
            tick_size,
            bytes: bytes.into(),
        };

        Ok(message)
    } else {
        let mut cursor = Cursor::new(Vec::new());
        DefaultOptions::new().serialize_into(&mut cursor, &client.init_tick())?;
        let tick_size = cursor.get_ref().len();
        event_data.serialize(ctx, event, &mut cursor)?;
        let message = SerializedMessage {
            tick: client.init_tick(),
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
