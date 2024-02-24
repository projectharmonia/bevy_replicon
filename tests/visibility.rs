mod connect;

use bevy::prelude::*;
use bevy_replicon::{prelude::*, renet::transport::NetcodeClientTransport};
use serde::{Deserialize, Serialize};

#[test]
fn all() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = client_transport.client_id();
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, false); // Shouldn't have any effect for this policy.

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<DummyComponent>)>()
        .single(&client_app.world);

    // Reverse visibility back.
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, true);

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<DummyComponent>)>()
        .single(&client_app.world);
}

#[test]
fn empty_blacklist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                visibility_policy: VisibilityPolicy::Blacklist,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    server_app.world.spawn((Replication, DummyComponent));

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<DummyComponent>)>()
        .single(&client_app.world);
}

#[test]
fn blacklist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                visibility_policy: VisibilityPolicy::Blacklist,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = client_transport.client_id();
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, false);

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());

    // Reverse visibility back.
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, true);

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<DummyComponent>)>()
        .single(&client_app.world);
}

#[test]
fn blacklist_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                visibility_policy: VisibilityPolicy::Blacklist,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn(Replication).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = client_transport.client_id();
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, false);
    server_app.world.despawn(server_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());

    let connected_clients = server_app.world.resource::<ConnectedClients>();
    let visibility = connected_clients.client(client_id).visibility();
    assert!(visibility.is_visible(server_entity)); // The missing entity must be removed from the list, so this should return `true`.
}

#[test]
fn empty_whitelist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                visibility_policy: VisibilityPolicy::Whitelist,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    server_app.world.spawn((Replication, DummyComponent));

    server_app.update();
    client_app.update();

    assert!(
        client_app.world.entities().is_empty(),
        "no entities should be replicated without adding to whitelist"
    );
}

#[test]
fn whitelist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                visibility_policy: VisibilityPolicy::Whitelist,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = client_transport.client_id();
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, true);

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<DummyComponent>)>()
        .single(&client_app.world);

    // Reverse visibility.
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, false);

    server_app.update();
    client_app.update();

    assert!(
        client_app.world.entities().is_empty(),
        "entity should be despawned after removing from whitelist"
    );
}

#[test]
fn whitelist_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                visibility_policy: VisibilityPolicy::Whitelist,
                ..Default::default()
            }),
        ))
        .replicate::<DummyComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn(Replication).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = client_transport.client_id();
    let mut connected_clients = server_app.world.resource_mut::<ConnectedClients>();
    let visibility = connected_clients.client_mut(client_id).visibility_mut();
    visibility.set_visibility(server_entity, true);
    server_app.world.despawn(server_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());

    let connected_clients = server_app.world.resource::<ConnectedClients>();
    let visibility = connected_clients.client(client_id).visibility();
    assert!(!visibility.is_visible(server_entity));
}

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;
