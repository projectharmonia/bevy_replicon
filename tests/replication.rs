mod common;

use bevy::prelude::*;
use bevy_replicon::{prelude::*, server};

use bevy_renet::renet::transport::NetcodeClientTransport;
use serde::{Deserialize, Serialize};

#[test]
fn acked_ticks_cleanup() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ));
    }

    common::connect(&mut server_app, &mut client_app);

    let mut client_transport = client_app.world.resource_mut::<NetcodeClientTransport>();
    client_transport.disconnect();
    let client_id = client_transport.client_id();

    client_app.update();
    server_app.update();
    server_app.update();

    let acked_ticks = server_app.world.resource::<AckedTicks>();
    assert!(!acked_ticks.acked_ticks().contains_key(&client_id));
}

#[test]
fn tick_acks_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ));
    }

    common::connect(&mut server_app, &mut client_app);

    client_app.update();
    server_app.update();

    let acked_ticks = server_app.world.resource::<AckedTicks>();
    let client_id = client_app
        .world
        .resource::<NetcodeClientTransport>()
        .client_id();
    let acked_tick = acked_ticks.acked_ticks()[&client_id];
    assert_eq!(acked_tick.get(), 0);
}

#[test]
fn spawn_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ))
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    server_app.world.spawn(Replication);
    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();

    server_app.update();
    client_app.update();

    let client_entity = client_app
        .world
        .query_filtered::<Entity, (With<Replication>, With<TableComponent>)>()
        .single(&client_app.world);
    let entity_map = client_app.world.resource::<NetworkEntityMap>();
    assert_eq!(
        entity_map.to_client().get(&server_entity),
        Some(&client_entity),
        "server entity should be mapped to a replicated entity on client"
    );
    assert_eq!(
        entity_map.to_server().get(&client_entity),
        Some(&server_entity),
        "replicated entity on client should be mapped to a server entity"
    );
    assert_eq!(
        client_app.world.entities().len(),
        1,
        "empty entity shouldn't be replicated"
    );
}

#[test]
fn spawn_prediction_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    // make sure server entity ids don't align with client accidentally..
    server_app.world.spawn(NonReplicatingComponent);
    server_app.world.spawn(NonReplicatingComponent);
    server_app.world.spawn(NonReplicatingComponent);
    // we need the (Replication, MappedComponent) archetype to exist before the client predicted test later,
    // see comment below for more. Archetypes are never removed, so we just need to spawn this entity then
    // remove it, so the archetype exists but the entity doesn't interfere with our testing.
    let tmp_id = server_app
        .world
        .spawn((Replication, MappedComponent(Entity::PLACEHOLDER)))
        .id();
    server_app.world.entity_mut(tmp_id).despawn();

    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ))
        .replicate_mapped::<MappedComponent>()
        .replicate::<Projectile>();
    }

    common::connect(&mut server_app, &mut client_app);

    let client_id = client_app
        .world
        .get_resource::<NetcodeClientTransport>()
        .unwrap()
        .client_id();

    let tick = *server_app.world.get_resource::<RepliconTick>().unwrap();

    // let's pretend the client sent a message to the server saying:
    // "I pressed my [spawn Projectile] button, and predicted the spawn with entity: X"
    let client_predicted_entity = client_app.world.spawn((Projectile, Prediction)).id();
    // so the server spawns in response to a player command..
    let server_entity = server_app.world.spawn((Projectile, Replication)).id();
    // and registers the client's predicted entity
    server_app
        .world
        .resource_scope(|_world, mut pt: Mut<ClientEntityMap>| {
            pt.insert(client_id, server_entity, client_predicted_entity, tick)
        });
    // an edge case to test is when the server spawns an entity that has a predicted entity,
    // and also spawns another entity with a mapped component that references that entity.
    // we need to make sure the mapping works properly at the client.
    // this edge case is triggered when the archetype of the entity with the mapping was created before the
    // archetype of the predicted entity, since replication diffs are built in order of Archetypes iter,
    // which is backed by a vec so the ordering is stable.
    // this is why we ensured the archetype for (Replication, MappedComponent) was created above.
    server_app
        .world
        .spawn((Replication, MappedComponent(server_entity)));
    // the server update which sends this replication data will attach the predicted entity
    server_app.update();
    client_app.update();

    // Repliction component should be inserted for correctly predicted entities
    let client_entity = client_app
        .world
        .query_filtered::<Entity, (With<Projectile>, With<Replication>, With<Prediction>)>()
        .single(&client_app.world);

    let mapped_component = client_app
        .world
        .query_filtered::<&MappedComponent, With<Replication>>()
        .single(&client_app.world);

    // TODO if the mapped component entity was processed before the predicted projectile entity,
    // a new entity would be spawned, because currently the client predicted entity is only sent alongside
    // the server_entity it predicted, so the mapping doesn't exist if a mapped component referencing it
    // is created first.
    //
    // as a result, both of the following two asserts will fail until the bug is fixed:
    //
    assert_eq!(
        mapped_component.0, client_predicted_entity,
        "MappedComponent should point to correctly mapped predicted client entity"
    );

    assert_eq!(
        client_entity, client_predicted_entity,
        "Predicted client entity should match"
    );

    let entity_map = client_app.world.resource::<NetworkEntityMap>();
    assert_eq!(
        entity_map.to_client().get(&server_entity),
        Some(&client_entity),
        "server entity should be mapped to a replicated entity on client"
    );
    assert_eq!(
        entity_map.to_server().get(&client_entity),
        Some(&server_entity),
        "replicated entity on client should be mapped to a server entity"
    );
}

