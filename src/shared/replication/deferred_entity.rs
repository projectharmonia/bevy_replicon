use bevy::{ecs::world::CommandQueue, prelude::*};

/// An entity reference that disallows structural ECS changes.
///
/// Similar to [`EntityMut`], but additionally provides a read-only access to the world.
#[derive(Deref, DerefMut)]
pub struct DeferredEntity<'w> {
    #[deref]
    entity: EntityMut<'w>,
    world: &'w World,
}

impl<'w> DeferredEntity<'w> {
    pub(crate) fn new(world: &'w mut World, entity: Entity) -> Self {
        let world_cell = world.as_unsafe_world_cell();
        // SAFETY: access split, `EntityMut` cannot make structural ECS changes,
        // and the world cannot be accessed simultaneously with the entity.
        unsafe {
            let entity: EntityMut = world_cell.world_mut().entity_mut(entity).into();
            let world = world_cell.world();
            Self { entity, world }
        }
    }

    pub(crate) fn commands<'s>(&self, queue: &'s mut CommandQueue) -> Commands<'w, 's> {
        Commands::new_from_entities(queue, self.world.entities())
    }

    /// Gets read-only access to the world that the current entity belongs to.
    pub fn world(&self) -> &World {
        self.world
    }
}
