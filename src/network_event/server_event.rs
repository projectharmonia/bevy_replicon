use bevy::{ecs::event::Event, prelude::*};
use bevy_renet::{
    renet::{RenetClient, RenetServer, SendType},
    transport::client_connected,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::EventChannel;
use crate::{
    client::{ClientSet, ServerEntityMap},
    network_event::EventMapper,
    replicon_core::{replication_rules::MapNetworkEntities, NetworkChannels},
    server::{has_authority, ServerSet, SERVER_ID},
};

/// An extension trait for [`App`] for creating server events.
pub trait ServerEventAppExt {
    /// Registers event `T` that will be emitted on client after sending [`ToClients<T>`] on server.
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        policy: impl Into<SendType>,
    ) -> &mut Self;

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client after receiving.
    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapNetworkEntities>(
        &mut self,
        policy: impl Into<SendType>,
    ) -> &mut Self;

    /**
    Same as [`Self::add_server_event`], but uses specified sending and receiving systems.

    It's advised to not panic in sending system because it runs on server.

    # Examples

    Serialize an event with `Box<dyn Reflect>`:

    ```
    use bevy::{prelude::*, reflect::serde::{ReflectSerializer, UntypedReflectDeserializer}};
    use bevy_replicon::{network_event::server_event, prelude::*};
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ReplicationPlugins));
    app.add_server_event_with::<ReflectEvent, _, _>(
        SendPolicy::Ordered,
        sending_reflect_system,
        receiving_reflect_system,
    );

    fn sending_reflect_system(
        mut server: ResMut<RenetServer>,
        mut server_events: EventReader<ToClients<ReflectEvent>>,
        channel: Res<EventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for ToClients { event, mode } in &mut server_events {
            let serializer = ReflectSerializer::new(&*event.0, &registry);
            let message = DefaultOptions::new()
                .serialize(&serializer)
                .expect("server event should be serializable");

            server_event::send(&mut server, *channel, *mode, message)
        }
    }

    fn receiving_reflect_system(
        mut server_events: EventWriter<ReflectEvent>,
        mut client: ResMut<RenetClient>,
        channel: Res<EventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        while let Some(message) = client.receive_message(*channel) {
            let mut deserializer = bincode::Deserializer::from_slice(&message, DefaultOptions::new());
            let event = UntypedReflectDeserializer::new(&registry)
                .deserialize(&mut deserializer)
                .expect("server should send valid events");

            server_events.send(ReflectEvent(event));
        }
    }

    #[derive(Event)]
    struct ReflectEvent(Box<dyn Reflect>);
    ```
    */
    fn add_server_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        policy: impl Into<SendType>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        policy: impl Into<SendType>,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(policy, sending_system::<T>, receiving_system::<T>)
    }

    fn add_mapped_server_event<T: Event + Serialize + DeserializeOwned + MapNetworkEntities>(
        &mut self,
        policy: impl Into<SendType>,
    ) -> &mut Self {
        self.add_server_event_with::<T, _, _>(
            policy,
            sending_system::<T>,
            receiving_and_mapping_system::<T>,
        )
    }

    fn add_server_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        policy: impl Into<SendType>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<NetworkChannels>()
            .create_server_channel(policy.into());

        self.add_event::<T>()
            .init_resource::<Events<ToClients<T>>>()
            .insert_resource(EventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                receiving_system
                    .in_set(ClientSet::Receive)
                    .run_if(client_connected()),
            )
            .add_systems(
                PostUpdate,
                (
                    sending_system.run_if(resource_exists::<RenetServer>()),
                    local_resending_system::<T>.run_if(has_authority()),
                )
                    .chain()
                    .in_set(ServerSet::Send),
            );

        self
    }
}

fn receiving_system<T: Event + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    channel: Res<EventChannel<T>>,
) {
    while let Some(message) = client.receive_message(channel.id) {
        let event = DefaultOptions::new()
            .deserialize(&message)
            .expect("server should send valid events");

        server_events.send(event);
    }
}

fn receiving_and_mapping_system<T: Event + MapNetworkEntities + DeserializeOwned>(
    mut server_events: EventWriter<T>,
    mut client: ResMut<RenetClient>,
    entity_map: Res<ServerEntityMap>,
    channel: Res<EventChannel<T>>,
) {
    while let Some(message) = client.receive_message(channel.id) {
        let mut event: T = DefaultOptions::new()
            .deserialize(&message)
            .expect("server should send valid mapped events");

        event.map_entities(&mut EventMapper(entity_map.to_client()));
        server_events.send(event);
    }
}

fn sending_system<T: Event + Serialize>(
    mut server: ResMut<RenetServer>,
    mut server_events: EventReader<ToClients<T>>,
    channel: Res<EventChannel<T>>,
) {
    for ToClients { event, mode } in &mut server_events {
        let message = DefaultOptions::new()
            .serialize(&event)
            .expect("server event should be serializable");

        send(&mut server, *channel, *mode, message);
    }
}

/// Sends serialized `message` to clients.
///
/// Helper for custom sending system.
/// See also [`ServerEventAppExt::add_server_event_with`]
pub fn send<T>(
    server: &mut RenetServer,
    channel: EventChannel<T>,
    mode: SendMode,
    message: Vec<u8>,
) {
    match mode {
        SendMode::Broadcast => {
            server.broadcast_message(channel.id, message);
        }
        SendMode::BroadcastExcept(client_id) => {
            if client_id == SERVER_ID {
                server.broadcast_message(channel.id, message);
            } else {
                server.broadcast_message_except(client_id, channel.id, message);
            }
        }
        SendMode::Direct(client_id) => {
            if client_id != SERVER_ID {
                server.send_message(client_id, channel.id, message);
            }
        }
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
    BroadcastExcept(u64),
    Direct(u64),
}
