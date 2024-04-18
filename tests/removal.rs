use bevy::prelude::*;
use bevy_replicon::{
    client::client_mapper::ServerEntityMap, core::replication_fns::command_fns, prelude::*,
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};

#[test]
fn single() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();
    let client_entity = client_app.world.spawn((Replication, DummyComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<DummyComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<DummyComponent>());
}

#[test]
fn marker() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .register_marker::<RemoveMarker>()
        .replicate::<DummyComponent>();

        // SAFETY: `replace` can be safely called with a `SerdeFns` created for `DummyComponent`.
        unsafe {
            app.set_marker_fns::<RemoveMarker, DummyComponent>(
                command_fns::write::<DummyComponent>,
                command_fns::remove::<RemovingComponent>,
            );
        }
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();
    let client_entity = client_app
        .world
        .spawn((Replication, RemoveMarker, RemovingComponent))
        .id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<DummyComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<RemovingComponent>());
}

#[test]
fn group() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate_group::<(GroupComponentA, GroupComponentB)>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, (GroupComponentA, GroupComponentB)))
        .id();

    let client_entity = client_app
        .world
        .spawn((Replication, (GroupComponentA, GroupComponentB)))
        .id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<(GroupComponentA, GroupComponentB)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<GroupComponentA>());
    assert!(!client_entity.contains::<GroupComponentB>());
}

#[test]
fn not_replicated() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ));
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, NotReplicatedComponent))
        .id();

    let client_entity = client_app
        .world
        .spawn((Replication, NotReplicatedComponent))
        .id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<NotReplicatedComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<NotReplicatedComponent>());
}

#[test]
fn after_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();
    let client_entity = client_app.world.spawn((Replication, DummyComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Insert and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .insert(DummyComponent)
        .remove::<DummyComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<DummyComponent>());
}

#[test]
fn with_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();
    let client_entity = client_app.world.spawn((Replication, DummyComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Un-replicate and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .remove::<DummyComponent>()
        .remove::<Replication>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(client_app.world.entities().is_empty());
}

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;

#[derive(Component, Deserialize, Serialize)]
struct GroupComponentA;

#[derive(Component, Deserialize, Serialize)]
struct GroupComponentB;

#[derive(Component, Deserialize, Serialize)]
struct NotReplicatedComponent;

#[derive(Component)]
struct RemoveMarker;

#[derive(Component, Deserialize, Serialize)]
struct RemovingComponent;
