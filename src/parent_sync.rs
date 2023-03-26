use bevy::{
    ecs::{
        entity::{EntityMap, MapEntities, MapEntitiesError},
        reflect::ReflectMapEntities,
    },
    prelude::*,
};

use crate::prelude::{AppReplicationExt, ReflectMapEntity};

pub struct ParentSyncPlugin;

/// Automatically updates hierarchy when [`ParentSync`] is changed.
///
/// This allows to save / replicate hierarchy using only [`ParentSync`] component.
impl Plugin for ParentSyncPlugin {
    fn build(&self, app: &mut App) {
        app.register_and_replicate::<ParentSync>()
            .add_system(Self::parent_sync_system);
    }
}

impl ParentSyncPlugin {
    fn parent_sync_system(
        mut commands: Commands,
        changed_parents: Query<(Entity, &ParentSync), Changed<ParentSync>>,
    ) {
        for (entity, parent) in &changed_parents {
            commands.entity(parent.0).push_children(&[entity]);
        }
    }
}

#[derive(Component, Reflect, Clone, Copy)]
#[reflect(Component, MapEntities, MapEntity)]
pub struct ParentSync(pub Entity);

// We need to impl either [`FromWorld`] or [`Default`] so [`SyncParent`] can be registered as [`Reflect`].
// Same technicue is used in Bevy for [`Parent`]
impl FromWorld for ParentSync {
    fn from_world(_world: &mut World) -> Self {
        Self(Entity::from_raw(u32::MAX))
    }
}

impl MapEntities for ParentSync {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapEntitiesError> {
        self.0 = entity_map.get(self.0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::replication_core::ReplicationCorePlugin;

    use super::*;

    #[test]
    fn entity_mapping() {
        let mut app = App::new();
        app.add_plugin(ReplicationCorePlugin)
            .add_plugin(ParentSyncPlugin);

        let parent = app.world.spawn_empty().id();
        app.world.spawn(ParentSync(parent));

        app.update();

        let (child_parent, parent_sync) = app
            .world
            .query::<(&Parent, &ParentSync)>()
            .single(&app.world);
        assert_eq!(child_parent.get(), parent_sync.0);
    }
}
