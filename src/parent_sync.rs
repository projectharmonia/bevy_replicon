use bevy::{
    ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::{
    client::ClientSet,
    replicon_core::replication_rules::AppReplicationExt,
    server::{has_authority, ServerSet},
};

pub struct ParentSyncPlugin;

/// Automatically updates hierarchy on client if [`ParentSync`] component is present on entity.
///
/// This allows to save / replicate hierarchy using only single component.
impl Plugin for ParentSyncPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Option<Entity>>()
            .register_type::<ParentSync>()
            .replicate_mapped::<ParentSync>()
            .add_systems(PreUpdate, Self::sync_system.after(ClientSet::Receive))
            .add_systems(
                PostUpdate,
                (Self::update_system, Self::removal_system)
                    .run_if(has_authority)
                    .before(ServerSet::Send),
            );
    }
}

impl ParentSyncPlugin {
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

    fn update_system(mut hierarchy: Query<(&Parent, &mut ParentSync), Changed<Parent>>) {
        for (parent, mut parent_sync) in &mut hierarchy {
            parent_sync.0 = Some(**parent);
        }
    }

    fn removal_system(
        mut removed_parents: RemovedComponents<Parent>,
        mut hierarchy: Query<&mut ParentSync>,
    ) {
        for entity in removed_parents.read() {
            if let Ok(mut parent_sync) = hierarchy.get_mut(entity) {
                parent_sync.0 = None;
            }
        }
    }
}

/// Updates entity parent on change.
///
/// Removes the parent if `None`.
/// The component captures changes in `PostUpdate` on server before sending
/// and applies them on `PreUpdate` after receive on clients or scene deserialization.
#[derive(Component, Default, Reflect, Clone, Copy, Serialize, Deserialize)]
#[reflect(Component, MapEntities)]
pub struct ParentSync(Option<Entity>);

impl MapEntities for ParentSync {
    fn map_entities<T: EntityMapper>(&mut self, entity_mapper: &mut T) {
        if let Some(ref mut entity) = self.0 {
            *entity = entity_mapper.map_entity(*entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::scene::ScenePlugin;

    use super::*;
    use crate::replicon_core::RepliconCorePlugin;

    #[test]
    fn update() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let child_entity = app.world.spawn_empty().id();
        app.world.spawn_empty().add_child(child_entity);

        app.add_systems(Update, move |mut commands: Commands| {
            // Should be inserted in `Update` to avoid sync in `PreUpdate`.
            commands.entity(child_entity).insert(ParentSync::default());
        });

        app.update();

        let child_entity = app.world.entity(child_entity);
        let parent = child_entity.get::<Parent>().unwrap();
        let parent_sync = child_entity.get::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }

    #[test]
    fn removal() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let parent_entity = app.world.spawn_empty().id();
        let child_entity = app
            .world
            .spawn_empty()
            .set_parent(parent_entity)
            .remove_parent()
            .id();

        app.add_systems(Update, move |mut commands: Commands| {
            // Should be inserted in `Update` to avoid sync in `PreUpdate`.
            commands
                .entity(child_entity)
                .insert(ParentSync(Some(parent_entity)));
        });

        app.update();

        let parent_sync = app.world.get::<ParentSync>(child_entity).unwrap();
        assert!(parent_sync.0.is_none());
    }

    #[test]
    fn update_sync() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let parent_entity = app.world.spawn_empty().id();
        let child_entity = app.world.spawn(ParentSync(Some(parent_entity))).id();

        app.update();

        let child_entity = app.world.entity(child_entity);
        let parent = child_entity.get::<Parent>().unwrap();
        let parent_sync = child_entity.get::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }

    #[test]
    fn removal_sync() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

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
    fn scene_update_sync() {
        let mut app = App::new();
        app.add_plugins((
            AssetPlugin::default(),
            ScenePlugin,
            RepliconCorePlugin,
            ParentSyncPlugin,
        ));

        let mut scene_world = World::new();
        scene_world.insert_resource(app.world.resource::<AppTypeRegistry>().clone());
        let parent_entity = scene_world.spawn_empty().id();
        scene_world.spawn(ParentSync(Some(parent_entity)));
        let dynamic_scene = DynamicScene::from_world(&scene_world);

        let mut scenes = app.world.resource_mut::<Assets<DynamicScene>>();
        let scene_handle = scenes.add(dynamic_scene);
        let mut scene_spawner = app.world.resource_mut::<SceneSpawner>();
        scene_spawner.spawn_dynamic(scene_handle.clone()); // Needs to be cloned to avoid dropping: https://github.com/bevyengine/bevy/issues/10482.

        app.update();
        app.update();

        let (parent, parent_sync) = app
            .world
            .query::<(&Parent, &ParentSync)>()
            .single(&app.world);
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }
}
