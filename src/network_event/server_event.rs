use std::io::Cursor;

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bevy_renet::{
    client_connected,
    renet::{Bytes, ClientId, RenetClient, RenetServer, SendType},
};
use bincode::{DefaultOptions, Options};
use ordered_multimap::ListOrderedMultimap;
use serde::{de::DeserializeOwned, Serialize};

use super::ServerEventChannel;
use crate::{
    client::{client_mapper::ServerEntityMap, ClientSet},
    network_event::EventMapper,
    prelude::{ClientPlugin, ServerPlugin},
    replicon_core::{replicon_tick::RepliconTick, NetworkChannels},
    server::{
        client_cache::{ClientCache, ClientState},
        has_authority, ServerSet, SERVER_ID,
    },
};

/// An extension trait for [`App`] for creating server events.
pub trait ServerEventAppExt {
    /// Registers event `T` that will be emitted on client after sending [`ToClients<T>`] on server.
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self;

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client inside the event after receiving.
    ///
    /// Always use it for events that contain entities.
    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self;

    /**
    Same as [`Self::add_server_event`], but uses specified sending and receiving systems.

    It's advised to not panic in sending system because it runs on server.

    # Examples

    Serialize an event with `Box<dyn Reflect>`:

    ```
    use std::io::Cursor;

    use bevy::{
        prelude::*,
        reflect::{
            serde::{ReflectSerializer, UntypedReflectDeserializer},
            TypeRegistry,
        },
    };
    use bevy_replicon::{network_event::server_event, prelude::*};
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ReplicationPlugins));
    app.add_server_event_with::<ReflectEvent, _, _>(
        EventType::Ordered,
        sending_reflect_system,
        receiving_reflect_system,
    );

    fn sending_reflect_system(
        mut server: ResMut<RenetServer>,
        mut reflect_events: EventReader<ToClients<ReflectEvent>>,
        client_cache: Res<ClientCache>,
        channel: Res<ServerEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for ToClients { event, mode } in reflect_events.read() {
            server_event::send_with(&mut server, &client_cache, *channel, *mode, |cursor| {
                let serializer = ReflectSerializer::new(&*event.0, &registry);
                DefaultOptions::new().serialize_into(cursor, &serializer)
            })
            .expect("server event should be serializable");
        }
    }

    fn receiving_reflect_system(
        mut reflect_events: EventWriter<ReflectEvent>,
        mut client: ResMut<RenetClient>,
        mut event_queue: ResMut<ServerEventQueue<ReflectEvent>>,
        replicon_tick: Res<RepliconTick>,
        channel: Res<ServerEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        while let Some(message) = client.receive_message(*channel) {
            let (tick, event) = server_event::deserialize_with(&message, |cursor| {
                let mut deserializer =
                    bincode::Deserializer::with_reader(cursor, DefaultOptions::new());
                let reflect = UntypedReflectDeserializer::new(&registry).deserialize(&mut deserializer)?;
                Ok(ReflectEvent(reflect))
            })
            .expect("server should send valid events");

            // Event should be sent to the queue if replication message with its tick has not yet arrived.
            if tick <= *replicon_tick {
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
        send_type: impl Into<SendType>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(send_type, sending_system::<T>, receiving_system::<T>)
    }

    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(
            send_type,
            sending_system::<T>,
            receiving_and_mapping_system::<T>,
        )
    }

    fn add_server_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        send_type: impl Into<SendType>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<NetworkChannels>()
            .create_server_channel(send_type.into());

        self.add_event::<T>()
            .init_resource::<Events<ToClients<T>>>()
            .init_resource::<ServerEventQueue<T>>()
            .insert_resource(ServerEventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                (
                    reset_system::<T>.in_set(ClientSet::ResetEvents),
                    (queue_system::<T>, receiving_system)
                        .chain()
                        .after(ClientPlugin::replication_receiving_system)
                        .in_set(ClientSet::Receive)
                        .run_if(client_connected),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    sending_system.run_if(resource_exists::<RenetServer>),
                    local_resending_system::<T>.run_if(has_authority),
                )
                    .chain()
                    .after(ServerPlugin::replication_sending_system)
                    .in_set(ServerSet::Send),
            );

        self
    }
}

/// Applies all queued events if their tick is less or equal to [`RepliconTick`].
fn queue_system<T: Event>(
    replicon_tick: Res<RepliconTick>,
    mut server_events: EventWriter<T>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
) {
    while let Some(event) = event_queue.try_pop(*replicon_tick) {
        server_events.send(event);
    }
}

fn receiving_system<T: Event + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
    replicon_tick: Res<RepliconTick>,
    channel: Res<ServerEventChannel<T>>,
) {
    while let Some(message) = client.receive_message(*channel) {
        let (tick, event) = deserialize_with(&message, |cursor| {
            DefaultOptions::new().deserialize_from(cursor)
        })
        .expect("server should send valid events");

        if tick <= *replicon_tick {
            server_events.send(event);
        } else {
            event_queue.insert(tick, event);
        }
    }
}

fn receiving_and_mapping_system<T: Event + MapEntities + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
    replicon_tick: Res<RepliconTick>,
    entity_map: Res<ServerEntityMap>,
    channel: Res<ServerEventChannel<T>>,
) {
    while let Some(message) = client.receive_message(*channel) {
        let (tick, mut event): (_, T) = deserialize_with(&message, |cursor| {
            DefaultOptions::new().deserialize_from(cursor)
        })
        .expect("server should send valid events");

        event.map_entities(&mut EventMapper(entity_map.to_client()));
        if tick <= *replicon_tick {
            server_events.send(event);
        } else {
            event_queue.insert(tick, event);
        }
    }
}

fn sending_system<T: Event + Serialize>(
    mut server: ResMut<RenetServer>,
    mut server_events: EventReader<ToClients<T>>,
    client_cache: Res<ClientCache>,
    channel: Res<ServerEventChannel<T>>,
) {
    for ToClients { event, mode } in server_events.read() {
        send_with(&mut server, &client_cache, *channel, *mode, |cursor| {
            DefaultOptions::new().serialize_into(cursor, &event)
        })
        .expect("server event should be serializable");
    }
}

/// Transforms [`ToClients<T>`] events into `T` events to "emulate"
/// message sending for offline mode or when server is also a player
fn local_resending_system<T: Event>(
    mut server_events: ResMut<Events<ToClients<T>>>,
    mut local_events: EventWriter<T>,
) {
    for ToClients { event, mode } in server_events.drain() {
        match mode {
            SendMode::Broadcast => {
                local_events.send(event);
            }
            SendMode::BroadcastExcept(client_id) => {
                if client_id != SERVER_ID {
                    local_events.send(event);
                }
            }
            SendMode::Direct(client_id) => {
                if client_id == SERVER_ID {
                    local_events.send(event);
                }
            }
        }
    }
}

/// Clears queued events.
///
/// We clear events while waiting for a connection to ensure clean reconnects.
fn reset_system<T: Event>(mut event_queue: ResMut<ServerEventQueue<T>>) {
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
    server: &mut RenetServer,
    client_cache: &ClientCache,
    channel: ServerEventChannel<T>,
    mode: SendMode,
    serialize_fn: impl Fn(&mut Cursor<Vec<u8>>) -> bincode::Result<()>,
) -> bincode::Result<()> {
    match mode {
        SendMode::Broadcast => {
            let mut previous_message = None;
            for client_state in client_cache.iter() {
                let message = serialize_with(client_state, previous_message, &serialize_fn)?;
                server.send_message(client_state.id(), channel, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::BroadcastExcept(client_id) => {
            let mut previous_message = None;
            for client_state in client_cache.iter() {
                if client_state.id() == client_id {
                    continue;
                }
                let message = serialize_with(client_state, previous_message, &serialize_fn)?;
                server.send_message(client_state.id(), channel, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::Direct(client_id) => {
            if client_id != SERVER_ID {
                if let Some(client_state) = client_cache.get_client(client_id) {
                    let message = serialize_with(client_state, None, &serialize_fn)?;
                    server.send_message(client_state.id(), channel, message.bytes);
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
    client_state: &ClientState,
    previous_message: Option<SerializedMessage>,
    serialize_fn: impl Fn(&mut Cursor<Vec<u8>>) -> bincode::Result<()>,
) -> bincode::Result<SerializedMessage> {
    if let Some(previous_message) = previous_message {
        if previous_message.tick == client_state.change_tick() {
            return Ok(previous_message);
        }

        let tick_size =
            DefaultOptions::new().serialized_size(&client_state.change_tick())? as usize;
        let mut bytes = Vec::with_capacity(tick_size + previous_message.event_bytes().len());
        DefaultOptions::new().serialize_into(&mut bytes, &client_state.change_tick())?;
        bytes.extend_from_slice(previous_message.event_bytes());
        let message = SerializedMessage {
            tick: client_state.change_tick(),
            tick_size,
            bytes: bytes.into(),
        };

        Ok(message)
    } else {
        let mut cursor = Cursor::new(Vec::new());
        DefaultOptions::new().serialize_into(&mut cursor, &client_state.change_tick())?;
        let tick_size = cursor.get_ref().len();
        (serialize_fn)(&mut cursor)?;
        let message = SerializedMessage {
            tick: client_state.change_tick(),
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
    deserialize_fn: impl FnOnce(&mut Cursor<&[u8]>) -> bincode::Result<T>,
) -> bincode::Result<(RepliconTick, T)> {
    let mut cursor = Cursor::new(message);
    let tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
    let event = (deserialize_fn)(&mut cursor)?;

    Ok((tick, event))
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
    fn try_pop(&mut self, replicon_tick: RepliconTick) -> Option<T> {
        let (tick, _) = self.0.front()?;
        if *tick > replicon_tick {
            return None;
        }
        self.0.pop_front().map(|(_, event)| event)
    }
}

impl<T> Default for ServerEventQueue<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}
