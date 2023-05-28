use std::fmt::Debug;

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bevy_renet::renet::{RenetClient, RenetServer};
use bincode::{DefaultOptions, Options};
use serde::{
    de::{DeserializeOwned, DeserializeSeed},
    Serialize,
};

use super::{BuildEventDeserializer, BuildEventSerializer, EventChannel};
use crate::{
    client::{ClientState, NetworkEntityMap},
    prelude::NetworkChannels,
    server::{ServerSet, ServerState, SERVER_ID},
};

/// An extension trait for [`App`] for creating server events.
pub trait ServerEventAppExt {
    /// Registers event `T` that will be emitted on client after sending [`ToClients<T>`] on server.
    fn add_server_event<T: Event + Serialize + DeserializeOwned + Debug>(&mut self) -> &mut Self;

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client after receiving.
    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + Debug + MapEntities>(
        &mut self,
    ) -> &mut Self;

    /// Same as [`Self::add_server_event`], but the event will be serialized/deserialized using `S`/`D`
    /// with access to [`AppTypeRegistry`].
    ///
    /// Needed to send events that contain things like `Box<dyn Reflect>`.
    fn add_server_reflect_event<T, S, D>(&mut self) -> &mut Self
    where
        T: Event + Debug,
        S: BuildEventSerializer<T> + 'static,
        D: BuildEventDeserializer + 'static,
        for<'a> S::EventSerializer<'a>: Serialize,
        for<'a, 'de> D::EventDeserializer<'a>: DeserializeSeed<'de, Value = T>;

    /// Same as [`Self::add_server_reflect_event`], but additionally maps client entities to client after receiving.
    fn add_mapped_server_reflect_event<T, S, D>(&mut self) -> &mut Self
    where
        T: Event + Debug + MapEntities,
        S: BuildEventSerializer<T> + 'static,
        D: BuildEventDeserializer + 'static,
        for<'a> S::EventSerializer<'a>: Serialize,
        for<'a, 'de> D::EventDeserializer<'a>: DeserializeSeed<'de, Value = T>;

    /// Same as [`Self::add_server_event`], but uses specified sending and receiving systems.
    fn add_server_event_with<T: Event + Debug, Marker1, Marker2>(
        &mut self,
        sending_system: impl IntoSystemConfig<Marker1>,
        receiving_system: impl IntoSystemConfig<Marker2>,
    ) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event<T: Event + Serialize + DeserializeOwned + Debug>(&mut self) -> &mut Self {
        self.add_server_event_with::<T, _, _>(sending_system::<T>, receiving_system::<T>)
    }

    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + Debug + MapEntities>(
        &mut self,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(
            sending_system::<T>,
            receiving_and_mapping_system::<T>,
        )
    }

    fn add_server_reflect_event<T, S, D>(&mut self) -> &mut Self
    where
        T: Event + Debug,
        S: BuildEventSerializer<T> + 'static,
        D: BuildEventDeserializer + 'static,
        for<'a> S::EventSerializer<'a>: Serialize,
        for<'a, 'de> D::EventDeserializer<'a>: DeserializeSeed<'de, Value = T>,
    {
        self.add_server_event_with::<T, _, _>(
            sending_reflect_system::<T, S>,
            receiving_reflect_system::<T, D>,
        )
    }

    fn add_mapped_server_reflect_event<T, S, D>(&mut self) -> &mut Self
    where
        T: Event + Debug + MapEntities,
        S: BuildEventSerializer<T> + 'static,
        D: BuildEventDeserializer + 'static,
        for<'a> S::EventSerializer<'a>: Serialize,
        for<'a, 'de> D::EventDeserializer<'a>: DeserializeSeed<'de, Value = T>,
    {
        self.add_server_event_with::<T, _, _>(
            sending_reflect_system::<T, S>,
            receiving_and_mapping_reflect_system::<T, D>,
        )
    }

