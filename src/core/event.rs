pub mod client_event;
pub mod client_trigger;
pub mod ctx;
pub mod event_fns;
pub(crate) mod event_registry;
pub mod server_event;
pub mod server_trigger;

use bevy::prelude::*;

/// An event that used under the hood for client and server triggers.
///
/// We can't just observe for triggers like we do for events since we need access to all its targets
/// and we need to buffer them. This is why we just emit this event instead and after receive drain it
/// to trigger regular events.
///
/// Traditional trigger interface is provided by [`ClientTriggerExt`](client_trigger::ClientTriggerExt)
/// and [`ServerTriggerExt`](server_trigger::ServerTriggerExt).
#[derive(Event)]
struct RemoteTrigger<E> {
    event: E,
    targets: Vec<Entity>,
}
