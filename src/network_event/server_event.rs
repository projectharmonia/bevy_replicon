use std::{any, io::Cursor, iter};

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;
use ordered_multimap::ListOrderedMultimap;
use serde::{de::DeserializeOwned, Serialize};

use super::{EventMapper, NetworkEventFns, ReceiveFn, SendFn};
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
    prelude::ClientPlugin,
    server::{
        connected_clients::{ConnectedClient, ConnectedClients},
        replicon_server::RepliconServer,
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
    fn add_server_event_with<T: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_fn: SendFn,
        receive_fn: ReceiveFn,
    ) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self
        // self.add_server_event_with::<T>(channel, send::<T>, receive::<T>)
    }

    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self
        // self.add_server_event_with::<T>(channel, send::<T>, receive_and_map::<T>)
    }

    fn add_server_event_with<T: Event>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_fn: SendFn,
        receive_fn: ReceiveFn,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<RepliconChannels>()
            .create_server_channel(channel.into());

        self.add_event::<T>()
            .init_resource::<Events<ToClients<T>>>()
            .init_resource::<ServerEventQueue<T>>();

        // self.world
        // .resource_mut::<ServerEventRegistry>()
        // .events
        // .push(NetworkEventFns {
        // channel_id,
        // send: send_fn,
        // resend_locally: resend_locally::<T>,
        // receive: receive_fn,
        // reset: reset::<T>,
        // });

        self
    }
}

/// Applies all queued events if their tick is less or equal to [`RepliconTick`].
fn pop_from_queue<T: Event>(world: &mut World) {
    world.resource_scope(|world, mut server_events: Mut<Events<T>>| {
        world.resource_scope(|world, mut event_queue: Mut<ServerEventQueue<T>>| {
            let init_tick = world.resource::<ServerInitTick>();
            let events = iter::from_fn(|| {
                event_queue.pop_if_le(**init_tick).map(|(tick, event)| {
                    trace!(
                        "applying event `{}` from queue with `{tick:?}`",
                        any::type_name::<T>()
                    );
                    event
                })
            });

            server_events.send_batch(events);
        });
    });
}

fn receive<T: Event + DeserializeOwned>(world: &mut World, channel_id: u8) {
    pop_from_queue::<T>(world);
    world.resource_scope(|world, mut server_events: Mut<Events<T>>| {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, mut event_queue: Mut<ServerEventQueue<T>>| {
                let init_tick = world.resource::<ServerInitTick>();

                for message in client.receive(channel_id) {
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
            });
        });
    });
}

fn receive_and_map<T: Event + MapEntities + DeserializeOwned>(world: &mut World, channel_id: u8) {
    pop_from_queue::<T>(world);
    world.resource_scope(|world, mut server_events: Mut<Events<T>>| {
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            world.resource_scope(|world, mut event_queue: Mut<ServerEventQueue<T>>| {
                let init_tick = world.resource::<ServerInitTick>();
                let entity_map = world.resource::<ServerEntityMap>();
                for message in client.receive(channel_id) {
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
            });
        });
    });
}

fn send<T: Event + Serialize>(world: &mut World, channel_id: u8) {
    world.resource_scope(|world, mut server: Mut<RepliconServer>| {
        let events = world.resource::<Events<ToClients<T>>>();
        let connected_clients = world.resource::<ConnectedClients>();
        for ToClients { event, mode } in events.get_reader().read(&events) {
            trace!("sending event `{}` with `{mode:?}`", any::type_name::<T>());
            send_with(
                &mut server,
                &connected_clients,
                channel_id,
                *mode,
                |cursor| DefaultOptions::new().serialize_into(cursor, &event),
            )
            .expect("server event should be serializable");
        }
    });
}

/// Transforms [`ToClients<T>`] events into `T` events to "emulate"
/// message sending for offline mode or when server is also a player.
fn resend_locally<T: Event>(world: &mut World) {
    world.resource_scope(|world, mut local_events: Mut<Events<T>>| {
        world.resource_scope(|_world, mut server_events: Mut<Events<ToClients<T>>>| {
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
        });
    });
}

/// Clears queued events.
///
/// We clear events while waiting for a connection to ensure clean reconnects.
fn reset<T: Event>(world: &mut World) {
    let mut event_queue = world.resource_mut::<ServerEventQueue<T>>();
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
pub fn send_with(
    server: &mut RepliconServer,
    connected_clients: &ConnectedClients,
    channel_id: u8,
    mode: SendMode,
    serialize: impl Fn(&mut Cursor<Vec<u8>>) -> bincode::Result<()>,
) -> bincode::Result<()> {
    match mode {
        SendMode::Broadcast => {
            let mut previous_message = None;
            for client in connected_clients.iter() {
                let message = serialize_with(client, previous_message, &serialize)?;
                server.send(client.id(), channel_id, message.bytes.clone());
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
                server.send(client.id(), channel_id, message.bytes.clone());
                previous_message = Some(message);
            }
        }
        SendMode::Direct(client_id) => {
            if client_id != ClientId::SERVER {
                if let Some(client) = connected_clients.get_client(client_id) {
                    let message = serialize_with(client, None, &serialize)?;
                    server.send(client.id(), channel_id, message.bytes);
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

#[derive(Resource, Default)]
struct ServerEventRegistry {
    events: Vec<NetworkEventFns>,
}

pub struct ServerEventPlugin;

impl Plugin for ServerEventPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerEventRegistry>()
            .add_systems(
                PreUpdate,
                (
                    reset_system.in_set(ClientSet::ResetEvents),
                    receive_system
                        .after(ClientPlugin::receive_replication)
                        .in_set(ClientSet::Receive)
                        .run_if(client_connected),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    send_system.run_if(server_running),
                    resend_locally_system.run_if(has_authority),
                )
                    .chain(),
            );
    }
}

fn reset_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ServerEventRegistry>| {
        for event in registry.events.iter() {
            (event.reset)(world);
        }
    });
}

fn receive_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ServerEventRegistry>| {
        for event in registry.events.iter() {
            (event.receive)(world, event.channel_id);
        }
    });
}

fn send_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ServerEventRegistry>| {
        for event in registry.events.iter() {
            // (event.send)(world, event.channel_id);
        }
    })
}

fn resend_locally_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ServerEventRegistry>| {
        for event in registry.events.iter() {
            (event.resend_locally)(world);
        }
    })
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