    fn add_server_event_with<T: Event + Debug, Marker1, Marker2>(
        &mut self,
        sending_system: impl IntoSystemConfig<Marker1>,
        receiving_system: impl IntoSystemConfig<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<NetworkChannels>()
            .create_server_channel();

        self.add_event::<T>()
            .init_resource::<Events<ToClients<T>>>()
            .insert_resource(EventChannel::<T>::new(channel_id))
            .add_system(receiving_system.in_set(ServerSet::ReceiveEvents).run_if(
                resource_exists::<State<ClientState>>().and_then(in_state(ClientState::Connected)),
            ))
            .add_systems(
                (
                    sending_system.in_set(ServerSet::SendEvents).run_if(
                        resource_exists::<State<ServerState>>()
                            .and_then(in_state(ServerState::Hosting)),
                    ),
                    local_resending_system::<T>.in_set(ServerSet::Authority),
                )
                    .chain()
                    .in_set(ServerSet::Tick),
            );

        self
    }
}

fn sending_system<T: Event + Serialize + Debug>(
    mut server: ResMut<RenetServer>,
    mut server_events: EventReader<ToClients<T>>,
    channel: Res<EventChannel<T>>,
) {
    for ToClients { event, mode } in &mut server_events {
        let message = bincode::serialize(&event).expect("server event should be serializable");

        match *mode {
            SendMode::Broadcast => {
                server.broadcast_message(channel.id, message);
                debug!("broadcasted server event {event:?}");
            }
            SendMode::BroadcastExcept(client_id) => {
                if client_id == SERVER_ID {
                    server.broadcast_message(channel.id, message);
                } else {
                    server.broadcast_message_except(client_id, channel.id, message);
                }
                debug!("broadcasted server event {event:?} except client {client_id}");
            }
            SendMode::Direct(client_id) => {
                if client_id != SERVER_ID {
                    server.send_message(client_id, channel.id, message);
                    debug!("sent direct server event {event:?} to client {client_id}");
                }
            }
        }
    }
}

fn sending_reflect_system<T, S>(
    mut server: ResMut<RenetServer>,
    mut server_events: EventReader<ToClients<T>>,
    channel: Res<EventChannel<T>>,
    registry: Res<AppTypeRegistry>,
) where
    T: Event + Debug,
    S: BuildEventSerializer<T>,
    for<'a> S::EventSerializer<'a>: Serialize,
{
    let registry = registry.read();
    for ToClients { event, mode } in &mut server_events {
        let serializer = S::new(event, &registry);
        let message = bincode::serialize(&serializer).expect("server event should be serializable");

        match *mode {
            SendMode::Broadcast => {
                server.broadcast_message(channel.id, message);
                debug!("broadcasted server reflect event {event:?}");
            }
            SendMode::BroadcastExcept(client_id) => {
                if client_id == SERVER_ID {
                    server.broadcast_message(channel.id, message);
                } else {
                    server.broadcast_message_except(client_id, channel.id, message);
                }
                debug!("broadcasted server reflect event {event:?} except client {client_id}");
            }
            SendMode::Direct(client_id) => {
                if client_id != SERVER_ID {
                    server.send_message(client_id, channel.id, message);
                    debug!("sent direct server reflect event {event:?} to client {client_id}");
                }
            }
        }
    }
}

/// Transforms [`ToClients<T>`] events into [`T`] events to "emulate"
/// message sending for offline mode or when server is also a player
fn local_resending_system<T: Event + Debug>(
    mut server_events: ResMut<Events<ToClients<T>>>,
    mut local_events: EventWriter<T>,
) {
    for ToClients { event, mode } in server_events.drain() {
        match mode {
            SendMode::Broadcast => {
                debug!("converted broadcasted server event {event:?} into a local");
                local_events.send(event);
            }
            SendMode::BroadcastExcept(client_id) => {
                if client_id != SERVER_ID {
                    debug!("converted broadcasted server event {event:?} except client {client_id} into a local");
                    local_events.send(event);
                }
            }
            SendMode::Direct(client_id) => {
                if client_id == SERVER_ID {
                    debug!("converted direct server event {event:?} into a local");
                    local_events.send(event);
                }
            }
        }
    }
}

fn receiving_system<T: Event + DeserializeOwned + Debug>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    channel: Res<EventChannel<T>>,
) {
    while let Some(message) = client.receive_message(channel.id) {
        let event = bincode::deserialize(&message).expect("server should send valid events");
        debug!("received event {event:?} from server");
        server_events.send(event);
    }
}

