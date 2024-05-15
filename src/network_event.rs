pub mod client_event;
pub mod server_event;

use std::{any::Any, fmt::Debug};

use bevy::{
    ecs::{change_detection::MutUntyped, component::ComponentId, entity::EntityHashMap, event},
    prelude::*,
    ptr::{Ptr, PtrMut},
    utils::dbg,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    client_connected, has_authority, server_running, ClientId, ClientSet, FromClient,
    RepliconChannel, RepliconChannels, RepliconClient, RepliconServer, ServerSet,
};

use self::client_event::ClientEventChannel;

/// Maps server entities into client entities inside events.
///
/// Panics if a mapping doesn't exists.
pub struct EventMapper<'a>(pub &'a EntityHashMap<Entity>);

impl EntityMapper for EventMapper<'_> {
    fn map_entity(&mut self, entity: Entity) -> Entity {
        *self
            .0
            .get(&entity)
            .unwrap_or_else(|| panic!("{entity:?} should be mappable"))
    }
}

struct ClientEventFns {
    channel_id: u8,
    send: fn(&mut World, u8),
    resend_locally: fn(&mut World),
    receive: fn(&mut World, u8),
    reset: fn(&mut World),
}

impl ClientEventFns {
    fn new<T: Event + Serialize + DeserializeOwned + Debug>(channel_id: u8) -> Self {
        Self {
            channel_id,
            send: send::<T>,
            resend_locally: resend_locally::<T>,
            receive: receive::<T>,
            reset: reset::<T>,
        }
    }
}

fn send<T: Event + Serialize + Debug>(world: &mut World, channel_id: u8) {
    world.resource_scope(|world, mut client: Mut<RepliconClient>| {
        world.resource_scope(|_, events: Mut<Events<T>>| {
            for event in events.get_reader().read(&events) {
                let message = DefaultOptions::new()
                    .serialize(&event)
                    .expect("mapped client event should be serializable");

                info!("Sending event: {}", std::any::type_name::<T>());
                client.send(channel_id, message)
            }
        });
    })
}

fn resend_locally<T: Event + Serialize + Debug>(world: &mut World) {
    world.resource_scope(|world, mut events: Mut<Events<T>>| {
        world.resource_scope(|world, mut client_events: Mut<Events<FromClient<T>>>| {
            if events.len() > 0 {
                let mapped_events = events.drain().map(|event| FromClient {
                    client_id: ClientId::SERVER,
                    event,
                });
                info!("Resending event: {}", std::any::type_name::<T>());

                client_events.send_batch(mapped_events);
            }
        })
    })
}

fn receive<T: Event + DeserializeOwned + Debug>(world: &mut World, channel_id: u8) {
    world.resource_scope(|world, mut server: Mut<RepliconServer>| {
        world.resource_scope(|world, mut client_events: Mut<Events<FromClient<T>>>| {
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

fn reset<T: Event>(world: &mut World) {
    world.resource_scope(|world, mut events: Mut<Events<T>>| {
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
    fn register_event<T: Event + Serialize + DeserializeOwned + Debug>(&mut self, channel_id: u8) {
        self.events.push(ClientEventFns::new::<T>(channel_id));
    }
}

pub struct NetWorkEventPlugin;

impl Plugin for NetWorkEventPlugin {
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
