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
use serde::Serialize;

use crate::{
    client_connected, has_authority, ClientId, ClientSet, FromClient, RepliconChannel,
    RepliconChannels, RepliconClient,
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
    event_component_id: ComponentId,
    channel_component_id: ComponentId,
    from_client_component_id: ComponentId,

    send: fn(&mut RepliconClient, Ptr, Ptr),
    resend_locally: fn(Ptr, Ptr),
}

impl ClientEventFns {
    fn new<T: Event + Serialize + Debug>(
        event_component_id: ComponentId,
        channel_component_id: ComponentId,
        from_client_component_id: ComponentId,
    ) -> Self {
        Self {
            event_component_id,
            channel_component_id,
            from_client_component_id,
            send: send::<T>,
            resend_locally: resend_locally::<T>,
        }
    }
}

fn send<T: Event + Serialize + Debug>(client: &mut RepliconClient, events: Ptr, channel_id: Ptr) {
    unsafe {
        let events = events.deref::<Events<T>>();
        let channel_id = channel_id.deref::<ClientEventChannel<T>>();
        events.get_reader().read(events).for_each(|event| {
            let message = DefaultOptions::new()
                .serialize(&event)
                .expect("mapped client event should be serializable");

            info!("Sending event: {}", std::any::type_name::<T>());
            client.send(*channel_id, message);
        });
    };
}

fn resend_locally<T: Event + Serialize + Debug>(events: Ptr, from_client: Ptr) {
    unsafe {
        let mut events = events.deref::<Events<T>>();

        for event in events.get_reader().read(&events) {
            info!("Resending event: {}", std::any::type_name::<T>());
        }
    }
}

#[derive(Resource, Default)]
struct ClientEventRegistry {
    events: Vec<ClientEventFns>,
}

impl ClientEventRegistry {
    fn register_event<T: Event + Serialize + Debug>(
        &mut self,
        event_component_id: ComponentId,
        channel_component_id: ComponentId,
        from_client_component_id: ComponentId,
    ) {
        self.events.push(ClientEventFns::new::<T>(
            event_component_id,
            channel_component_id,
            from_client_component_id,
        ));
    }
}

pub struct NetWorkEventPlugin;

impl Plugin for NetWorkEventPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientEventRegistry>().add_systems(
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
        world.resource_scope(|world, mut client: Mut<RepliconClient>| {
            for event in &registry.events {
                let Some(untyped_event) = world.get_resource_by_id(event.event_component_id) else {
                    continue;
                };

                let Some(channel) = world.get_resource_by_id(event.channel_component_id) else {
                    continue;
                };

                (event.send)(&mut client, untyped_event, channel);
            }
        })
    })
}

fn resend_locally_system(world: &mut World) {
    world.resource_scope(|world, registry: Mut<ClientEventRegistry>| {
        for event in &registry.events {
            let Some(event_id) = world.get_resource_by_id(event.event_component_id) else {
                continue;
            };

            let Some(from_client_id) = world.get_resource_by_id(event.from_client_component_id)
            else {
                continue;
            };

            (event.resend_locally)(event_id, from_client_id)
        }
    })
}
