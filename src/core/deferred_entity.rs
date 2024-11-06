use bevy::{ecs::world::unsafe_world_cell::UnsafeWorldCell, prelude::*};

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
    /// Creates a new instance from a world cell.
    ///
    /// # Safety
    ///
    /// - The cell must have been created using [`World::as_unsafe_world_cell`].
    /// - No structural ECS changes can be done using the cell.
    /// - No other mutable references to the entity's components should exist.
    pub(crate) unsafe fn new(world_cell: UnsafeWorldCell<'w>, entity: Entity) -> Self {
        // Split access, `EntityMut` can't make structural changes and they share the lifetime.
        let entity: EntityMut = world_cell.world_mut().entity_mut(entity).into();
        let world = world_cell.world();
        Self { entity, world }
    }

    /// Gets read-only access to the world that the current entity belongs to.
    pub fn world(&self) -> &World {
        self.world
    }
}