fn receiving_and_mapping_system<T: Event + MapEntities + DeserializeOwned + Debug>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    entity_map: Res<NetworkEntityMap>,
    channel: Res<EventChannel<T>>,
) {
    while let Some(message) = client.receive_message(channel.id) {
        let mut event: T =
            bincode::deserialize(&message).expect("server should send valid mapped events");
        debug!("received mapped event {event:?} from server");
        event
            .map_entities(entity_map.to_client())
            .unwrap_or_else(|e| panic!("server event {event:?} should map its entities: {e}"));
        server_events.send(event);
    }
}

fn receiving_reflect_system<T, D>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    channel: Res<EventChannel<T>>,
    registry: Res<AppTypeRegistry>,
) where
    T: Event + Debug,
    D: BuildEventDeserializer,
    for<'a, 'de> D::EventDeserializer<'a>: DeserializeSeed<'de, Value = T>,
{
    let registry = registry.read();
    while let Some(message) = client.receive_message(channel.id) {
        // Set options to match `bincode::serialize`.
        // https://docs.rs/bincode/latest/bincode/config/index.html#options-struct-vs-bincode-functions
        let options = DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();
        let mut deserializer = bincode::Deserializer::from_slice(&message, options);
        let event = D::new(&registry)
            .deserialize(&mut deserializer)
            .expect("server should send valid reflect events");
        debug!("received reflect event {event:?} from server");
        server_events.send(event);
    }
}

fn receiving_and_mapping_reflect_system<T, D>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    entity_map: Res<NetworkEntityMap>,
    channel: Res<EventChannel<T>>,
    registry: Res<AppTypeRegistry>,
) where
    T: Event + MapEntities + Debug,
    D: BuildEventDeserializer,
    for<'a, 'de> D::EventDeserializer<'a>: DeserializeSeed<'de, Value = T>,
{
    let registry = registry.read();
    while let Some(message) = client.receive_message(channel.id) {
        // Set options to match `bincode::serialize`.
        // https://docs.rs/bincode/latest/bincode/config/index.html#options-struct-vs-bincode-functions
        let options = DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();
        let mut deserializer = bincode::Deserializer::from_slice(&message, options);
        let mut event = D::new(&registry)
            .deserialize(&mut deserializer)
            .expect("server should send valid mapped reflect events");
        debug!("received mapped reflect event {event:?} from server");
        event
            .map_entities(entity_map.to_client())
            .unwrap_or_else(|e| {
                panic!("server reflect event {event:?} should map its entities: {e}")
            });
        server_events.send(event);
    }
}

/// An event that will be send to client(s).
#[derive(Clone, Copy, Debug)]
pub struct ToClients<T> {
    pub mode: SendMode,
    pub event: T,
}

/// Type of server message sending.
#[derive(Clone, Copy, Debug)]
pub enum SendMode {
    Broadcast,
    BroadcastExcept(u64),
    Direct(u64),
}

#[cfg(test)]
mod tests {
    use bevy::ecs::event::Events;
    use bevy_renet::renet::transport::NetcodeClientTransport;

    use super::*;
    use crate::{
        network_event::test_events::{
            DummyComponent, DummyEvent, ReflectEvent, ReflectEventDeserializer,
            ReflectEventSerializer,
        },
        test_network::TestNetworkPlugin,
        ClientPlugin, ReplicationPlugins, ServerPlugin,
    };

