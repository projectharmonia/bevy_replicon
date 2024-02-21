mod connect;

use bevy::prelude::*;
use bevy_renet::renet::transport::NetcodeClientTransport;
use bevy_replicon::{prelude::*, scene};
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
        .spawn((Replication, ReflectedComponent))
        .dont_replicate::<ReflectedComponent>()
        .id();

    let mut scene = DynamicScene::default();
    scene::replicate_into(&mut scene, &app.world);

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
struct DummyComponent;

#[derive(Component, Default, Deserialize, Reflect, Serialize)]
#[reflect(Component)]
struct ReflectedComponent;