#[test]
fn insert_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();

    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ))
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<IgnoredComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let server_map_entity = server_app.world.spawn_empty().id();
    let client_map_entity = client_app.world.spawn_empty().id();

    let client_entity = client_app.world.spawn(Replication).id();
    let server_entity = server_app
        .world
        .spawn((
            Replication,
            TableComponent,
            SparseSetComponent,
            NonReplicatingComponent,
            MappedComponent(server_map_entity),
            IgnoredComponent,
            Ignored::<IgnoredComponent>::default(),
        ))
        .id();

    let mut entity_map = client_app.world.resource_mut::<NetworkEntityMap>();
    entity_map.insert(server_map_entity, client_map_entity);
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<SparseSetComponent>());
    assert!(client_entity.contains::<TableComponent>());
    assert!(!client_entity.contains::<NonReplicatingComponent>());
    assert!(!client_entity.contains::<IgnoredComponent>());
    assert_eq!(
        client_entity.get::<MappedComponent>().unwrap().0,
        client_map_entity
    );
}

#[test]
fn removal_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ))
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    server_app.update();

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>();

    let client_entity = client_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    client_app
        .world
        .resource_mut::<NetworkEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
    assert!(client_entity.contains::<NonReplicatingComponent>());
}

#[test]
fn despawn_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::EveryFrame)),
        ));
    }

    common::connect(&mut server_app, &mut client_app);

    let server_child_entity = server_app.world.spawn(Replication).id();
    let server_entity = server_app
        .world
        .spawn(Replication)
        .push_children(&[server_child_entity])
        .id();

    server_app.update();

    server_app.world.despawn(server_entity);
    server_app.world.despawn(server_child_entity);

    let client_child_entity = client_app.world.spawn(Replication).id();
    let client_entity = client_app
        .world
        .spawn(Replication)
        .push_children(&[client_child_entity])
        .id();

    let mut entity_map = client_app.world.resource_mut::<NetworkEntityMap>();
    entity_map.insert(server_entity, client_entity);
    entity_map.insert(server_child_entity, client_child_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.get_entity(client_entity).is_none());
    assert!(client_app.world.get_entity(client_child_entity).is_none());

    let entity_map = client_app.world.resource::<NetworkEntityMap>();
    assert!(entity_map.to_client().is_empty());
    assert!(entity_map.to_server().is_empty());
}

#[test]
fn replication_into_scene() {
    let mut app = App::new();
    app.add_plugins(ReplicationPlugins)
        .register_type::<ReflectedComponent>()
        .replicate::<ReflectedComponent>();

    app.world.spawn(ReflectedComponent);
    let reflect_entity = app.world.spawn((Replication, ReflectedComponent)).id();
    let empty_entity = app
        .world
        .spawn((
            Replication,
            ReflectedComponent,
            Ignored::<ReflectedComponent>::default(),
        ))
        .id();

    let mut scene = DynamicScene::default();
    server::replicate_into_scene(&mut scene, &app.world);

    assert!(scene.resources.is_empty());

    let [reflect, empty] = &scene.entities[..] else {
        panic!("scene should only contain entities marked for replication");
    };

    assert_eq!(reflect.entity, reflect_entity);
    assert_eq!(reflect.components.len(), 1);

    assert_eq!(empty.entity, empty_entity);
    assert!(empty.components.is_empty());
}

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapNetworkEntities for MappedComponent {
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T) {
        self.0 = mapper.map(self.0);
    }
}

#[derive(Component, Deserialize, Serialize)]
struct TableComponent;

#[derive(Component, Deserialize, Serialize)]
struct Projectile;

#[derive(Component, Deserialize, Serialize)]
struct Prediction;

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSetComponent;

#[derive(Component)]
struct NonReplicatingComponent;

#[derive(Component, Deserialize, Serialize)]
struct IgnoredComponent;

#[derive(Component, Default, Deserialize, Reflect, Serialize)]
#[reflect(Component)]
struct ReflectedComponent;
