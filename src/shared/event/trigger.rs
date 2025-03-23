use bevy::prelude::*;

/// An event that used under the hood for client and server triggers.
///
/// We can't just observe for triggers like we do for events since we need access to all its targets
/// and we need to buffer them. This is why we just emit this event instead and after receive drain it
/// to trigger regular events.
///
/// Traditional trigger interface is provided by [`ClientTriggerExt`](super::client_trigger::ClientTriggerExt)
/// and [`ServerTriggerExt`](super::server_trigger::ServerTriggerExt).
#[derive(Event)]
pub(super) struct RemoteTrigger<E> {
    pub(super) event: E,
    pub(super) targets: Vec<Entity>,
}

/// Like [`TriggerTargets`](bevy::ecs::observer::TriggerTargets), but for remote triggers
/// where targets can only be entities.
pub trait RemoteTargets {
    /// Entities the trigger should target.
    fn into_entities(self) -> Vec<Entity>;
}

impl RemoteTargets for Entity {
    fn into_entities(self) -> Vec<Entity> {
        vec![self]
    }
}

impl RemoteTargets for Vec<Entity> {
    fn into_entities(self) -> Vec<Entity> {
        self
    }
}

impl<const N: usize> RemoteTargets for [Entity; N] {
    fn into_entities(self) -> Vec<Entity> {
        self.to_vec()
    }
}

impl RemoteTargets for &[Entity] {
    fn into_entities(self) -> Vec<Entity> {
        self.to_vec()
    }
}