    #[test]
    fn without_server_plugin() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ReplicationPlugins.build().disable::<ServerPlugin>())
            .add_server_event_with::<DummyEvent, _, _>(|| {}, || {})
            .update();
    }

    #[test]
    fn without_client_plugin() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ReplicationPlugins.build().disable::<ClientPlugin>())
            .add_server_event_with::<DummyEvent, _, _>(|| {}, || {})
            .update();
    }

    #[test]
    fn sending_receiving() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .add_server_event::<DummyEvent>()
            .add_plugin(TestNetworkPlugin);

        let client_id = app.world.resource::<NetcodeClientTransport>().client_id();
        for (mode, events_count) in [
            (SendMode::Broadcast, 1),
            (SendMode::Direct(SERVER_ID), 0),
            (SendMode::Direct(client_id), 1),
            (SendMode::BroadcastExcept(SERVER_ID), 1),
            (SendMode::BroadcastExcept(client_id), 0),
        ] {
            app.world
                .resource_mut::<Events<ToClients<DummyEvent>>>()
                .send(ToClients {
                    mode,
                    event: DummyEvent(Entity::PLACEHOLDER),
                });

            app.update();
            app.update();

            let dummy_events = app.world.resource::<Events<DummyEvent>>();
            assert_eq!(
                dummy_events.len(),
                events_count,
                "event should be emited {events_count} times for {mode:?}"
            );
        }
    }

    #[test]
    fn sending_receiving_and_mapping() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .add_mapped_server_event::<DummyEvent>()
            .add_plugin(TestNetworkPlugin);

        let client_entity = Entity::from_raw(0);
        let server_entity = Entity::from_raw(client_entity.index() + 1);
        app.world
            .resource_mut::<NetworkEntityMap>()
            .insert(server_entity, client_entity);

        app.world
            .resource_mut::<Events<ToClients<DummyEvent>>>()
            .send(ToClients {
                mode: SendMode::Broadcast,
                event: DummyEvent(server_entity),
            });

        app.update();
        app.update();

        let mapped_entities: Vec<_> = app
            .world
            .resource_mut::<Events<DummyEvent>>()
            .drain()
            .map(|event| event.0)
            .collect();
        assert_eq!(mapped_entities, [client_entity]);
    }

    #[test]
    fn sending_receiving_reflect() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .register_type::<DummyComponent>()
            .add_server_reflect_event::<ReflectEvent, ReflectEventSerializer, ReflectEventDeserializer>()
            .add_plugin(TestNetworkPlugin);

        let client_id = app.world.resource::<NetcodeClientTransport>().client_id();
        for (mode, events_count) in [
            (SendMode::Broadcast, 1),
            (SendMode::Direct(SERVER_ID), 0),
            (SendMode::Direct(client_id), 1),
            (SendMode::BroadcastExcept(SERVER_ID), 1),
            (SendMode::BroadcastExcept(client_id), 0),
        ] {
            app.world
                .resource_mut::<Events<ToClients<ReflectEvent>>>()
                .send(ToClients {
                    mode,
                    event: ReflectEvent {
                        entity: Entity::PLACEHOLDER,
                        component: DummyComponent.clone_value(),
                    },
                });

            app.update();
            app.update();

            let mut reflect_events = app.world.resource_mut::<Events<ReflectEvent>>();
            assert_eq!(reflect_events.drain().count(), events_count);
        }
    }

    #[test]
    fn sending_receiving_and_mapping_reflect() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .register_type::<DummyComponent>()
            .add_mapped_server_reflect_event::<ReflectEvent, ReflectEventSerializer, ReflectEventDeserializer>()
            .add_plugin(TestNetworkPlugin);

        let client_entity = Entity::from_raw(0);
        let server_entity = Entity::from_raw(client_entity.index() + 1);
        app.world
            .resource_mut::<NetworkEntityMap>()
            .insert(server_entity, client_entity);

        app.world
            .resource_mut::<Events<ToClients<ReflectEvent>>>()
            .send(ToClients {
                mode: SendMode::Broadcast,
                event: ReflectEvent {
                    entity: server_entity,
                    component: DummyComponent.clone_value(),
                },
            });

        app.update();
        app.update();

        let mapped_entities: Vec<_> = app
            .world
            .resource_mut::<Events<ReflectEvent>>()
            .drain()
            .map(|event| event.entity)
            .collect();
        assert_eq!(mapped_entities, [client_entity]);
    }

    #[test]
    fn local_resending() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .add_server_event::<DummyEvent>();

        const DUMMY_CLIENT_ID: u64 = 1;
        for (mode, events_count) in [
            (SendMode::Broadcast, 1),
            (SendMode::Direct(SERVER_ID), 1),
            (SendMode::Direct(DUMMY_CLIENT_ID), 0),
            (SendMode::BroadcastExcept(SERVER_ID), 0),
            (SendMode::BroadcastExcept(DUMMY_CLIENT_ID), 1),
        ] {
            app.world
                .resource_mut::<Events<ToClients<DummyEvent>>>()
                .send(ToClients {
                    mode,
                    event: DummyEvent(Entity::PLACEHOLDER),
                });

            app.update();

            let server_events = app.world.resource::<Events<ToClients<DummyEvent>>>();
            assert!(server_events.is_empty());

            let mut dummy_events = app.world.resource_mut::<Events<DummyEvent>>();
            assert_eq!(
                dummy_events.drain().count(),
                events_count,
                "event should be emited {events_count} times for {mode:?}"
            );
        }
    }
}
