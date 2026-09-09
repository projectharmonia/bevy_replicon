use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::UserdataReceived,
    prelude::*,
    server::ReplicationUserdata,
    shared::backend::channels::{ClientChannel, ServerChannel},
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn update_message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .add_observer(receive_userdata)
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&USERDATA.to_le_bytes());

    server_app.world_mut().spawn((Replicated, TestComponent));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<ClientMessages>();
    assert_eq!(messages.received_count(ServerChannel::Updates), 1);
    assert_eq!(messages.received_count(ServerChannel::Mutations), 0);

    client_app.update();

    let received = client_app.world().resource::<ReceivedUserdata>();
    assert_eq!(received.0, USERDATA);
}

#[test]
fn mutate_message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .add_observer(receive_userdata)
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, TestComponent))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut component = server_app
        .world_mut()
        .get_mut::<TestComponent>(server_entity)
        .unwrap();
    component.set_changed();

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&USERDATA.to_le_bytes());

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<ClientMessages>();
    assert_eq!(messages.received_count(ServerChannel::Updates), 0);
    assert_eq!(messages.received_count(ServerChannel::Mutations), 1);

    client_app.update();

    let received = client_app.world().resource::<ReceivedUserdata>();
    assert_eq!(received.0, USERDATA);
}

#[test]
fn deferred_update_blocks_later_updates() {
    let (mut server_app, mut client_app) = create_apps();
    client_app.world_mut().resource_mut::<ReadyUserdata>().0 = 1;
    server_app.connect_client(&mut client_app);

    set_userdata(&mut server_app, 2);
    server_app.world_mut().spawn((Replicated, TestComponent));
    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    set_userdata(&mut server_app, 1);
    server_app.world_mut().spawn((Replicated, TestComponent));
    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert_eq!(
        client_app
            .world()
            .resource::<ClientMessages>()
            .received_count(ServerChannel::Updates),
        2
    );
    client_app.update();

    assert_eq!(
        remote_count(&mut client_app),
        0,
        "the ready second update must not pass the deferred first update"
    );

    client_app.world_mut().resource_mut::<ReadyUserdata>().0 = 2;
    client_app.update();

    assert_eq!(remote_count(&mut client_app), 2);
    assert_eq!(
        client_app.world().resource::<ReceivedUserdata>().0,
        1,
        "buffered updates should be applied in wire order"
    );
}

#[test]
fn deferred_updates_cleared_on_disconnect() {
    let (mut server_app, mut client_app) = create_apps();
    server_app.connect_client(&mut client_app);

    // Defer a spawn; it stays buffered and unapplied.
    set_userdata(&mut server_app, USERDATA);
    server_app.world_mut().spawn((Replicated, TestComponent));
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    assert_eq!(remote_count(&mut client_app), 0);

    // Disconnect runs the client reset, which must drop the deferred message.
    server_app.disconnect_client(&mut client_app);
    assert_eq!(remote_count(&mut client_app), 0);

    // Reconnect with a policy that would still defer the stale message (`USERDATA` > 1).
    // If it survived, it would head-of-line block the fresh state. The userdata is
    // set before the handshake so handshake updates don't re-attach the stale value.
    client_app.world_mut().resource_mut::<ReadyUserdata>().0 = 1;
    set_userdata(&mut server_app, 1);
    server_app.connect_client(&mut client_app);
    server_app.world_mut().spawn((Replicated, TestComponent));
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(
        remote_count(&mut client_app),
        2,
        "stale deferred updates must not survive disconnect"
    );
    assert_eq!(
        client_app.world().resource::<ReceivedUserdata>().0,
        1,
        "only fresh userdata should be applied after reconnect"
    );
}

