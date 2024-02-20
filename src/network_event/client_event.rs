use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bevy_renet::{
    client_connected,
    renet::{ClientId, RenetClient, RenetServer, SendType},
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::ClientEventChannel;
use crate::{
    client::{client_mapper::ServerEntityMap, ClientSet},
    network_event::EventMapper,
    replicon_core::NetworkChannels,
    server::{has_authority, ServerSet, SERVER_ID},
};

/// An extension trait for [`App`] for creating client events.
pub trait ClientEventAppExt {
    /// Registers [`FromClient<T>`] event that will be emitted on server after sending `T` event on client.
    fn add_client_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self;

    /// Same as [`Self::add_client_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities.
    fn add_mapped_client_event<T: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self;

    /**
    Same as [`Self::add_client_event`], but uses specified sending and receiving systems.

    It's advised to not panic in the receiving system because it runs on the server.

    # Examples

    Serialize an event with `Box<dyn Reflect>`:

    ```
    use bevy::{prelude::*, reflect::serde::{ReflectSerializer, UntypedReflectDeserializer}};
    use bevy_replicon::prelude::*;
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ReplicationPlugins));
    app.add_client_event_with::<ReflectEvent, _, _>(
        EventType::Ordered,
        sending_reflect_system,
        receiving_reflect_system,
    );

    fn sending_reflect_system(
        mut reflect_events: EventReader<ReflectEvent>,
        mut client: ResMut<RenetClient>,
        channel: Res<ClientEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for event in reflect_events.read() {
            let serializer = ReflectSerializer::new(&*event.0, &registry);
            let message = DefaultOptions::new()
                .serialize(&serializer)
                .expect("client event should be serializable");

            client.send_message(*channel, message);
        }
    }

    fn receiving_reflect_system(
        mut reflect_events: EventWriter<FromClient<ReflectEvent>>,
        mut server: ResMut<RenetServer>,
        channel: Res<ClientEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for client_id in server.clients_id() {
            while let Some(message) = server.receive_message(client_id, *channel) {
                let mut deserializer =
                    bincode::Deserializer::from_slice(&message, DefaultOptions::new());
                match UntypedReflectDeserializer::new(&registry).deserialize(&mut deserializer) {
                    Ok(reflect) => {
                        reflect_events.send(FromClient {
                            client_id,
                            event: ReflectEvent(reflect),
                        });
                    }
                    Err(e) => {
                        debug!("unable to deserialize event from client {client_id}: {e}")
                    }
                }
            }
        }
    }

    #[derive(Event)]
    struct ReflectEvent(Box<dyn Reflect>);
    ```
    */
    fn add_client_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        send_type: impl Into<SendType>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self {
        self.add_client_event_with::<T, _, _>(send_type, sending_system::<T>, receiving_system::<T>)
    }

    fn add_mapped_client_event<T: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        send_type: impl Into<SendType>,
    ) -> &mut Self {
        self.add_client_event_with::<T, _, _>(
            send_type,
            mapping_and_sending_system::<T>,
            receiving_system::<T>,
        )
    }

    fn add_client_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        send_type: impl Into<SendType>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<NetworkChannels>()
            .create_client_channel(send_type.into());

        self.add_event::<T>()
            .init_resource::<Events<FromClient<T>>>()
            .insert_resource(ClientEventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                (
                    reset_system::<T>.in_set(ClientSet::ResetEvents),
                    receiving_system
                        .in_set(ServerSet::Receive)
                        .run_if(resource_exists::<RenetServer>),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    sending_system.run_if(client_connected),
                    local_resending_system::<T>.run_if(has_authority),
                )
                    .chain()
                    .in_set(ClientSet::Send),
            );

        self
    }
}

fn receiving_system<T: Event + DeserializeOwned>(
    mut client_events: EventWriter<FromClient<T>>,
    mut server: ResMut<RenetServer>,
    channel: Res<ClientEventChannel<T>>,
) {
    for client_id in server.clients_id() {
        while let Some(message) = server.receive_message(client_id, *channel) {
            match DefaultOptions::new().deserialize(&message) {
                Ok(event) => {
                    client_events.send(FromClient { client_id, event });
                }
                Err(e) => debug!("unable to deserialize event from client {client_id}: {e}"),
            }
        }
    }
}

fn sending_system<T: Event + Serialize>(
    mut events: EventReader<T>,
    mut client: ResMut<RenetClient>,
    channel: Res<ClientEventChannel<T>>,
) {
    for event in events.read() {
        let message = DefaultOptions::new()
            .serialize(&event)
            .expect("client event should be serializable");

        client.send_message(*channel, message);
    }
}

fn mapping_and_sending_system<T: Event + MapEntities + Serialize + Clone>(
    mut events: EventReader<T>,
    mut client: ResMut<RenetClient>,
    entity_map: Res<ServerEntityMap>,
    channel: Res<ClientEventChannel<T>>,
) {
    for mut event in events.read().cloned() {
        event.map_entities(&mut EventMapper(entity_map.to_server()));
        let message = DefaultOptions::new()
            .serialize(&event)
            .expect("mapped client event should be serializable");

        client.send_message(*channel, message);
    }
}

/// Transforms `T` events into [`FromClient<T>`] events to "emulate"
/// message sending for offline mode or when server is also a player
fn local_resending_system<T: Event>(
    mut events: ResMut<Events<T>>,
    mut client_events: EventWriter<FromClient<T>>,
) {
    for event in events.drain() {
        client_events.send(FromClient {
            client_id: SERVER_ID,
            event,
        });
    }
}

/// Discards all pending events.
///
/// We discard events while waiting to connect to ensure clean reconnects.
fn reset_system<T: Event>(mut events: ResMut<Events<T>>) {
    let drained_count = events.drain().count();
    if drained_count > 0 {
        warn!("discarded {drained_count} client events due to a disconnect");
    }
}

/// An event indicating that a message from client was received.
/// Emited only on server.
#[derive(Clone, Copy, Event)]
pub struct FromClient<T> {
    pub client_id: ClientId,
    pub event: T,
}
