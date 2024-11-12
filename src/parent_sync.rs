use bevy::{
    ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "client")]
use crate::client::ClientSet;
use crate::core::{common_conditions::*, replication::replication_rules::AppRuleExt};
#[cfg(feature = "server")]
use crate::server::ServerSet;

pub struct ParentSyncPlugin;

/// Automatically updates hierarchy on client if [`ParentSync`] component is present on entity.
///
/// This allows to save / replicate hierarchy using only single component.
///
/// If your system runs in [`PreUpdate`] and depends on hierarchies controlled by [`ParentSync`],
/// then you need to run it after [`ClientSet::SyncHierarchy`].
///
/// If your system runs in [`PostUpdate`] and modifies hierarchies with [`ParentSync`],
/// then you need to run it before [`ServerSet::StoreHierarchy`].
impl Plugin for ParentSyncPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ParentSync>()
            .replicate_mapped::<ParentSync>();

        #[cfg(feature = "client")]
        app.add_systems(
            PreUpdate,
            Self::sync_hierarchy.in_set(ClientSet::SyncHierarchy),
        );

        #[cfg(feature = "server")]
        app.add_systems(
            PostUpdate,
            (Self::store_changes, Self::store_removals)
                .run_if(server_or_singleplayer)
                .in_set(ServerSet::StoreHierarchy),
        );
    }
}

impl ParentSyncPlugin {
    /// Synchronizes hierarchy if [`ParentSync`] changes.
    ///
    /// Runs not only on clients, but also on server in order to update the hierarchy when the server state is deserialized.
    #[cfg(feature = "client")]
    fn sync_hierarchy(
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

    #[cfg(feature = "server")]
    fn store_changes(mut hierarchy: Query<(&Parent, &mut ParentSync), Changed<Parent>>) {
        for (parent, mut parent_sync) in &mut hierarchy {
            parent_sync.0 = Some(**parent);
        }
    }

    #[cfg(feature = "server")]
    fn store_removals(
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

/// Replicates parent-children relations for an entity.
///
/// The component captures changes in [`PostUpdate`] on server before sending
/// and applies them on [`PreUpdate`] after receive on clients or scene deserialization.
///
/// # Example
///
/// Replicating two entities and their parent-children relation:
///
/// ```
/// # use bevy::prelude::*;
/// # use bevy_replicon::prelude::*;
/// # let mut world = World::default();
/// # let mut commands = world.commands();
/// commands.spawn(Replicated).with_children(|parent| {
///     parent.spawn((Replicated, ParentSync::default()));
/// });
/// # world.flush();
/// ```
#[derive(Component, Default, Reflect, Clone, Copy, Debug, Serialize, Deserialize)]
#[reflect(Component, MapEntities)]
pub struct ParentSync(Option<Entity>);

impl MapEntities for ParentSync {
    fn map_entities<T: EntityMapper>(&mut self, entity_mapper: &mut T) {
        if let Some(ref mut entity) = self.0 {
            *entity = entity_mapper.map_entity(*entity);
        }
    }
}

#[cfg(all(test, feature = "server", feature = "client"))]
mod tests {
    use bevy::scene::ScenePlugin;

    use super::*;
    use crate::core::RepliconCorePlugin;

    #[test]
    fn update() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let child_entity = app.world_mut().spawn_empty().id();
        app.world_mut().spawn_empty().add_child(child_entity);

        app.add_systems(Update, move |mut commands: Commands| {
            // Should be inserted in `Update` to avoid sync in `PreUpdate`.
            commands.entity(child_entity).insert(ParentSync::default());
        });

        app.update();

        let child_entity = app.world().entity(child_entity);
        let parent = child_entity.get::<Parent>().unwrap();
        let parent_sync = child_entity.get::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }

    #[test]
    fn removal() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let child_entity = app
            .world_mut()
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

        let parent_sync = app.world().get::<ParentSync>(child_entity).unwrap();
        assert!(parent_sync.0.is_none());
    }

    #[test]
    fn update_sync() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let child_entity = app.world_mut().spawn(ParentSync(Some(parent_entity))).id();

        app.update();

        let child_entity = app.world().entity(child_entity);
        let parent = child_entity.get::<Parent>().unwrap();
        let parent_sync = child_entity.get::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }

    #[test]
    fn removal_sync() {
        let mut app = App::new();
        app.add_plugins((RepliconCorePlugin, ParentSyncPlugin));

        let child_entity = app.world_mut().spawn_empty().id();
        app.world_mut().spawn_empty().add_child(child_entity);

        app.update();

        app.world_mut()
            .entity_mut(child_entity)
            .insert(ParentSync::default());

        app.update();

        let child_entity = app.world().entity(child_entity);
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
        scene_world.insert_resource(app.world().resource::<AppTypeRegistry>().clone());
        let parent_entity = scene_world.spawn_empty().id();
        scene_world.spawn(ParentSync(Some(parent_entity)));
        let dynamic_scene = DynamicScene::from_world(&scene_world);

        let mut scenes = app.world_mut().resource_mut::<Assets<DynamicScene>>();
        let scene_handle = scenes.add(dynamic_scene);
        let mut scene_spawner = app.world_mut().resource_mut::<SceneSpawner>();
        scene_spawner.spawn_dynamic(scene_handle);

        app.update();
        app.update();

        let (parent, parent_sync) = app
            .world_mut()
            .query::<(&Parent, &ParentSync)>()
            .single(app.world());
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }
}
