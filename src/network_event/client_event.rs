use std::{any, marker::PhantomData};

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::EventMapper;
use crate::{
    client::{replicon_client::RepliconClient, server_entity_map::ServerEntityMap, ClientSet},
    core::{
        common_conditions::{client_connected, has_authority, server_running},
        replicon_channels::{RepliconChannel, RepliconChannels},
        ClientId,
    },
    server::{replicon_server::RepliconServer, ServerSet},
};

/// An extension trait for [`App`] for creating client events.
pub trait ClientEventAppExt {
    /// Registers [`FromClient<T>`] event that will be emitted on server after sending `T` event on client.
    ///
    /// For usage example see the [corresponding section](../../index.html#from-client-to-server)
    /// in the quick start guide.
    fn add_client_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self;

    /// Same as [`Self::add_client_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities.
    /// For usage example see the [corresponding section](../../index.html#from-client-to-server)
    /// in the quick start guide.
    fn add_mapped_client_event<T: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self;

    /**
    Same as [`Self::add_client_event`], but uses specified sending and receiving systems.

    It's advised to not panic in the receiving system because it runs on the server.

    # Examples

    Serialize an event with [`Box<dyn Reflect>`]:

    ```
    use bevy::{prelude::*, reflect::serde::{ReflectSerializer, UntypedReflectDeserializer}};
    use bevy_replicon::{network_event::client_event::ClientEventChannel, prelude::*};
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_client_event_with::<ReflectEvent, _, _>(
        ChannelKind::Ordered,
        send_reflect,
        receive_reflect,
    );

    fn send_reflect(
        mut reflect_events: EventReader<ReflectEvent>,
        mut client: ResMut<RepliconClient>,
        channel: Res<ClientEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for event in reflect_events.read() {
            let serializer = ReflectSerializer::new(&*event.0, &registry);
            let message = DefaultOptions::new()
                .serialize(&serializer)
                .expect("client event should be serializable");

            client.send(*channel, message);
        }
    }

    fn receive_reflect(
        mut reflect_events: EventWriter<FromClient<ReflectEvent>>,
        mut server: ResMut<RepliconServer>,
        channel: Res<ClientEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for (client_id, message) in server.receive(*channel) {
            let mut deserializer = bincode::Deserializer::from_slice(&message, DefaultOptions::new());
            match UntypedReflectDeserializer::new(&registry).deserialize(&mut deserializer) {
                Ok(reflect) => {
                    reflect_events.send(FromClient {
                        client_id,
                        event: ReflectEvent(reflect),
                    });
                }
                Err(e) => {
                    debug!("unable to deserialize event from {client_id:?}: {e}")
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
        channel: impl Into<RepliconChannel>,
        send_system: impl IntoSystemConfigs<Marker1>,
        receive_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with::<T, _, _>(channel, send::<T>, receive::<T>)
    }

    fn add_mapped_client_event<T: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with::<T, _, _>(channel, map_and_send::<T>, receive::<T>)
    }

    fn add_client_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_system: impl IntoSystemConfigs<Marker1>,
        receive_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel.into());

        self.add_event::<T>()
            .init_resource::<Events<FromClient<T>>>()
            .insert_resource(ClientEventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                (
                    reset::<T>.in_set(ClientSet::ResetEvents),
                    receive_system
                        .in_set(ServerSet::Receive)
                        .run_if(server_running),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    send_system.run_if(client_connected),
                    resend_locally::<T>.run_if(has_authority),
                )
                    .chain()
                    .in_set(ClientSet::Send),
            );

        self
    }
}

fn receive<T: Event + DeserializeOwned>(
    mut client_events: EventWriter<FromClient<T>>,
    mut server: ResMut<RepliconServer>,
    channel: Res<ClientEventChannel<T>>,
) {
    for (client_id, message) in server.receive(*channel) {
        match DefaultOptions::new().deserialize(&message) {
            Ok(event) => {
                trace!(
                    "applying event `{}` from `{client_id:?}`",
                    any::type_name::<T>()
                );
                client_events.send(FromClient { client_id, event });
            }
            Err(e) => debug!("unable to deserialize event from {client_id:?}: {e}"),
        }
    }
}

fn send<T: Event + Serialize>(
    mut events: EventReader<T>,
    mut client: ResMut<RepliconClient>,
    channel: Res<ClientEventChannel<T>>,
) {
    for event in events.read() {
        let message = DefaultOptions::new()
            .serialize(&event)
            .expect("client event should be serializable");

        trace!("sending event `{}`", any::type_name::<T>());
        client.send(*channel, message);
    }
}

fn map_and_send<T: Event + MapEntities + Serialize + Clone>(
    mut events: EventReader<T>,
    mut client: ResMut<RepliconClient>,
    entity_map: Res<ServerEntityMap>,
    channel: Res<ClientEventChannel<T>>,
) {
    for mut event in events.read().cloned() {
        event.map_entities(&mut EventMapper(entity_map.to_server()));
        let message = DefaultOptions::new()
            .serialize(&event)
            .expect("mapped client event should be serializable");

        trace!("sending event `{}`", any::type_name::<T>());
        client.send(*channel, message);
    }
}

/// Transforms `T` events into [`FromClient<T>`] events to "emulate"
/// message sending for offline mode or when server is also a player.
fn resend_locally<T: Event>(
    mut events: ResMut<Events<T>>,
    mut client_events: EventWriter<FromClient<T>>,
) {
    for event in events.drain() {
        client_events.send(FromClient {
            client_id: ClientId::SERVER,
            event,
        });
    }
}

/// Discards all pending events.
///
/// We discard events while waiting to connect to ensure clean reconnects.
fn reset<T: Event>(mut events: ResMut<Events<T>>) {
    let drained_count = events.drain().count();
    if drained_count > 0 {
        warn!("discarded {drained_count} client events due to a disconnect");
    }
}

/// Holds a client's channel ID for `T`.
#[derive(Resource)]
pub struct ClientEventChannel<T> {
    id: u8,
    marker: PhantomData<T>,
}

impl<T> ClientEventChannel<T> {
    fn new(id: u8) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
}

impl<T> Clone for ClientEventChannel<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ClientEventChannel<T> {}

impl<T> From<ClientEventChannel<T>> for u8 {
    fn from(value: ClientEventChannel<T>) -> Self {
        value.id
    }
}

/// An event indicating that a message from client was received.
/// Emited only on server.
#[derive(Clone, Copy, Event)]
pub struct FromClient<T> {
    pub client_id: ClientId,
    pub event: T,
}
