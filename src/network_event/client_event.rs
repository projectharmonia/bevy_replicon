use std::marker::PhantomData;

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::EventMapper;
use crate::{
    client::{client_mapper::ServerEntityMap, replicon_client::RepliconClient, ClientSet},
    core::{
        common_conditions::{connected, no_connection, server_active},
        replicon_channels::{RepliconChannel, RepliconChannels},
        PeerId,
    },
    server::{replicon_server::RepliconServer, ServerSet},
    ConnectedClients,
};

/// An extension trait for [`App`] for creating client events.
pub trait ClientEventAppExt {
    /// Registers [`FromPeer<T>`] event that will be emitted on server after sending `T` event on client.
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

    Serialize an event with `Box<dyn Reflect>`:

    ```
    use bevy::{prelude::*, reflect::serde::{ReflectSerializer, UntypedReflectDeserializer}};
    use bevy_replicon::prelude::*;
    use bincode::{DefaultOptions, Options};
    use serde::de::DeserializeSeed;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_client_event_with::<ReflectEvent, _, _>(
        ChannelKind::Ordered,
        sending_reflect_system,
        receiving_reflect_system,
    );

    fn sending_reflect_system(
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

    fn receiving_reflect_system(
        mut reflect_events: EventWriter<FromPeer<ReflectEvent>>,
        mut server: ResMut<RepliconServer>,
        connected_clients: Res<ConnectedClients>,
        channel: Res<ClientEventChannel<ReflectEvent>>,
        registry: Res<AppTypeRegistry>,
    ) {
        let registry = registry.read();
        for peer_id in connected_clients.iter_peer_ids() {
            while let Some(message) = server.receive(peer_id, *channel) {
                let mut deserializer =
                    bincode::Deserializer::from_slice(&message, DefaultOptions::new());
                match UntypedReflectDeserializer::new(&registry).deserialize(&mut deserializer) {
                    Ok(reflect) => {
                        reflect_events.send(FromPeer {
                            peer_id,
                            event: ReflectEvent(reflect),
                        });
                    }
                    Err(e) => {
                        debug!("unable to deserialize event from {peer_id:?}: {e}")
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
        channel: impl Into<RepliconChannel>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with::<T, _, _>(channel, sending_system::<T>, receiving_system::<T>)
    }

    fn add_mapped_client_event<T: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with::<T, _, _>(
            channel,
            mapping_and_sending_system::<T>,
            receiving_system::<T>,
        )
    }

    fn add_client_event_with<T: Event, Marker1, Marker2>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        sending_system: impl IntoSystemConfigs<Marker1>,
        receiving_system: impl IntoSystemConfigs<Marker2>,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel.into());

        self.add_event::<T>()
            .init_resource::<Events<FromPeer<T>>>()
            .insert_resource(ClientEventChannel::<T>::new(channel_id))
            .add_systems(
                PreUpdate,
                (
                    reset_system::<T>.in_set(ClientSet::ResetEvents),
                    receiving_system
                        .in_set(ServerSet::Receive)
                        .run_if(server_active),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    sending_system.run_if(connected),
                    local_resending_system::<T>.run_if(no_connection),
                )
                    .chain()
                    .in_set(ClientSet::Send),
            );

        self
    }
}

fn receiving_system<T: Event + DeserializeOwned>(
    mut client_events: EventWriter<FromPeer<T>>,
    connected_clients: Res<ConnectedClients>,
    mut server: ResMut<RepliconServer>,
    channel: Res<ClientEventChannel<T>>,
) {
    for peer_id in connected_clients.iter_peer_ids() {
        while let Some(message) = server.receive(peer_id, *channel) {
            match DefaultOptions::new().deserialize(&message) {
                Ok(event) => {
                    client_events.send(FromPeer { peer_id, event });
                }
                Err(e) => debug!("unable to deserialize event from {peer_id:?}: {e}"),
            }
        }
    }
}

fn sending_system<T: Event + Serialize>(
    mut events: EventReader<T>,
    mut client: ResMut<RepliconClient>,
    channel: Res<ClientEventChannel<T>>,
) {
    for event in events.read() {
        let message = DefaultOptions::new()
            .serialize(&event)
            .expect("client event should be serializable");

        client.send(*channel, message);
    }
}

fn mapping_and_sending_system<T: Event + MapEntities + Serialize + Clone>(
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

        client.send(*channel, message);
    }
}

/// Transforms `T` events into [`FromPeer<T>`] events to "emulate"
/// message sending for offline mode or when server is also a player.
fn local_resending_system<T: Event>(
    mut events: ResMut<Events<T>>,
    mut client_events: EventWriter<FromPeer<T>>,
) {
    for event in events.drain() {
        client_events.send(FromPeer {
            peer_id: PeerId::SERVER,
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
pub struct FromPeer<T> {
    pub peer_id: PeerId,
    pub event: T,
}
