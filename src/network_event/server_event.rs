use std::fmt::Debug;

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bevy_renet::renet::{RenetClient, RenetServer};
use serde::{de::DeserializeOwned, Serialize};

use super::EventChannel;
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
            .add_system(receiving_system.in_set(OnUpdate(ClientState::Connected)))
            .add_systems(
                (
                    sending_system.in_set(OnUpdate(ServerState::Hosting)),
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
        let message = bincode::serialize(&event).expect("event should be serializable");

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
            .unwrap_or_else(|e| panic!("unable to map entities for server event {event:?}: {e}"));
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
    use bevy::ecs::{
        entity::{EntityMap, MapEntitiesError},
        event::Events,
    };
    use serde::Deserialize;

    use super::*;
    use crate::{test_network::TestNetworkPlugin, ReplicationPlugins};

    #[test]
    fn sending_receiving() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .add_server_event::<DummyEvent>()
            .add_plugin(TestNetworkPlugin);

        let client_id = app.world.resource::<RenetClient>().client_id();
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

    #[derive(Deserialize, Serialize, Debug)]
    struct DummyEvent(Entity);

    impl MapEntities for DummyEvent {
        fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapEntitiesError> {
            self.0 = entity_map.get(self.0)?;
            Ok(())
        }
    }
}
