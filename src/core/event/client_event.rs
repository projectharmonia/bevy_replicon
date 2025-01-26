use std::{any, io::Cursor};

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities, event::EventCursor},
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::{
    ctx::{ClientSendCtx, ServerReceiveCtx},
    event_fns::{EventDeserializeFn, EventFns, EventSerializeFn, UntypedEventFns},
    event_registry::EventRegistry,
};
use crate::core::{
    channels::{RepliconChannel, RepliconChannels},
    replicon_client::RepliconClient,
    replicon_server::RepliconServer,
    ClientId,
};

/// An extension trait for [`App`] for creating client events.
pub trait ClientEventAppExt {
    /// Registers [`FromClient<E>`] and `E` events.
    ///
    /// The API matches [`ClientTriggerAppExt::add_client_trigger`](super::client_trigger::ClientTriggerAppExt::add_client_trigger):
    /// [`FromClient<E>`] will be emitted on the server after sending `E` event on client.
    /// When [`RepliconClient`] is inactive, the event will be drained right after sending and re-emitted
    /// locally as [`FromClient<E>`] with [`ClientId::SERVER`](crate::core::ClientId::SERVER).
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

    See also [`ClientTriggerAppExt::add_client_trigger`](super::client_trigger::ClientTriggerAppExt::add_client_trigger).

    # Examples

    Register an event with [`Box<dyn PartialReflect>`]:

    ```
    use std::io::Cursor;

    use bevy::{
        prelude::*,
        reflect::serde::{ReflectSerializer, ReflectDeserializer},
    };
    use bevy_replicon::{
        core::event::ctx::{ClientSendCtx, ServerReceiveCtx},
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
        message: &mut Vec<u8>,
    ) -> bincode::Result<()> {
        let serializer = ReflectSerializer::new(&*event.0, ctx.registry);
        DefaultOptions::new().serialize_into(message, &serializer)
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
    struct ReflectEvent(Box<dyn PartialReflect>);
    ```
    */
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: EventSerializeFn<ClientSendCtx, E>,
        deserialize: EventDeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        serialize: EventSerializeFn<ClientSendCtx, E>,
        deserialize: EventDeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        debug!("registering event `{}`", any::type_name::<E>());

        let event_fns = EventFns::new(serialize, deserialize);
        let event = ClientEvent::new(self, channel, event_fns);
        let mut event_registry = self.world_mut().resource_mut::<EventRegistry>();
        event_registry.register_client_event(event);

        self
    }
}

/// Type-erased functions and metadata for a registered client event.
///
/// Needed so events of different types can be processed together.
pub(crate) struct ClientEvent {
    /// ID of [`Events<E>`] resource.
    events_id: ComponentId,

    /// ID of [`ClientEventReader<E>`] resource.
    reader_id: ComponentId,

    /// ID of [`Events<FromClient<E>>`] resource.
    client_events_id: ComponentId,

    /// Used channel.
    channel_id: u8,

    send: SendFn,
    receive: ReceiveFn,
    resend_locally: ResendLocallyFn,
    reset: ResetFn,
    event_fns: UntypedEventFns,
}

