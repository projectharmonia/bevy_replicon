use std::{any, io::Cursor, marker::PhantomData};

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;
use ordered_multimap::ListOrderedMultimap;
use serde::{de::DeserializeOwned, Serialize};

use super::EventMapper;
use crate::{
    client::{
        replicon_client::RepliconClient, server_entity_map::ServerEntityMap, ClientSet,
        ServerInitTick,
    },
    core::{
        common_conditions::{client_connected, has_authority, server_running},
        replicon_channels::{RepliconChannel, RepliconChannels},
        replicon_tick::RepliconTick,
        ClientId,
    },
    prelude::{ClientPlugin, ServerPlugin},
    server::{
        connected_clients::{ConnectedClient, ConnectedClients},
        replicon_server::RepliconServer,
        ServerSet,
    },
};

/// An extension trait for [`App`] for creating server events.
pub trait ServerEventAppExt {
    /// Registers event `T` that will be emitted on client after sending [`ToClients<T>`] on server.
    ///
    /// For usage example see the [corresponding section](../../index.html#from-server-to-client)
    /// in the quick start guide.
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self;

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client inside the event after receiving.
    ///
    /// Always use it for events that contain entities.
    /// For usage example see the [corresponding section](../../index.html#from-server-to-client)
    /// in the quick start guide.
    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self;

    /**
    Same as [`Self::add_server_event`], but uses specified sending and receiving systems.

    It's advised to not panic in sending system because it runs on server.

    # Examples

    Serialize an event with [`Box<dyn Reflect>`]:

    ```
    use bevy::{
        prelude::*,
        reflect::serde::{ReflectSerializer, UntypedReflectDeserializer},
    };
    use bevy_replicon::{
        client::ServerInitTick,
        network_event::server_event::{self, ServerEventChannel, ServerEventQueue},
        prelude::*,
    };
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_server_event_with::<ReflectEvent, _, _>(
        ChannelKind::Ordered,
        send_reflect,
        receive_reflect,
    );

    fn send_reflect(
        mut server: ResMut<RepliconServer>,
        mut reflect_events: EventReader<ToClients<ReflectEvent>>,
        connected_clients: Res<ConnectedClients>,
        channel: Res<ServerEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for ToClients { event, mode } in reflect_events.read() {
            server_event::send_with(&mut server, &connected_clients, *channel, *mode, |cursor| {
                let serializer = ReflectSerializer::new(&*event.0, &registry);
                DefaultOptions::new().serialize_into(cursor, &serializer)
            })
            .expect("server event should be serializable");
        }
    }

    fn receive_reflect(
        mut reflect_events: EventWriter<ReflectEvent>,
        mut client: ResMut<RepliconClient>,
        mut event_queue: ResMut<ServerEventQueue<ReflectEvent>>,
        init_tick: Res<ServerInitTick>,
        channel: Res<ServerEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for message in client.receive(*channel) {
            let (tick, event) = server_event::deserialize_with(&message, |cursor| {
                let mut deserializer =
                    bincode::Deserializer::with_reader(cursor, DefaultOptions::new());
                let reflect = UntypedReflectDeserializer::new(&registry).deserialize(&mut deserializer)?;
                Ok(ReflectEvent(reflect))
            })
            .expect("server should send valid events");

            // Event should be sent to the queue if replication message with its tick has not yet arrived.
            if tick <= **init_tick {
                reflect_events.send(event);
            } else {
                event_queue.insert(tick, event);
            }
        }
    }

    #[derive(Event)]
    struct ReflectEvent(Box<dyn Reflect>);
    ```
    */
    fn add_server_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_system: impl IntoSystemConfigs<Marker1>,
        receive_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(channel, send::<T>, receive::<T>)
    }

    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(channel, send::<T>, receive_and_map::<T>)
    }

    fn add_server_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_system: impl IntoSystemConfigs<Marker1>,
        receive_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_server_channel(channel.into());

        self.add_event::<T>()
            .init_resource::<Events<ToClients<T>>>()
            .init_resource::<ServerEventQueue<T>>()
            .insert_resource(ServerEventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                (
                    reset::<T>.in_set(ClientSet::ResetEvents),
                    (pop_from_queue::<T>, receive_system)
                        .chain()
                        .after(ClientPlugin::receive_replication)
                        .in_set(ClientSet::Receive)
                        .run_if(client_connected),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    send_system.run_if(server_running),
                    resend_locally::<T>.run_if(has_authority),
                )
                    .chain()
                    .after(ServerPlugin::send_replication)
                    .in_set(ServerSet::Send),
            );

        self
    }
}

/// Applies all queued events if their tick is less or equal to [`RepliconTick`].
fn pop_from_queue<T: Event>(
    init_tick: Res<ServerInitTick>,
    mut server_events: EventWriter<T>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
) {
    while let Some((tick, event)) = event_queue.pop_if_le(**init_tick) {
        trace!(
            "applying event `{}` from queue with `{tick:?}`",
            any::type_name::<T>()
        );
        server_events.send(event);
    }
}

fn receive<T: Event + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RepliconClient>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
    init_tick: Res<ServerInitTick>,
    channel: Res<ServerEventChannel<T>>,
) {
    for message in client.receive(*channel) {
        let (tick, event) = deserialize_with(&message, |cursor| {
            DefaultOptions::new().deserialize_from(cursor)
        })
        .expect("server should send valid events");

        if tick <= **init_tick {
            trace!("applying event `{}` with `{tick:?}`", any::type_name::<T>());
            server_events.send(event);
        } else {
            trace!("queuing event `{}` with `{tick:?}`", any::type_name::<T>());
            event_queue.insert(tick, event);
        }
    }
}

