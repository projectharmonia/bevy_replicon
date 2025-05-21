use bevy::prelude::*;
use bevy_replicon::{
    prelude::*,
    server::server_tick::ServerTick,
    shared::backend::{
        connected_client::{ConnectedClient, NetworkId, NetworkIdMap},
        replicon_channels::ReplicationChannel,
    },
    test_app::ServerTestAppExt,
};

#[test]
fn client_to_server() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();
    }

    const MESSAGES: &[&[u8]] = &[&[0], &[1]];
    let mut client = client_app.world_mut().resource_mut::<RepliconClient>();
    client.set_status(RepliconClientStatus::Connected);
    for &message in MESSAGES {
        client.send(ReplicationChannel::Updates, message);
    }

    let mut server = server_app.world_mut().resource_mut::<RepliconServer>();
    server.set_running(true);

    for (channel_id, message) in client.drain_sent() {
        server.insert_received(Entity::PLACEHOLDER, channel_id, message);
    }

    let messages: Vec<_> = server
        .receive(ReplicationChannel::Updates)
        .map(|(_, message)| message)
        .collect();
    assert_eq!(messages, MESSAGES);
}

#[test]
fn server_to_client() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();
    }

    const MESSAGES: &[&[u8]] = &[&[0], &[1]];
    let mut server = server_app.world_mut().resource_mut::<RepliconServer>();
    server.set_running(true);
    for &message in MESSAGES {
        server.send(Entity::PLACEHOLDER, ReplicationChannel::Updates, message);
    }

    let mut client = client_app.world_mut().resource_mut::<RepliconClient>();
    client.set_status(RepliconClientStatus::Connected);

    for (_, channel_id, message) in server.drain_sent() {
        client.insert_received(channel_id, message);
    }

    let messages: Vec<_> = client.receive(ReplicationChannel::Updates).collect();
    assert_eq!(messages, MESSAGES);
}

#[test]
fn connect_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<ConnectedClient>>();
    let client_entity = clients.single(server_app.world_mut()).unwrap();

    // Assign a placeholder network ID to test network map.
    server_app
        .world_mut()
        .entity_mut(client_entity)
        .insert(NetworkId::new(0));
    assert_eq!(server_app.world().resource::<NetworkIdMap>().len(), 1);

    server_app.disconnect_client(&mut client_app);
    assert_eq!(clients.iter(server_app.world()).len(), 0);
    assert_eq!(server_app.world().resource::<NetworkIdMap>().len(), 0);
}

#[test]
fn client_cleanup_on_disconnect() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();

    let mut client = app.world_mut().resource_mut::<RepliconClient>();
    client.set_status(RepliconClientStatus::Connected);

    client.send(ReplicationChannel::Updates, Vec::new());
    client.insert_received(ReplicationChannel::Updates, Vec::new());

    client.set_status(RepliconClientStatus::Disconnected);

    assert_eq!(client.drain_sent().count(), 0);
    assert_eq!(client.receive(ReplicationChannel::Updates).count(), 0);

    app.update();
}

#[test]
fn server_cleanup_on_stop() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();

    let mut server = app.world_mut().resource_mut::<RepliconServer>();
    server.set_running(true);

    server.send(Entity::PLACEHOLDER, ReplicationChannel::Updates, Vec::new());
    server.insert_received(Entity::PLACEHOLDER, ReplicationChannel::Updates, Vec::new());

    server.set_running(false);

    assert_eq!(server.drain_sent().count(), 0);
    assert_eq!(server.receive(ReplicationChannel::Updates).count(), 0);

    app.update();

    assert_eq!(app.world().resource::<ServerTick>().get(), 0);
}

#[test]
fn client_disconnected() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();

    app.update();

    let mut client = app.world_mut().resource_mut::<RepliconClient>();

    client.send(ReplicationChannel::Updates, Vec::new());
    client.insert_received(ReplicationChannel::Updates, Vec::new());

    assert_eq!(client.drain_sent().count(), 0);
    assert_eq!(client.receive(ReplicationChannel::Updates).count(), 0);

    app.update();
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
    ))
    .finish();

    app.update();

    let mut server = app.world_mut().resource_mut::<RepliconServer>();

    server.send(Entity::PLACEHOLDER, ReplicationChannel::Updates, Vec::new());
    server.insert_received(Entity::PLACEHOLDER, ReplicationChannel::Updates, Vec::new());

    assert_eq!(server.drain_sent().count(), 0);
    assert_eq!(server.receive(ReplicationChannel::Updates).count(), 0);

    app.update();

    assert_eq!(app.world().resource::<ServerTick>().get(), 0);
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
