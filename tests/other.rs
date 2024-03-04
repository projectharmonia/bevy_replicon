use bevy::prelude::*;
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use serde::{Deserialize, Serialize};

#[test]
fn connect_disconnect() {
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

    let connected_clients = server_app.world.resource::<ConnectedClients>();
    assert_eq!(connected_clients.len(), 1);

    server_app.disconnect_client(&mut client_app);

    let connected_clients = server_app.world.resource::<ConnectedClients>();
    assert!(connected_clients.is_empty());
}

#[test]
fn client_disconnected() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        RepliconPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ));

    app.update();

    let mut client = app.world.resource_mut::<RepliconClient>();

    client.send(ReplicationChannel::Reliable, Vec::new());
    assert_eq!(client.drain_sent().count(), 0);

    client.insert_received(ReplicationChannel::Reliable, Vec::new());
    assert!(client.receive(ReplicationChannel::Reliable).is_none());
}

#[test]
fn server_inactive() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        RepliconPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ));

    app.update();

    let mut server = app.world.resource_mut::<RepliconServer>();

    const DUMMY_CLIENT_ID: ClientId = ClientId::new(1);

    server.send(DUMMY_CLIENT_ID, ReplicationChannel::Reliable, Vec::new());
    assert_eq!(server.drain_sent().count(), 0);

    server.insert_received(DUMMY_CLIENT_ID, ReplicationChannel::Reliable, Vec::new());
    assert_eq!(server.receive(ReplicationChannel::Reliable).count(), 0);
}

#[test]
fn diagnostics() {
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
    client_app.add_plugins(ClientDiagnosticsPlugin);

    server_app.connect_client(&mut client_app);

    let client_entity = client_app.world.spawn_empty().id();
    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();

    let client = client_app.world.resource::<RepliconClient>();
    let client_id = client.id().unwrap();

    let mut entity_map = server_app.world.resource_mut::<ClientEntityMap>();
    entity_map.insert(
        client_id,
        ClientMapping {
            server_entity,
            client_entity,
        },
    );

    server_app.world.spawn(Replication).despawn();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .get_mut::<DummyComponent>(server_entity)
        .unwrap()
        .set_changed();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let stats = client_app.world.resource::<ClientStats>();
    assert_eq!(stats.entities_changed, 2);
    assert_eq!(stats.components_changed, 2);
    assert_eq!(stats.mappings, 1);
    assert_eq!(stats.despawns, 1);
    assert_eq!(stats.packets, 2);
    assert_eq!(stats.bytes, 33);
}

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;
