mod connect;

use bevy::prelude::*;
use bevy_renet::renet::transport::NetcodeClientTransport;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

#[test]
fn reset() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ));
    }

    connect::single_client(&mut server_app, &mut client_app);

    client_app.world.resource_mut::<RenetClient>().disconnect();

    client_app.update();
    server_app.update();

    client_app.update();
    server_app.update();

    client_app.world.remove_resource::<RenetClient>();
    server_app.world.remove_resource::<RenetServer>();

    server_app.update();
    client_app.update();

    assert_eq!(server_app.world.resource::<RepliconTick>().get(), 0);
    assert_eq!(client_app.world.resource::<RepliconTick>().get(), 0);
}

#[test]
fn diagnostics() {
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
    client_app.add_plugins(ClientDiagnosticsPlugin);

    connect::single_client(&mut server_app, &mut client_app);

    let client_entity = client_app.world.spawn_empty().id();
    let server_entity = server_app.world.spawn((Replication, DummyComponent)).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = client_transport.client_id();
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
    client_app.update();

    server_app
        .world
        .get_mut::<DummyComponent>(server_entity)
        .unwrap()
        .set_changed();

    server_app.update();
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
