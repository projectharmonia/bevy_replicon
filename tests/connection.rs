use bevy::prelude::*;
use bevy_replicon::{
    prelude::*,
    server::server_tick::ServerTick,
    shared::backend::connected_client::{ConnectedClient, NetworkId, NetworkIdMap},
    test_app::ServerTestAppExt,
};

#[test]
fn client_connect_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<ConnectedClient>>();
    assert_eq!(clients.iter(server_app.world()).len(), 1);

    server_app.disconnect_client(&mut client_app);

    assert_eq!(clients.iter(server_app.world()).len(), 0);
}

#[test]
fn server_start_stop() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                replicate_after_connect: false,
                ..Default::default()
            }),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<ConnectedClient>>();
    assert_eq!(clients.iter(server_app.world()).len(), 1);
    assert_ne!(server_app.world().resource::<ServerTick>().get(), 0);

    server_app
        .world_mut()
        .resource_mut::<RepliconServer>()
        .set_running(false);
    server_app.update();

    assert_eq!(clients.iter(server_app.world()).len(), 0);
    assert_eq!(server_app.world().resource::<ServerTick>().get(), 0);
}

#[test]
fn network_id_map() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();

    let client_entity = app.world_mut().spawn(NetworkId::new(0)).id();
    assert_eq!(app.world().resource::<NetworkIdMap>().len(), 1);

    app.world_mut().despawn(client_entity);
    assert!(app.world().resource::<NetworkIdMap>().is_empty());
}

#[test]
fn deferred_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                replicate_after_connect: false,
                ..Default::default()
            }),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<&ConnectedClient, Without<ReplicatedClient>>();
    assert_eq!(clients.iter(server_app.world()).count(), 1);
}
