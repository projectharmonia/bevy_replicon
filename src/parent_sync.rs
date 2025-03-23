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
#[cfg(feature = "server")]
use crate::server::ServerSet;
use crate::shared::{
    backend::replicon_client::RepliconClient, common_conditions::*,
    replication::replication_rules::AppRuleExt,
};

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
        app.add_systems(PreUpdate, sync_hierarchy.in_set(ClientSet::SyncHierarchy));

        // Trigger on both `Parent` and `ParentSync` to initialize depending on what inserted last.
        #[cfg(feature = "server")]
        app.add_observer(init::<ParentSync>)
            .add_observer(init::<Parent>)
            .add_observer(store_removals)
            .add_systems(
                PostUpdate,
                store_changes
                    .run_if(server_or_singleplayer)
                    .in_set(ServerSet::StoreHierarchy),
            );
    }
}

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
            if parent.is_none_or(|parent| **parent != sync_entity) {
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
        parent_sync.set_if_neq(ParentSync(Some(**parent)));
    }
}

#[cfg(feature = "server")]
fn init<C: Component>(
    trigger: Trigger<OnAdd, C>,
    client: Option<Res<RepliconClient>>,
    mut hierarchy: Query<(&Parent, &mut ParentSync)>,
) {
    if !server_or_singleplayer(client) {
        return;
    }

    if let Ok((parent, mut parent_sync)) = hierarchy.get_mut(trigger.entity()) {
        parent_sync.set_if_neq(ParentSync(Some(**parent)));
    }
}

#[cfg(feature = "server")]
fn store_removals(
    trigger: Trigger<OnRemove, Parent>,
    client: Option<Res<RepliconClient>>,
    mut hierarchy: Query<&mut ParentSync>,
) {
    if !server_or_singleplayer(client) {
        return;
    }

    if let Ok(mut parent_sync) = hierarchy.get_mut(trigger.entity()) {
        parent_sync.0 = None;
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
#[derive(Component, Default, Reflect, Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
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
    use crate::shared::RepliconSharedPlugin;

    #[test]
    fn spawn() {
        let mut app = App::new();
        app.add_plugins((RepliconSharedPlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let child_entity = app
            .world_mut()
            .spawn(ParentSync::default())
            .set_parent(parent_entity)
            .id();

        app.update();

        let child_entity = app.world().entity(child_entity);
        let (parent, parent_sync) = child_entity.components::<(&Parent, &ParentSync)>();
        assert_eq!(**parent, parent_entity);
        assert!(parent_sync.0.is_some_and(|entity| entity == parent_entity));
    }

    #[test]
    fn insertion() {
        let mut app = App::new();
        app.add_plugins((RepliconSharedPlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let child_entity = app
            .world_mut()
            .spawn_empty()
            .set_parent(parent_entity)
            .insert(ParentSync::default())
            .id();

        app.update();

        let child_entity = app.world().entity(child_entity);
        let (parent, parent_sync) = child_entity.components::<(&Parent, &ParentSync)>();
        assert_eq!(**parent, parent_entity);
        assert!(parent_sync.0.is_some_and(|entity| entity == parent_entity));
    }

    #[test]
    fn removal() {
        let mut app = App::new();
        app.add_plugins((RepliconSharedPlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let child_entity = app
            .world_mut()
            .spawn(ParentSync::default())
            .set_parent(parent_entity)
            .id();

        app.update();

        app.world_mut().entity_mut(child_entity).remove_parent();

        let child_entity = app.world().entity(child_entity);
        let (has_parent, parent_sync) = child_entity.components::<(Has<Parent>, &ParentSync)>();
        assert!(!has_parent);
        assert!(parent_sync.0.is_none());
    }

    #[test]
    fn change() {
        let mut app = App::new();
        app.add_plugins((RepliconSharedPlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let child_entity = app
            .world_mut()
            .spawn(ParentSync::default())
            .set_parent(parent_entity)
            .id();

        app.update();

        let new_entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(child_entity)
            .set_parent(new_entity);

        app.update();

        let child_entity = app.world().entity(child_entity);
        let (parent, parent_sync) = child_entity.components::<(&Parent, &ParentSync)>();
        assert_eq!(**parent, new_entity);
        assert!(parent_sync.0.is_some_and(|entity| entity == new_entity));
    }

    #[test]
    fn change_sync() {
        let mut app = App::new();
        app.add_plugins((RepliconSharedPlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let mut child_entity = app.world_mut().spawn(ParentSync::default());

        // Mutate component after the insertion to make it affect the hierarchy.
        let mut parent_sync = child_entity.get_mut::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_none());
        parent_sync.0 = Some(parent_entity);

        let child_entity = child_entity.id();

        app.update();

        let child_entity = app.world().entity(child_entity);
        let (parent, parent_sync) = child_entity.components::<(&Parent, &ParentSync)>();
        assert_eq!(**parent, parent_entity);
        assert!(parent_sync.0.is_some_and(|entity| entity == parent_entity));
    }

    #[test]
    fn removal_sync() {
        let mut app = App::new();
        app.add_plugins((RepliconSharedPlugin, ParentSyncPlugin));

        let parent_entity = app.world_mut().spawn_empty().id();
        let mut child_entity = app.world_mut().spawn_empty();
        child_entity
            .set_parent(parent_entity)
            .insert(ParentSync::default());

        // Mutate component after the insertion to make it affect the hierarchy.
        let mut parent_sync = child_entity.get_mut::<ParentSync>().unwrap();
        assert!(parent_sync.0.is_some_and(|entity| entity == parent_entity));
        parent_sync.0 = None;

        let child_entity = child_entity.id();

        app.update();

        app.world_mut().entity_mut(child_entity).remove_parent();

        let child_entity = app.world().entity(child_entity);
        let (has_parent, parent_sync) = child_entity.components::<(Has<Parent>, &ParentSync)>();
        assert!(!has_parent);
        assert!(parent_sync.0.is_none());
    }

    #[test]
    fn scene_spawn_sync() {
        let mut app = App::new();
        app.add_plugins((
            AssetPlugin::default(),
            ScenePlugin,
            RepliconSharedPlugin,
            ParentSyncPlugin,
        ));

        let mut scene_world = World::new();
        scene_world.insert_resource(app.world().resource::<AppTypeRegistry>().clone());

        app.world_mut().spawn_empty().with_children(|parent| {
            parent.spawn(ParentSync::default());
        });

        let mut scenes = app.world_mut().resource_mut::<Assets<DynamicScene>>();
        let dynamic_scene = DynamicScene::from_world(&scene_world);
        let scene_handle = scenes.add(dynamic_scene);
        let mut scene_spawner = app.world_mut().resource_mut::<SceneSpawner>();
        scene_spawner.spawn_dynamic(scene_handle);

        app.update();

        let (parent, parent_sync) = app
            .world_mut()
            .query::<(&Parent, &ParentSync)>()
            .single(app.world());
        assert!(parent_sync.0.is_some_and(|entity| entity == **parent));
    }
}
