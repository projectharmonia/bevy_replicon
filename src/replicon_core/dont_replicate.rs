use std::{any, marker::PhantomData};

use bevy::{ecs::system::EntityCommands, prelude::*};

pub trait CommandDontReplicateExt {
    /**
    Disables replication for component `T`.

    May only be called on an entity without `T` or if it inserted on it this tick.

    # Panics

    Panics if was called on this entity before or if `T` component was present before this tick.

    # Examples

    ```
    # use bevy::{prelude::*, ecs::system::CommandQueue};
    # use bevy_replicon::prelude::*;
    # let mut world = World::new();
    # let mut queue = CommandQueue::default();
    # let mut commands = Commands::new(&mut queue, &world);
    commands.spawn((Replication, Transform::default())).dont_replicate::<Transform>();
    # queue.apply(&mut world);
    ```
    */
    fn dont_replicate<T: Component>(&mut self) -> &mut Self;
}

impl CommandDontReplicateExt for EntityCommands<'_, '_, '_> {
    fn dont_replicate<T: Component>(&mut self) -> &mut Self {
        self.add(|mut entity: EntityWorldMut| {
            entity.dont_replicate::<T>();
        });

        self
    }
}

pub trait EntityDontReplicateExt {
    /// Same as [`CommandDontReplicateExt::dont_replicate`], but for direct use on an entity.
    fn dont_replicate<T: Component>(&mut self) -> &mut Self;
}

impl EntityDontReplicateExt for EntityWorldMut<'_> {
    fn dont_replicate<T: Component>(&mut self) -> &mut Self {
        if cfg!(debug_assertions) {
            let component_name = any::type_name::<T>();
            assert!(
                !self.contains::<DontReplicate<T>>(),
                "`dont_replicate::<{component_name}>` shouldn't be called twice for the same entity"
            );

            // SAFETY: world is not mutated and used only to obtain the tick without atomic synchronization.
            let tick = unsafe { self.world_mut().change_tick() };
            if let Some(component_ticks) = self.get_change_ticks::<T>() {
                assert_eq!(
                    tick,
                    component_ticks.added_tick(),
                    "disabling replication for `{component_name}` should be done only with its insertion",
                );
            }
        }

        self.insert(DontReplicate::<T>(PhantomData));

        self
    }
}

/// Replication will be ignored for `T` if this component is present on the same entity.
#[derive(Component, Debug)]
pub(super) struct DontReplicate<T>(PhantomData<T>);

#[cfg(test)]
mod tests {
    use bevy::ecs::system::CommandQueue;

    use super::*;

    #[test]
    #[should_panic]
    fn called_twice() {
        let mut world = World::new();

        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands
            .spawn(Transform::default())
            .dont_replicate::<Transform>()
            .dont_replicate::<Transform>();

        queue.apply(&mut world);
    }

    #[test]
    #[should_panic]
    fn after_spawn() {
        let mut world = World::new();

        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let entity = commands.spawn(Transform::default()).id();
        queue.apply(&mut world);

        world.increment_change_tick();

        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands.entity(entity).dont_replicate::<Transform>();
        queue.apply(&mut world);
    }
}
