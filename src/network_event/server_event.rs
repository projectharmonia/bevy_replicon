use bevy::{ecs::event::Event, prelude::*};
use bevy_renet::{
    client_connected,
    renet::{ClientId, RenetClient, RenetServer, SendType},
};
use bincode::{DefaultOptions, Options};
use ordered_multimap::ListOrderedMultimap;
use serde::{de::DeserializeOwned, Serialize};

use super::EventChannel;
use crate::{
    client::{client_mapper::ServerEntityMap, ClientSet},
    network_event::EventMapper,
    prelude::{ClientPlugin, ServerPlugin},
    replicon_core::{
        replication_rules::MapNetworkEntities, replicon_tick::RepliconTick, NetworkChannels,
    },
    server::{has_authority, LastChangeTick, ServerSet, SERVER_ID},
};

/// An extension trait for [`App`] for creating server events.
pub trait ServerEventAppExt {
    /// Registers event `T` that will be emitted on client after sending [`ToClients<T>`] on server.
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self;

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client after receiving.
    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapNetworkEntities>(
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
        last_change_tick: Res<LastChangeTick>,
        channel: Res<EventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for ToClients { event, mode } in reflect_events.read() {
            let message = serialize_reflect_event(**last_change_tick, &event, &registry)
                .expect("server event should be serializable");

            server_event::send(&mut server, *channel, *mode, message)
        }
    }

    fn receiving_reflect_system(
        mut reflect_events: EventWriter<ReflectEvent>,
        mut client: ResMut<RenetClient>,
        mut event_queue: ResMut<ServerEventQueue<ReflectEvent>>,
        replicon_tick: Res<RepliconTick>,
        channel: Res<EventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        while let Some(message) = client.receive_message(*channel) {
            let (tick, event) = deserialize_reflect_event(&message, &registry)
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

    fn serialize_reflect_event(
        tick: RepliconTick,
        event: &ReflectEvent,
        registry: &TypeRegistry,
    ) -> bincode::Result<Vec<u8>> {
        let mut message = Vec::new();
        DefaultOptions::new().serialize_into(&mut message, &tick)?;
        let serializer = ReflectSerializer::new(&*event.0, registry);
        DefaultOptions::new().serialize_into(&mut message, &serializer)?;

        Ok(message)
    }

    fn deserialize_reflect_event(
        message: &[u8],
        registry: &TypeRegistry,
    ) -> bincode::Result<(RepliconTick, ReflectEvent)> {
        let mut cursor = Cursor::new(message);
        let tick = DefaultOptions::new().deserialize_from(&mut cursor)?;
        let mut deserializer =
            bincode::Deserializer::with_reader(&mut cursor, DefaultOptions::new());
        let reflect = UntypedReflectDeserializer::new(registry).deserialize(&mut deserializer)?;

        Ok((tick, ReflectEvent(reflect)))
    }
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

    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapNetworkEntities>(
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
            .insert_resource(EventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                (queue_system::<T>, receiving_system)
                    .chain()
                    .after(ClientPlugin::replication_receiving_system)
                    .in_set(ClientSet::Receive)
                    .run_if(client_connected()),
            )
            .add_systems(
                PostUpdate,
                (
                    (
                        sending_system.run_if(resource_exists::<RenetServer>()),
                        local_resending_system::<T>.run_if(has_authority()),
                    )
                        .chain()
                        .after(ServerPlugin::replication_sending_system)
                        .in_set(ServerSet::Send),
                    reset_system::<T>.run_if(resource_removed::<RenetClient>()),
                ),
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
    while event_queue
        .front()
        .filter(|(&tick, _)| tick <= *replicon_tick)
        .is_some()
    {
        let (_, event) = event_queue.pop_front().unwrap();
        server_events.send(event);
    }
}

fn receiving_system<T: Event + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
    replicon_tick: Res<RepliconTick>,
    channel: Res<EventChannel<T>>,
) {
    while let Some(message) = client.receive_message(*channel) {
        let (tick, event) = DefaultOptions::new()
            .deserialize(&message)
            .expect("server should send valid events");

        if tick <= *replicon_tick {
            server_events.send(event);
        } else {
            event_queue.insert(tick, event);
        }
    }
}

fn receiving_and_mapping_system<T: Event + MapNetworkEntities + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    mut event_queue: ResMut<ServerEventQueue<T>>,
    replicon_tick: Res<RepliconTick>,
    entity_map: Res<ServerEntityMap>,
    channel: Res<EventChannel<T>>,
) {
    while let Some(message) = client.receive_message(*channel) {
        let (tick, mut event): (_, T) = DefaultOptions::new()
            .deserialize(&message)
            .expect("server should send valid mapped events");

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
    last_change_tick: Res<LastChangeTick>,
    channel: Res<EventChannel<T>>,
) {
    for ToClients { event, mode } in server_events.read() {
        let message = DefaultOptions::new()
            .serialize(&(**last_change_tick, event))
            .expect("server event should be serializable");

        send(&mut server, *channel, *mode, message);
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

fn reset_system<T: Event>(mut event_queue: ResMut<ServerEventQueue<T>>) {
    event_queue.clear();
}

/// Sends serialized `message` to clients.
///
/// Helper for custom sending systems.
/// See also [`ServerEventAppExt::add_server_event_with`]
pub fn send<T>(
    server: &mut RenetServer,
    channel: EventChannel<T>,
    mode: SendMode,
    message: Vec<u8>,
) {
    match mode {
        SendMode::Broadcast => {
            server.broadcast_message(channel, message);
        }
        SendMode::BroadcastExcept(client_id) => {
            if client_id == SERVER_ID {
                server.broadcast_message(channel, message);
            } else {
                server.broadcast_message_except(client_id, channel, message);
            }
        }
        SendMode::Direct(client_id) => {
            if client_id != SERVER_ID {
                server.send_message(client_id, channel, message);
            }
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
#[derive(Deref, DerefMut, Resource)]
pub struct ServerEventQueue<T>(ListOrderedMultimap<RepliconTick, T>);

impl<T> Default for ServerEventQueue<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}