fn receive_and_map<T: Event + MapEntities + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RepliconClient>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
    init_tick: Res<ServerInitTick>,
    entity_map: Res<ServerEntityMap>,
    channel: Res<ServerEventChannel<T>>,
) {
    for message in client.receive(*channel) {
        let (tick, mut event): (_, T) = deserialize_with(&message, |cursor| {
            DefaultOptions::new().deserialize_from(cursor)
        })
        .expect("server should send valid events");

        event.map_entities(&mut EventMapper(entity_map.to_client()));
        if tick <= **init_tick {
            trace!("applying event `{}` for `{tick:?}`", any::type_name::<T>());
            server_events.send(event);
        } else {
            trace!("queuing event `{}` for `{tick:?}`", any::type_name::<T>());
            event_queue.insert(tick, event);
        }
    }
}

fn send<T: Event + Serialize>(
    mut server: ResMut<RepliconServer>,
    mut server_events: EventReader<ToClients<T>>,
    connected_clients: Res<ConnectedClients>,
    channel: Res<ServerEventChannel<T>>,
) {
    for ToClients { event, mode } in server_events.read() {
        trace!("sending event `{}` with `{mode:?}`", any::type_name::<T>());
        send_with(&mut server, &connected_clients, *channel, *mode, |cursor| {
            DefaultOptions::new().serialize_into(cursor, &event)
        })
        .expect("server event should be serializable");
    }
}

/// Transforms [`ToClients<T>`] events into `T` events to "emulate"
/// message sending for offline mode or when server is also a player.
fn resend_locally<T: Event>(
    mut server_events: ResMut<Events<ToClients<T>>>,
    mut local_events: EventWriter<T>,
) {
    for ToClients { event, mode } in server_events.drain() {
        match mode {
            SendMode::Broadcast => {
                local_events.send(event);
            }
            SendMode::BroadcastExcept(client_id) => {
                if client_id != ClientId::SERVER {
                    local_events.send(event);
                }
            }
            SendMode::Direct(client_id) => {
                if client_id == ClientId::SERVER {
                    local_events.send(event);
                }
            }
        }
    }
}

/// Clears queued events.
///
/// We clear events while waiting for a connection to ensure clean reconnects.
fn reset<T: Event>(mut event_queue: ResMut<ServerEventQueue<T>>) {
    if !event_queue.0.is_empty() {
        warn!(
            "discarding {} queued server events due to a disconnect",
            event_queue.0.values_len()
        );
    }
    event_queue.0.clear();
}

/// Helper for custom sending systems.
///
/// See also [`ServerEventAppExt::add_server_event_with`].
pub fn send_with<T>(
    server: &mut RepliconServer,
    connected_clients: &ConnectedClients,
    channel: ServerEventChannel<T>,
    mode: SendMode,
    serialize: impl Fn(&mut Cursor<Vec<u8>>) -> bincode::Result<()>,
) -> bincode::Result<()> {
    match mode {
        SendMode::Broadcast => {
            let mut previous_message = None;
            for client in connected_clients.iter() {
                let message = serialize_with(client, previous_message, &serialize)?;
                server.send(client.id(), channel, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::BroadcastExcept(client_id) => {
            let mut previous_message = None;
            for client in connected_clients.iter() {
                if client.id() == client_id {
                    continue;
                }
                let message = serialize_with(client, previous_message, &serialize)?;
                server.send(client.id(), channel, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::Direct(client_id) => {
            if client_id != ClientId::SERVER {
                if let Some(client) = connected_clients.get_client(client_id) {
                    let message = serialize_with(client, None, &serialize)?;
                    server.send(client.id(), channel, message.bytes);
                }
            }
        }
    }

    Ok(())
}

/// Helper for serializing a server event.
///
/// Will prepend the client's change tick to the injected message.
///
/// Optimized to avoid reallocations when consecutive clients have the same change tick.
fn serialize_with(
    client: &ConnectedClient,
    previous_message: Option<SerializedMessage>,
    serialize: impl Fn(&mut Cursor<Vec<u8>>) -> bincode::Result<()>,
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
        (serialize)(&mut cursor)?;
        let message = SerializedMessage {
            tick: client.change_tick(),
            tick_size,
            bytes: cursor.into_inner().into(),
        };

        Ok(message)
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

/// Deserializes event change tick first and then calls the specified deserialization function to get the event itself.
pub fn deserialize_with<T>(
    message: &[u8],
    deserialize: impl FnOnce(&mut Cursor<&[u8]>) -> bincode::Result<T>,
) -> bincode::Result<(RepliconTick, T)> {
    let mut cursor = Cursor::new(message);
    let tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    let event = (deserialize)(&mut cursor)?;

    Ok((tick, event))
}

/// Holds a server's channel ID for `T`.
#[derive(Resource)]
pub struct ServerEventChannel<T> {
    id: u8,
    marker: PhantomData<T>,
}

impl<T> ServerEventChannel<T> {
    fn new(id: u8) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
}

impl<T> Clone for ServerEventChannel<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ServerEventChannel<T> {}

impl<T> From<ServerEventChannel<T>> for u8 {
    fn from(value: ServerEventChannel<T>) -> Self {
        value.id
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
#[derive(Resource)]
pub struct ServerEventQueue<T>(ListOrderedMultimap<RepliconTick, T>);

impl<T> ServerEventQueue<T> {
    /// Inserts a new event.
    ///
    /// The event will be queued until [`RepliconTick`] is bigger or equal to the tick specified here.
    pub fn insert(&mut self, tick: RepliconTick, event: T) {
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
