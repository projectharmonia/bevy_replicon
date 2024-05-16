pub mod client_event;
pub mod server_event;

use bevy::{ecs::entity::EntityHashMap, prelude::*};

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

pub type SendFn = fn(&mut World, u8);
pub type ReceiveFn = fn(&mut World, u8);

struct NetworkEventFns {
    channel_id: u8,
    send: SendFn,
    resend_locally: fn(&mut World),
    receive: ReceiveFn,
    reset: fn(&mut World),
}
