use bevy::{
    ecs::{
        entity::{EntityMap, MapEntities, MapEntitiesError},
        reflect::ReflectMapEntities,
    },
    prelude::*,
    scene,
};

use crate::{prelude::ServerSet, AppReplicationExt};

pub struct ParentSyncPlugin;

/// Automatically updates hierarchy on client if [`ParentSync`] component is present on entity.
///
/// This allows to save / replicate hierarchy using only single component.
impl Plugin for ParentSyncPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Option<Entity>>()
            .replicate::<ParentSync>()
            .add_systems(
                (
                    Self::update_system
                        .before(Self::sync_system)
                        .in_set(ServerSet::Authority),
                    Self::removal_system
                        .before(Self::sync_system)
                        .in_set(ServerSet::Authority),
                    Self::sync_system,
                )
                    .after(scene::scene_spawner_system),
            );
    }
}

impl ParentSyncPlugin {
    fn update_system(mut hierarchy: Query<(&Parent, &mut ParentSync), Changed<Parent>>) {
        for (parent, mut parent_sync) in &mut hierarchy {
            parent_sync.0 = Some(**parent);
        }
    }

    fn removal_system(
        mut removed_parents: RemovedComponents<Parent>,
        mut hierarchy: Query<&mut ParentSync>,
    ) {
        for entity in &mut removed_parents {
            if let Ok(mut parent_sync) = hierarchy.get_mut(entity) {
                parent_sync.0 = None;
            }
        }
    }

    /// Synchronizes hierarchy if [`ParentSync`] changes.
    ///
    /// Runs not only on clients, but also on server in order to update the hierarchy when the server state is deserialized.
    fn sync_system(
        mut commands: Commands,
        hierarchy: Query<(Entity, &ParentSync, Option<&Parent>), Changed<ParentSync>>,
    ) {
        for (entity, parent_sync, parent) in &hierarchy {
            if let Some(sync_entity) = parent_sync.0 {
                if parent.filter(|&parent| **parent == sync_entity).is_none() {
                    commands.entity(entity).set_parent(sync_entity);
                }
            } else if parent.is_some() {
                commands.entity(entity).remove_parent();
            }
        }
    }
}

/// Updates entity parent on change.
///
/// Removes the parent if `None`.
#[derive(Component, Default, Reflect, Clone, Copy)]
#[reflect(Component, MapEntities)]
pub struct ParentSync(Option<Entity>);

impl MapEntities for ParentSync {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapEntitiesError> {
        if let Some(ref mut entity) = self.0 {
            *entity = entity_map.get(*entity)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bevy::scene::ScenePlugin;

    use super::*;
    use crate::replication_core::ReplicationCorePlugin;

    #[test]
    fn update() {
        let mut app = App::new();
        app.add_plugin(ReplicationCorePlugin)
            .add_plugin(ParentSyncPlugin);

        let child_entity = app.world.spawn(ParentSync::default()).id();
        app.world.spawn_empty().add_child(child_entity);

        app.update();

        let child_entity = app.world.entity(child_entity);
        let parent = child_entity.get::<Parent>().unwrap();
        let parent_sync = child_entity.get::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }

    #[test]
    fn set_hierarchy() {
        let mut app = App::new();
        app.add_plugin(ReplicationCorePlugin)
            .add_plugin(ParentSyncPlugin);

        let parent_entity = app.world.spawn_empty().id();
        app.world.spawn(ParentSync(Some(parent_entity)));

        app.update();

        let (parent, parent_sync) = app
            .world
            .query::<(&Parent, &ParentSync)>()
            .single(&app.world);
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }

    #[test]
    fn unset_hierarchy() {
        let mut app = App::new();
        app.add_plugin(ReplicationCorePlugin)
            .add_plugin(ParentSyncPlugin);

        let child_entity = app.world.spawn_empty().id();
        app.world.spawn_empty().add_child(child_entity);

        app.update();

        app.world
            .entity_mut(child_entity)
            .insert(ParentSync::default());

        app.update();

        let child_entity = app.world.entity(child_entity);
        assert!(!child_entity.contains::<Parent>());
        assert!(child_entity.get::<ParentSync>().unwrap().0.is_none());
    }

    #[test]
    fn set_scene_hierarchy() {
        let mut app = App::new();
        app.add_plugin(AssetPlugin::default())
            .add_plugin(ScenePlugin)
            .add_plugin(ReplicationCorePlugin)
            .add_plugin(ParentSyncPlugin);

        let mut scene_world = World::new();
        let parent_entity = scene_world.spawn_empty().id();
        scene_world.spawn(ParentSync(Some(parent_entity)));
        let dynamic_scene =
            DynamicScene::from_world(&scene_world, app.world.resource::<AppTypeRegistry>());

        let mut scenes = app.world.resource_mut::<Assets<DynamicScene>>();
        let scene_handle = scenes.add(dynamic_scene);
        let mut scene_spawner = app.world.resource_mut::<SceneSpawner>();
        scene_spawner.spawn_dynamic(scene_handle);

        app.update();

        let (parent, parent_sync) = app
            .world
            .query::<(&Parent, &ParentSync)>()
            .single(&app.world);
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }
}