impl ClientEvent {
    pub(super) fn new<E: Event, I: 'static>(
        app: &mut App,
        channel: impl Into<RepliconChannel>,
        event_fns: EventFns<ClientSendCtx, ServerReceiveCtx, E, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel);

        app.add_event::<E>()
            .add_event::<FromClient<E>>()
            .init_resource::<ClientEventReader<E>>();

        let events_id = app.world().resource_id::<Events<E>>().unwrap();
        let client_events_id = app.world().resource_id::<Events<FromClient<E>>>().unwrap();
        let reader_id = app.world().resource_id::<ClientEventReader<E>>().unwrap();

        Self {
            events_id,
            reader_id,
            client_events_id,
            channel_id,
            send: Self::send_typed::<E, I>,
            receive: Self::receive_typed::<E, I>,
            resend_locally: Self::resend_locally_typed::<E>,
            reset: Self::reset_typed::<E>,
            event_fns: event_fns.into(),
        }
    }

    pub(crate) fn events_id(&self) -> ComponentId {
        self.events_id
    }

    pub(crate) fn reader_id(&self) -> ComponentId {
        self.reader_id
    }

    pub(crate) fn client_events_id(&self) -> ComponentId {
        self.client_events_id
    }

    /// Sends an event to the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `reader` is [`ClientEventReader<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn send(
        &self,
        ctx: &mut ClientSendCtx,
        events: &Ptr,
        reader: PtrMut,
        client: &mut RepliconClient,
    ) {
        (self.send)(self, ctx, events, reader, client);
    }

    /// Typed version of [`Self::send`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `reader` is [`ClientEventReader<E>`],
    /// and this instance was created for `E` and `I`.
    unsafe fn send_typed<E: Event, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        events: &Ptr,
        reader: PtrMut,
        client: &mut RepliconClient,
    ) {
        let reader: &mut ClientEventReader<E> = reader.deref_mut();
        for event in reader.read(events.deref()) {
            let mut message = Vec::new();
            self.serialize::<E, I>(ctx, event, &mut message)
                .expect("client event should be serializable");

            debug!("sending event `{}`", any::type_name::<E>());
            client.send(self.channel_id, message);
        }
    }

    /// Receives events from a client.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ServerReceiveCtx,
        client_events: PtrMut,
        server: &mut RepliconServer,
    ) {
        (self.receive)(self, ctx, client_events, server);
    }

    /// Typed version of [`Self::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E` and `I`.
    unsafe fn receive_typed<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        client_events: PtrMut,
        server: &mut RepliconServer,
    ) {
        let client_events: &mut Events<FromClient<E>> = client_events.deref_mut();
        for (client_id, message) in server.receive(self.channel_id) {
            let mut cursor = Cursor::new(&*message);
            match self.deserialize::<E, I>(ctx, &mut cursor) {
                Ok(event) => {
                    debug!(
                        "applying event `{}` from `{client_id:?}`",
                        any::type_name::<E>()
                    );
                    client_events.send(FromClient { client_id, event });
                }
                Err(e) => debug!(
                    "ignoring event `{}` from {client_id:?} that failed to deserialize: {e}",
                    any::type_name::<E>()
                ),
            }
        }
    }

    /// Drains events `E` and re-emits them as [`FromClient<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn resend_locally(&self, client_events: PtrMut, events: PtrMut) {
        (self.resend_locally)(client_events, events);
    }

    /// Typed version of [`ClientEvent::resend_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`] and `server_events` is [`Events<ToClients<E>>`].
    unsafe fn resend_locally_typed<E: Event>(server_events: PtrMut, events: PtrMut) {
        let client_events: &mut Events<FromClient<E>> = server_events.deref_mut();
        let events: &mut Events<E> = events.deref_mut();
        if !events.is_empty() {
            debug!(
                "resending {} event(s) `{}` locally",
                events.len(),
                any::type_name::<E>()
            );
            client_events.send_batch(events.drain().map(|event| FromClient {
                client_id: ClientId::SERVER,
                event,
            }));
        }
    }

    /// Drains all events.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn reset(&self, events: PtrMut) {
        (self.reset)(events);
    }

    /// Typed version of [`ClientEvent::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`].
    unsafe fn reset_typed<E: Event>(events: PtrMut) {
        let events: &mut Events<E> = events.deref_mut();
        let drained_count = events.drain().count();
        if drained_count > 0 {
            warn!("discarded {drained_count} events due to a disconnect");
        }
    }

    /// Serializes an event.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn serialize<E: 'static, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        event: &E,
        message: &mut Vec<u8>,
    ) -> bincode::Result<()> {
        self.event_fns
            .typed::<ClientSendCtx, ServerReceiveCtx, E, I>()
            .serialize(ctx, event, message)
    }

    /// Deserializes an event from a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn deserialize<E: 'static, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<E> {
        self.event_fns
            .typed::<ClientSendCtx, ServerReceiveCtx, E, I>()
            .deserialize(ctx, cursor)
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

/// Tracks read events for [`ClientEventPlugin::send`].
///
/// Unlike with server events, we don't always drain all events in [`ClientEventPlugin::resend_locally`].
#[derive(Resource, Deref, DerefMut)]
struct ClientEventReader<E: Event>(EventCursor<E>);

impl<E: Event> FromWorld for ClientEventReader<E> {
    fn from_world(world: &mut World) -> Self {
        let events = world.resource::<Events<E>>();
        Self(events.get_cursor())
    }
}

/// An event indicating that a message from client was received.
/// Emitted only on server.
#[derive(Clone, Copy, Event)]
pub struct FromClient<T> {
    pub client_id: ClientId,
    pub event: T,
}

/// Default event serialization function.
pub fn default_serialize<E: Event + Serialize>(
    _ctx: &mut ClientSendCtx,
    event: &E,
    message: &mut Vec<u8>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(message, event)
}

/// Like [`default_serialize`], but also maps entities.
pub fn default_serialize_mapped<E: Event + MapEntities + Clone + Serialize>(
    ctx: &mut ClientSendCtx,
    event: &E,
    message: &mut Vec<u8>,
) -> bincode::Result<()> {
    let mut event = event.clone();
    event.map_entities(ctx);
    DefaultOptions::new().serialize_into(message, &event)
}

/// Default event deserialization function.
pub fn default_deserialize<E: Event + DeserializeOwned>(
    _ctx: &mut ServerReceiveCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<E> {
    DefaultOptions::new().deserialize_from(cursor)
}