#[test]
fn deferred_mutation_is_acked_before_application() {
    let (mut server_app, mut client_app) = create_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, ValueComponent(0)))
        .id();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .get_mut::<ValueComponent>(server_entity)
        .unwrap()
        .0 = 1;
    set_userdata(&mut server_app, USERDATA);
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(client_value(&mut client_app), 0);
    assert_eq!(client_app.world().resource::<ReceivedUserdata>().0, 0);
    assert!(
        client_app
            .world()
            .resource::<ClientMessages>()
            .iter_sent()
            .any(
                |(channel, bytes)| channel == usize::from(ClientChannel::MutationAcks)
                    && !bytes.is_empty()
            ),
        "a successfully retained mutation should be acknowledged before application"
    );

    client_app.world_mut().resource_mut::<ReadyUserdata>().0 = USERDATA;
    client_app.update();

    assert_eq!(client_value(&mut client_app), 1);
    assert_eq!(
        client_app.world().resource::<ReceivedUserdata>().0,
        USERDATA
    );
}

#[test]
fn deferred_update_holds_back_mutation() {
    let (mut server_app, mut client_app) = create_apps();
    client_app.world_mut().resource_mut::<ReadyUserdata>().0 = 1;
    server_app.connect_client(&mut client_app);

    // Defer the spawn update; it stays buffered and unapplied.
    set_userdata(&mut server_app, 2);
    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, ValueComponent(0)))
        .id();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    assert_eq!(remote_count(&mut client_app), 0);

    // The observer accepts this mutation's userdata, but its update is still deferred,
    // so it must stay buffered until the update is applied.
    set_userdata(&mut server_app, 1);
    server_app
        .world_mut()
        .get_mut::<ValueComponent>(server_entity)
        .unwrap()
        .0 = 1;
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(
        remote_count(&mut client_app),
        0,
        "a mutation must not pass its deferred update"
    );
    assert_eq!(
        client_app.world().resource::<ReceivedUserdata>().0,
        0,
        "nothing should be applied while the update is deferred"
    );
    assert!(
        client_app
            .world()
            .resource::<ClientMessages>()
            .iter_sent()
            .any(
                |(channel, bytes)| channel == usize::from(ClientChannel::MutationAcks)
                    && !bytes.is_empty()
            ),
        "a retained mutation should be acknowledged even while waiting for its update"
    );

    client_app.world_mut().resource_mut::<ReadyUserdata>().0 = 2;
    client_app.update();

    assert_eq!(remote_count(&mut client_app), 1);
    assert_eq!(client_value(&mut client_app), 1);
    assert_eq!(
        client_app.world().resource::<ReceivedUserdata>().0,
        1,
        "messages should be applied in wire order once the update is ready"
    );
}

fn create_apps() -> (App, App) {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .init_resource::<ReadyUserdata>()
        .add_observer(receive_userdata)
        .replicate::<TestComponent>()
        .replicate::<ValueComponent>()
        .finish();
    }
    client_app.add_observer(apply_when_ready);

    (server_app, client_app)
}

fn set_userdata(app: &mut App, value: u32) {
    let mut userdata = app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.clear();
    userdata.extend_from_slice(&value.to_le_bytes());
}

fn apply_when_ready(mut trigger: On<ShouldApplyReplication>, ready: Res<ReadyUserdata>) {
    let Some(userdata) = trigger.userdata.as_ref() else {
        return;
    };
    let value = u32::from_le_bytes(
        userdata
            .as_ref()
            .try_into()
            .expect("test userdata should be a u32"),
    );
    if value > ready.0 {
        trigger.should_apply = false;
    }
}

fn remote_count(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<Remote>>()
        .iter(app.world())
        .count()
}

fn client_value(app: &mut App) -> u32 {
    app.world_mut()
        .query::<&ValueComponent>()
        .single(app.world())
        .unwrap()
        .0
}

const USERDATA: u32 = 42;

#[derive(Component, Deserialize, Serialize)]
struct TestComponent;

#[derive(Component, Deserialize, Serialize)]
struct ValueComponent(u32);

#[derive(Resource, Default)]
struct ReceivedUserdata(u32);

#[derive(Resource, Default)]
struct ReadyUserdata(u32);

fn receive_userdata(received: On<UserdataReceived>, mut storage: ResMut<ReceivedUserdata>) {
    let bytes = received.bytes.as_ref().try_into().unwrap();
    storage.0 = u32::from_le_bytes(bytes);
}
