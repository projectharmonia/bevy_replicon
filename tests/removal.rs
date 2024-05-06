use bevy::prelude::*;
use bevy_replicon::{core::replication_fns::command_fns, prelude::*, test_app::ServerTestAppExt};
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

    let server_entity = server_app.world.spawn((Replicated, DummyComponent)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, With<DummyComponent>>()
        .single(&client_app.world);

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
fn command_fns() {
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
        .replicate::<DummyComponent>()
        .set_command_fns(
            command_fns::default_write::<DummyComponent>,
            command_fns::default_remove::<RemovingComponent>,
        );
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replicated, DummyComponent)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, With<DummyComponent>>()
        .single(&client_app.world);

    client_app
        .world
        .entity_mut(client_entity)
        .insert(RemovingComponent);

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<DummyComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<DummyComponent>());
    assert!(!client_entity.contains::<RemovingComponent>());
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
        .replicate::<DummyComponent>()
        .set_marker_fns::<RemoveMarker, DummyComponent>(
            command_fns::default_write,
            command_fns::default_remove::<RemovingComponent>,
        );
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replicated, DummyComponent)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, With<DummyComponent>>()
        .single(&client_app.world);

    client_app
        .world
        .entity_mut(client_entity)
        .insert((RemoveMarker, RemovingComponent));

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<DummyComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<DummyComponent>());
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
        .spawn((Replicated, (GroupComponentA, GroupComponentB)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, (With<GroupComponentA>, With<GroupComponentB>)>()
        .single(&client_app.world);

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
        .spawn((Replicated, NotReplicatedComponent))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, (With<Replicated>, Without<NotReplicatedComponent>)>()
        .single(&client_app.world);

    client_app
        .world
        .entity_mut(client_entity)
        .insert(NotReplicatedComponent);

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

    let server_entity = server_app.world.spawn((Replicated, DummyComponent)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, With<DummyComponent>>()
        .single(&client_app.world);

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

    let server_entity = server_app.world.spawn((Replicated, DummyComponent)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert_eq!(client_app.world.entities().len(), 1);

    // Un-replicate and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .remove::<DummyComponent>()
        .remove::<Replicated>();

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
