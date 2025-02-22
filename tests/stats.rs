use bevy::prelude::*;
use bevy_replicon::{
    prelude::*,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};

#[test]
fn client_stats() {
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

    let client_entity = client_app.world_mut().spawn_empty().id();
    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, DummyComponent))
        .id();

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(test_client_entity)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.world_mut().spawn(Replicated).despawn();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .get_mut::<DummyComponent>(server_entity)
        .unwrap()
        .set_changed();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let stats = client_app.world().resource::<ClientReplicationStats>();
    assert_eq!(stats.entities_changed, 2);
    assert_eq!(stats.components_changed, 2);
    assert_eq!(stats.mappings, 1);
    assert_eq!(stats.despawns, 1);
    assert_eq!(stats.messages, 2);
    assert_eq!(stats.bytes, 17);
}

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;
