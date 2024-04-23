use std::io::Cursor;

use bevy::{prelude::*, utils::Duration};
use bevy_replicon::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::{
        replication_fns::{command_fns, rule_fns::RuleFns},
        replicon_tick::RepliconTick,
    },
    prelude::*,
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};

#[test]
fn small_component() {
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
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value.
    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world
        .query::<&BoolComponent>()
        .single(&client_app.world);
    assert!(component.0, "changed value should be updated on client");
}

#[test]
fn package_size_component() {
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
        .replicate::<VecComponent>();
    }

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, VecComponent::default()))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // To exceed packed size.
    const BIG_DATA: &[u8] = &[0; 1200];
    let mut component = server_app
        .world
        .get_mut::<VecComponent>(server_entity)
        .unwrap();
    component.0 = BIG_DATA.to_vec();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world
        .query::<&VecComponent>()
        .single(&client_app.world);
    assert_eq!(component.0, BIG_DATA);
}

#[test]
fn command_fns() {
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
        .replicate::<OriginalComponent>()
        .set_command_fns(replace, command_fns::default_remove::<ReplacedComponent>);
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, OriginalComponent(false)))
        .id();

    let client_entity = client_app
        .world
        .spawn((Replication, ReplacedComponent(false)))
        .id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value.
    let mut component = server_app
        .world
        .get_mut::<OriginalComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<OriginalComponent>());

    let component = client_entity.get::<ReplacedComponent>().unwrap();
    assert!(component.0);
}

#[test]
fn marker() {
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
        .register_marker::<ReplaceMarker>()
        .replicate::<OriginalComponent>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, OriginalComponent(false)))
        .id();

    let client_entity = client_app
        .world
        .spawn((Replication, ReplaceMarker, ReplacedComponent(false)))
        .id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value.
    let mut component = server_app
        .world
        .get_mut::<OriginalComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<OriginalComponent>());

    let component = client_entity.get::<ReplacedComponent>().unwrap();
    assert!(component.0);
}

#[test]
fn many_entities() {
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
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    // Spawn many entities to cover message splitting.
    const ENTITIES_COUNT: u32 = 300;
    server_app
        .world
        .spawn_batch([(Replication, BoolComponent(false)); ENTITIES_COUNT as usize]);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert_eq!(client_app.world.entities().len(), ENTITIES_COUNT);

    for mut component in server_app
        .world
        .query::<&mut BoolComponent>()
        .iter_mut(&mut server_app.world)
    {
        component.0 = true;
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    for component in client_app
        .world
        .query::<&BoolComponent>()
        .iter(&client_app.world)
    {
        assert!(component.0);
    }
}

#[test]
fn with_insertion() {
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
        .replicate::<BoolComponent>()
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut server_entity = server_app.world.entity_mut(server_entity);
    server_entity.get_mut::<BoolComponent>().unwrap().0 = true;
    server_entity.insert(DummyComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world
        .query_filtered::<&BoolComponent, With<DummyComponent>>()
        .single(&client_app.world);
    assert!(component.0);
}

#[test]
fn with_despawn() {
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
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    // Update without client to send update message.
    server_app.update();

    server_app.world.despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update(); // Let server receive an update to trigger acknowledgment.

    assert!(client_app.world.entities().is_empty());
}

#[test]
fn buffering() {
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
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    let previous_tick = *server_app.world.resource::<RepliconTick>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially rollback the client by 1 tick to force next received update to be buffered.
    *client_app.world.resource_mut::<RepliconTick>() = previous_tick;
    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let component = client_app
        .world
        .query::<&BoolComponent>()
        .single(&client_app.world);
    assert!(!component.0, "client should buffer the update");

    // Move tick forward to let the buffered update apply.
    client_app.world.resource_mut::<RepliconTick>().increment();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world
        .query::<&BoolComponent>()
        .single(&client_app.world);
    assert!(component.0, "buffered update should be applied");
}

#[test]
fn acknowledgment() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                update_timeout: Duration::ZERO, // Will cause dropping updates after each frame.
                ..Default::default()
            }),
        ))
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world
        .query::<Ref<BoolComponent>>()
        .single(&client_app.world);
    let tick1 = component.last_changed();

    // Take and drop ack message.
    let mut client = client_app.world.resource_mut::<RepliconClient>();
    assert_eq!(client.drain_sent().count(), 1);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let component = client_app
        .world
        .query::<Ref<BoolComponent>>()
        .single(&client_app.world);
    let tick2 = component.last_changed();

    assert!(
        tick1.get() < tick2.get(),
        "client should receive the same update twice because server missed the ack"
    );

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world
        .query::<Ref<BoolComponent>>()
        .single(&client_app.world);
    let tick3 = component.last_changed();

    assert_eq!(
        tick2.get(),
        tick3.get(),
        "client shouldn't receive acked update"
    );
}

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;

#[derive(Clone, Component, Copy, Deserialize, Serialize)]
struct BoolComponent(bool);

#[derive(Component, Default, Deserialize, Serialize)]
struct VecComponent(Vec<u8>);

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct OriginalComponent(bool);

#[derive(Component, Deserialize, Serialize)]
struct ReplacedComponent(bool);

/// Deserializes [`OriginalComponent`], but inserts it as [`ReplacedComponent`].
fn replace(
    rule_fns: &RuleFns<OriginalComponent>,
    commands: &mut Commands,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
    entity_map: &mut ServerEntityMap,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let mut mapper = ClientMapper {
        commands,
        entity_map,
    };

    let component = rule_fns.deserialize(cursor, &mut mapper)?;
    commands
        .entity(entity.id())
        .insert(ReplacedComponent(component.0));

    Ok(())
}
