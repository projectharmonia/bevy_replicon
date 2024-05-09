use std::io::Cursor;

use bevy::{ecs::entity::MapEntities, prelude::*, utils::Duration};
use bevy_replicon::{
    client::{confirmed::Confirmed, server_entity_map::ServerEntityMap, ServerInitTick},
    core::{
        command_markers::MarkerConfig,
        replication_fns::{command_fns, ctx::WriteCtx, rule_fns::RuleFns},
    },
    prelude::*,
    server::server_tick::ServerTick,
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
        .spawn((Replicated, BoolComponent(false)))
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
        .spawn((Replicated, VecComponent::default()))
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
        .spawn((Replicated, OriginalComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world
        .query_filtered::<Entity, With<ReplacedComponent>>()
        .single(&client_app.world);

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
        .spawn((Replicated, OriginalComponent(false)))
        .id();

    let client_entity = client_app.world.spawn(ReplaceMarker).id();

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
fn marker_with_history() {
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
        .register_marker_with::<HistoryMarker>(MarkerConfig {
            need_history: true,
            ..Default::default()
        })
        .set_marker_fns::<HistoryMarker, BoolComponent>(
            write_history,
            command_fns::default_remove::<BoolComponent>,
        )
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replicated, BoolComponent(false)))
        .id();

    let client_entity = client_app.world.spawn(HistoryMarker).id();

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

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value, but don't process it on client.
    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value again to generate another update.
    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = false;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    let history = client_entity.get::<BoolHistory>().unwrap();
    assert_eq!(
        history.0,
        [false, false, true],
        "the initial value should come first, then the latest update, \
        and after that the older update because recent updates processed first"
    );
}

#[test]
fn marker_with_history_consume() {
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
        .register_marker_with::<HistoryMarker>(MarkerConfig {
            need_history: true,
            ..Default::default()
        })
        .set_marker_fns::<HistoryMarker, BoolComponent>(
            write_history,
            command_fns::default_remove::<BoolComponent>,
        )
        .replicate::<BoolComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_map_entity = server_app.world.spawn_empty().id();
    let server_entity = server_app
        .world
        .spawn((
            Replicated,
            BoolComponent(false),
            MappedComponent(server_map_entity),
        ))
        .id();

    let client_entity = client_app.world.spawn(HistoryMarker).id();

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

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value, but don't process it on client.
    let update_entity1 = server_app.world.spawn_empty().id();
    let mut component = server_app
        .world
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = update_entity1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value again to generate another update.
    let update_entity2 = server_app.world.spawn_empty().id();
    let mut component = server_app
        .world
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = update_entity2;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world.resource::<ServerEntityMap>();
    assert!(entity_map.to_client().contains_key(&update_entity2));
    assert!(
        !entity_map.to_client().contains_key(&update_entity1),
        "client should consume older update for other components with marker that requested history"
    );
    assert_eq!(
        client_app.world.entities().len(),
        3,
        "client should have 2 initial entities and 1 from update"
    );
}

#[test]
fn marker_with_history_old_update() {
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
        .register_marker_with::<HistoryMarker>(MarkerConfig {
            need_history: true,
            ..Default::default()
        })
        .set_marker_fns::<HistoryMarker, BoolComponent>(
            write_history,
            command_fns::default_remove::<BoolComponent>,
        )
        .replicate::<BoolComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replicated, BoolComponent(false)))
        .id();

    let client_entity = client_app.world.spawn(HistoryMarker).id();

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

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially make the last confirmed tick too large
    // so that the next update for this entity is discarded.
    let mut tick = **server_app.world.resource::<ServerTick>();
    tick += u64::BITS + 1;
    let mut confirmed = client_app
        .world
        .get_mut::<Confirmed>(client_entity)
        .unwrap();
    confirmed.confirm(tick);

    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let history = client_app.world.get::<BoolHistory>(client_entity).unwrap();
    assert_eq!(
        history.0,
        [false],
        "update should be considered too old and discarded"
    );
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
        .spawn_batch([(Replicated, BoolComponent(false)); ENTITIES_COUNT as usize]);

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
        .spawn((Replicated, BoolComponent(false)))
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
        .spawn((Replicated, BoolComponent(false)))
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
        .spawn((Replicated, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the init tick to force the next received update to be buffered.
    let mut init_tick = client_app.world.resource_mut::<ServerInitTick>();
    let previous_tick = *init_tick;
    *init_tick = Default::default();
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

    // Restore the init tick to let the buffered update apply
    *client_app.world.resource_mut::<ServerInitTick>() = previous_tick;

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
fn old_ignored() {
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
        .replicate_mapped::<MappedComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_map_entity = server_app.world.spawn_empty().id();
    let server_entity = server_app
        .world
        .spawn((Replicated, MappedComponent(server_map_entity)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change the value, but don't process it on client.
    let update_entity1 = server_app.world.spawn_empty().id();
    let mut component = server_app
        .world
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = update_entity1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change the value again to generate another update.
    let update_entity2 = server_app.world.spawn_empty().id();
    let mut component = server_app
        .world
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = update_entity2;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world.resource::<ServerEntityMap>();
    assert!(
        !entity_map.to_client().contains_key(&update_entity1),
        "client should ignore older update"
    );
    assert_eq!(
        client_app.world.entities().len(),
        3,
        "client should have 2 initial entities and 1 from update"
    );
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
        .spawn((Replicated, BoolComponent(false)))
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

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapEntities for MappedComponent {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct OriginalComponent(bool);

#[derive(Component, Deserialize, Serialize)]
struct ReplacedComponent(bool);

#[derive(Component)]
struct HistoryMarker;

#[derive(Component, Deref, DerefMut)]
struct BoolHistory(Vec<bool>);

/// Deserializes [`OriginalComponent`], but inserts it as [`ReplacedComponent`].
fn replace(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<OriginalComponent>,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    let component = rule_fns.deserialize(ctx, cursor)?;
    ctx.commands
        .entity(entity.id())
        .insert(ReplacedComponent(component.0));

    Ok(())
}

/// Instead of writing into [`BoolComponent`], it writes data into [`BoolHistory`].
fn write_history(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<BoolComponent>,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    let component = rule_fns.deserialize(ctx, cursor)?;
    if let Some(mut history) = entity.get_mut::<BoolHistory>() {
        history.push(component.0);
    } else {
        ctx.commands
            .entity(entity.id())
            .insert(BoolHistory(vec![component.0]));
    }

    Ok(())
}
