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

use super::{ClientEventReader, DeserializeFn, FromClient, SerializeFn};
use crate::{
    client::replicon_client::RepliconClient,
    core::{
        ctx::{ClientSendCtx, ServerReceiveCtx},
        ClientId,
    },
    server::replicon_server::RepliconServer,
};

/// Type-erased functions and metadata for a registered client event.
///
/// Needed so events of different types can be processed together.
pub(super) struct ClientEventData {
    type_id: TypeId,
    type_name: &'static str,

    /// ID of [`Events<E>`] resource.
    events_id: ComponentId,

    /// ID of [`ClientEventReader<E>`] resource.
    reader_id: ComponentId,

    /// ID of [`Events<ToClients<E>>`] resource.
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

impl ClientEventData {
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

    pub(super) fn events_id(&self) -> ComponentId {
        self.events_id
    }

    pub(super) fn reader_id(&self) -> ComponentId {
        self.reader_id
    }

    pub(super) fn client_events_id(&self) -> ComponentId {
        self.client_events_id
    }

    /// Sends an event to the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `reader` is [`ClientEventReader<E>`]
    /// and this instance was created for `E`.
    pub(super) unsafe fn send(
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
    pub(super) unsafe fn receive(
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
    pub(super) unsafe fn resend_locally(&self, client_events: PtrMut, events: PtrMut) {
        (self.resend_locally)(client_events, events);
    }

    /// Drains all events.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`]
    /// and this instance was created for `E`.
    pub(super) unsafe fn reset(&self, events: PtrMut) {
        (self.reset)(events);
    }

    /// Serializes an event into a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E`.
    pub(super) unsafe fn serialize<E: Event>(
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
    pub(super) unsafe fn deserialize<E: Event>(
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

/// Signature of client event sending functions.
type SendFn = unsafe fn(&ClientEventData, &mut ClientSendCtx, &Ptr, PtrMut, &mut RepliconClient);

/// Signature of client event receiving functions.
type ReceiveFn = unsafe fn(&ClientEventData, &mut ServerReceiveCtx, PtrMut, &mut RepliconServer);

/// Signature of client event resending functions.
type ResendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of client event reset functions.
type ResetFn = unsafe fn(PtrMut);

/// Typed version of [`ClientEvent::send`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<FromClient<E>>`], `reader` is [`ClientEventReader<E>`],
/// and `event_data` was created for `E`.
unsafe fn send<E: Event>(
    event_data: &ClientEventData,
    ctx: &mut ClientSendCtx,
    events: &Ptr,
    reader: PtrMut,
    client: &mut RepliconClient,
) {
    let reader: &mut ClientEventReader<E> = reader.deref_mut();
    for event in reader.read(events.deref()) {
        let mut cursor = Default::default();
        event_data
            .serialize::<E>(ctx, event, &mut cursor)
            .expect("client event should be serializable");

        trace!("sending event `{}`", any::type_name::<E>());
        client.send(event_data.channel_id, cursor.into_inner());
    }
}

/// Typed version of [`ClientEvent::receive`].
///
/// # Safety
///
/// The caller must ensure that `events` is [`Events<E>`]
/// and `event_data` was created for `E`.
unsafe fn receive<E: Event>(
    event_data: &ClientEventData,
    ctx: &mut ServerReceiveCtx,
    events: PtrMut,
    server: &mut RepliconServer,
) {
    let events: &mut Events<FromClient<E>> = events.deref_mut();
    for (client_id, message) in server.receive(event_data.channel_id) {
        let mut cursor = Cursor::new(&*message);
        match event_data.deserialize::<E>(ctx, &mut cursor) {
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
unsafe fn resend_locally<E: Event>(client_events: PtrMut, events: PtrMut) {
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
