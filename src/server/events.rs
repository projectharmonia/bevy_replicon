mod event_data;

use std::io::Cursor;

use bevy::{ecs::entity::MapEntities, prelude::*};
use bincode::{DefaultOptions, Options};
use ordered_multimap::ListOrderedMultimap;
use serde::{de::DeserializeOwned, Serialize};

use super::{
    connected_clients::ConnectedClients, replicon_server::RepliconServer, ServerPlugin, ServerSet,
};
use crate::{
    client::{
        replicon_client::RepliconClient, server_entity_map::ServerEntityMap, ClientPlugin,
        ClientSet, ServerInitTick,
    },
    core::{
        channels::{RepliconChannel, RepliconChannels},
        common_conditions::*,
        ctx::{ClientReceiveCtx, ServerSendCtx},
        replicon_tick::RepliconTick,
        ClientId,
    },
};
use event_data::ServerEventData;

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

pub struct ServerEventsPlugin;

impl Plugin for ServerEventsPlugin {
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

impl ServerEventsPlugin {
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
                                .get_resource_by_id(event_data.server_events_id())
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
                                    .get_resource_mut_by_id(event_data.events_id())
                                    .expect("events shouldn't be removed");
                                let queue = world_cell
                                    .get_resource_mut_by_id(event_data.queue_id())
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
                        .get_resource_mut_by_id(event_data.server_events_id())
                        .expect("server events shouldn't be removed");
                    let events = world_cell
                        .get_resource_mut_by_id(event_data.events_id())
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
                    .get_resource_mut_by_id(event_data.events_id())
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

/// Signature of server event serialization functions.
pub type SerializeFn<E> = fn(&mut ServerSendCtx, &E, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of server event deserialization functions.
pub type DeserializeFn<E> = fn(&mut ClientReceiveCtx, &mut Cursor<&[u8]>) -> bincode::Result<E>;

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
