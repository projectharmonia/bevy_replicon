use bevy::{prelude::*, state::app::StatesPlugin};
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
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
        app.update();
    }

    const MESSAGES: &[&[u8]] = &[&[0], &[1]];
    let mut client = client_app.world_mut().resource_mut::<RepliconClient>();
    for &message in MESSAGES {
        client.send(ReplicationChannel::Updates, message);
    }

    let mut server = server_app.world_mut().resource_mut::<RepliconServer>();
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
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
        app.update();
    }

    const MESSAGES: &[&[u8]] = &[&[0], &[1]];
    let mut server = server_app.world_mut().resource_mut::<RepliconServer>();
    for &message in MESSAGES {
        server.send(Entity::PLACEHOLDER, ReplicationChannel::Updates, message);
    }

    let mut client = client_app.world_mut().resource_mut::<RepliconClient>();
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
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
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
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));

    app.update();

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::Connected);

    app.world_mut().run_schedule(StateTransition);

    let mut client = app.world_mut().resource_mut::<RepliconClient>();
    client.send(ReplicationChannel::Updates, Vec::new());
    client.insert_received(ReplicationChannel::Updates, Vec::new());

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::Disconnected);

    app.world_mut().run_schedule(StateTransition);

    let mut client = app.world_mut().resource_mut::<RepliconClient>();
    assert_eq!(client.drain_sent().count(), 0);
    assert_eq!(client.receive(ReplicationChannel::Updates).count(), 0);
}

#[test]
fn server_cleanup_on_stop() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));

    app.update();

    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);

    app.world_mut().run_schedule(StateTransition);

    let mut server = app.world_mut().resource_mut::<RepliconServer>();
    server.send(Entity::PLACEHOLDER, ReplicationChannel::Updates, Vec::new());
    server.insert_received(Entity::PLACEHOLDER, ReplicationChannel::Updates, Vec::new());

    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Stopped);

    app.world_mut().run_schedule(StateTransition);

    let mut server = app.world_mut().resource_mut::<RepliconServer>();
    assert_eq!(server.drain_sent().count(), 0);
    assert_eq!(server.receive(ReplicationChannel::Updates).count(), 0);
    assert_eq!(app.world().resource::<ServerTick>().get(), 0);
}

#[test]
fn deferred_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                replicate_after_connect: false,
                ..Default::default()
            }),
        ));
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<&ConnectedClient, Without<ReplicatedClient>>();
    assert_eq!(clients.iter(server_app.world()).count(), 1);
}
