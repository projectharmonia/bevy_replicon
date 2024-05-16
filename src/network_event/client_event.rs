use std::any;

use bevy::{
    ecs::{entity::MapEntities, event::Event},
    prelude::*,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::{EventMapper, ReceiveFn, SendFn};
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
    fn add_client_event_with<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_fn: SendFn,
        receive_fn: ReceiveFn,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with::<T>(channel, send::<T>, receive::<T>)
    }

    fn add_mapped_client_event<T: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: impl Into<RepliconChannel>,
    ) -> &mut Self {
        self.add_client_event_with::<T>(channel, map_and_send::<T>, receive::<T>)
    }

    fn add_client_event_with<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: impl Into<RepliconChannel>,
        send_fn: SendFn,
        receive_fn: ReceiveFn,
    ) -> &mut Self {
        let channel_id = self
            .world
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel.into());

        self.add_event::<T>()
            .init_resource::<Events<FromClient<T>>>();

        self.world
            .resource_mut::<ClientEventRegistry>()
            .register_event::<T>(channel_id, send_fn, receive_fn);

        self
    }
}

struct ClientEventFns {
    channel_id: u8,
    send: SendFn,
    resend_locally: fn(&mut World),
    receive: ReceiveFn,
    reset: fn(&mut World),
}

impl ClientEventFns {
    fn new<T: Event + Serialize + DeserializeOwned>(
        channel_id: u8,
        send: SendFn,
        receive: ReceiveFn,
    ) -> Self {
        Self {
            channel_id,
            send,
            resend_locally: resend_locally::<T>,
            receive,
            reset: reset::<T>,
        }
    }
}

fn send<T: Event + Serialize>(world: &mut World, channel_id: u8) {
    world.resource_scope(|world, mut client: Mut<RepliconClient>| {
        let events = world.resource::<Events<T>>();

        for event in events.get_reader().read(&events) {
            let message = DefaultOptions::new()
                .serialize(&event)
                .expect("mapped client event should be serializable");

            trace!("Sending event: {}", std::any::type_name::<T>());
            client.send(channel_id, message)
        }
    })
}

fn map_and_send<T: Event + MapEntities + Serialize + Clone>(world: &mut World, channel: u8) {
    world.resource_scope(|world, mut client: Mut<RepliconClient>| {
        let entity_map = world.resource::<ServerEntityMap>();
        let events = world.resource::<Events<T>>();

        for mut event in events.get_reader().read(events).cloned() {
            event.map_entities(&mut EventMapper(entity_map.to_server()));
            let message = DefaultOptions::new()
                .serialize(&event)
                .expect("mapped client event should be serializable");

            trace!("sending event `{}`", any::type_name::<T>());
            client.send(channel, message);
        }
    });
}

/// Transforms `T` events into [`FromClient<T>`] events to "emulate"
/// message sending for offline mode or when server is also a player.
fn resend_locally<T: Event + Serialize>(world: &mut World) {
    world.resource_scope(|world, mut events: Mut<Events<T>>| {
        world.resource_scope(|_world, mut client_events: Mut<Events<FromClient<T>>>| {
            if events.len() > 0 {
                let mapped_events = events.drain().map(|event| FromClient {
                    client_id: ClientId::SERVER,
                    event,
                });
                trace!("Resending event: {}", std::any::type_name::<T>());

                client_events.send_batch(mapped_events);
            }
        })
    })
}

fn receive<T: Event + DeserializeOwned>(world: &mut World, channel_id: u8) {
    world.resource_scope(|world, mut server: Mut<RepliconServer>| {
        world.resource_scope(|_world, mut client_events: Mut<Events<FromClient<T>>>| {
            for (client_id, message) in server.receive(channel_id) {
                match DefaultOptions::new().deserialize(&message) {
                    Ok(event) => {
                        trace!(
                            "applying event `{}` from `{client_id:?}`",
                            std::any::type_name::<T>()
                        );
                        client_events.send(FromClient { client_id, event });
                    }
                    Err(e) => debug!("unable to deserialize event from {client_id:?}: {e}"),
                }
            }
        })
    })
}

/// Discards all pending events.
///
/// We discard events while waiting to connect to ensure clean reconnects.
fn reset<T: Event>(world: &mut World) {
    world.resource_scope(|_world, mut events: Mut<Events<T>>| {
        let drained_count = events.drain().count();
        if drained_count > 0 {
            warn!("Discarded {drained_count} client events due to a disconnect");
        }
    })
}

#[derive(Resource, Default)]
struct ClientEventRegistry {
    events: Vec<ClientEventFns>,
}

impl ClientEventRegistry {
    fn register_event<T: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel_id: u8,
        send: SendFn,
        receive: ReceiveFn,
    ) {
        self.events
            .push(ClientEventFns::new::<T>(channel_id, send, receive));
    }
}

pub struct ClientEventPlugin;

impl Plugin for ClientEventPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientEventRegistry>()
            .add_systems(
                PreUpdate,
                (
                    reset_system.in_set(ClientSet::ResetEvents),
                    receive_system
                        .in_set(ServerSet::Receive)
                        .run_if(server_running),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    send_system.run_if(client_connected),
                    resend_locally_system.run_if(has_authority),
                )
                    .chain()
                    .in_set(ClientSet::Send),
            );
    }
}

fn send_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ClientEventRegistry>| {
        for event in &registry.events {
            (event.send)(world, event.channel_id);
        }
    });
}

fn resend_locally_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ClientEventRegistry>| {
        for event in &registry.events {
            (event.resend_locally)(world);
        }
    });
}

fn receive_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ClientEventRegistry>| {
        for event in &registry.events {
            (event.receive)(world, event.channel_id);
        }
    });
}

fn reset_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ClientEventRegistry>| {
        for event in &registry.events {
            (event.reset)(world);
        }
    });
}
/// An event indicating that a message from client was received.
/// Emited only on server.
#[derive(Clone, Copy, Event)]
pub struct FromClient<T> {
    pub client_id: ClientId,
    pub event: T,
}
