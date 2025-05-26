use bevy::prelude::*;

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
