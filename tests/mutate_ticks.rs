use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::server_mutate_ticks::{MutateTickReceived, ServerMutateTicks},
    prelude::*,
    server::server_tick::ServerTick,
    shared::replication::{
        deferred_entity::DeferredEntity,
        registry::{ctx::WriteCtx, receive_fns},
    },
    test_app::ServerTestAppExt,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn without_changes() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                track_mutate_messages: true,
                ..ServerPlugin::new(PostUpdate)
            }),
        ))
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages_count = client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>()
        .drain()
        .count();
    assert_eq!(
        messages_count, 2,
        "should receive one message for connection and one for the exchange"
    );
}

#[test]
fn one_message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                track_mutate_messages: true,
                ..ServerPlugin::new(PostUpdate)
            }),
        ))
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut received_ticks = client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>();
    assert_eq!(
        received_ticks.drain().count(),
        2,
        "should receive one message for connection and one for spawn"
    );

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    // Clear previous messages.
    client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let mut received_ticks = client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>();
    let [received_tick] = received_ticks
        .drain()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(received_tick.tick, tick);

    let mutate_ticks = client_app.world().resource::<ServerMutateTicks>();
    assert!(mutate_ticks.contains(tick));
    assert_eq!(mutate_ticks.last_confirmed_tick(), Some(tick));
}

#[test]
fn multiple_messages() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                track_mutate_messages: true,
                ..ServerPlugin::new(PostUpdate)
            }),
        ))
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn many entities to cover message splitting.
    const ENTITIES_COUNT: usize = 300;
    server_app
        .world_mut()
        .spawn_batch([(Replicated, BoolComponent(false)); ENTITIES_COUNT]);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), ENTITIES_COUNT);

    let mut received_ticks = client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>();
    assert_eq!(
        received_ticks.drain().count(),
        2,
        "should receive one message for connection and one for spawns"
    );

    for mut component in server_app
        .world_mut()
        .query::<&mut BoolComponent>()
        .iter_mut(server_app.world_mut())
    {
        component.0 = true;
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let mut received_ticks = client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>();
    let [received_tick] = received_ticks
        .drain()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(received_tick.tick, tick);

    let mutate_ticks = client_app.world().resource::<ServerMutateTicks>();
    assert!(mutate_ticks.contains(tick));
    assert_eq!(mutate_ticks.last_confirmed_tick(), Some(tick));
}

#[test]
fn confirmation_follows_mutation_application() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                track_mutate_messages: true,
                ..ServerPlugin::new(PostUpdate)
            }),
        ))
        .replicate::<BoolComponent>()
        .set_receive_fns(
            observe_confirmation,
            receive_fns::default_remove::<BoolComponent>,
        )
        .finish();
    }

    server_app.connect_client(&mut client_app);
    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app
        .world_mut()
        .resource_mut::<Messages<MutateTickReceived>>()
        .clear();

    server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap()
        .0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();
    let confirmation = client_app
        .world_mut()
        .query::<&ConfirmationObserved>()
        .single(client_app.world())
        .unwrap();
    assert!(
        !confirmation.0,
        "the current mutation must not be confirmed while its write function runs"
    );
    assert_eq!(
        client_app
            .world()
            .resource::<ServerMutateTicks>()
            .last_confirmed_tick(),
        Some(tick)
    );
}

#[derive(Clone, Component, Copy, Deserialize, Serialize)]
struct BoolComponent(bool);

#[derive(Component)]
struct ConfirmationObserved(bool);

fn observe_confirmation(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<BoolComponent>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let confirmed = unsafe {
        !entity
            .world_mut()
            .resource::<Messages<MutateTickReceived>>()
            .is_empty()
    };
    receive_fns::default_write(ctx, rule_fns, entity, message)?;
    entity.insert(ConfirmationObserved(confirmed));

    Ok(())
}
