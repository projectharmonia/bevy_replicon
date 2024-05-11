use std::io::Cursor;

use bevy::prelude::*;
use bevy_replicon::{
    core::{
        command_markers::MarkerConfig,
        replication_fns::{
            command_fns,
            ctx::{DespawnCtx, WriteCtx},
            rule_fns::RuleFns,
            test_fns::TestFnsEntityExt,
            ReplicationFns,
        },
    },
    prelude::*,
    server::server_tick::ServerTick,
};
use serde::{Deserialize, Serialize};

#[test]
#[should_panic]
fn serialize_missing_component() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn_empty();
    let _ = entity.serialize(fns_info);
}

#[test]
fn write() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn(OriginalComponent);
    let data = entity.serialize(fns_info);
    entity.remove::<OriginalComponent>();
    entity.apply_write(&data, fns_info, tick);
    assert!(entity.contains::<OriginalComponent>());
}

#[test]
fn remove() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn(OriginalComponent);
    entity.apply_remove(fns_info, tick);
    assert!(!entity.contains::<OriginalComponent>());
}

#[test]
fn write_with_command() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .set_command_fns(replace, command_fns::default_remove::<ReplacedComponent>);

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn(OriginalComponent);
    let data = entity.serialize(fns_info);
    entity.apply_write(&data, fns_info, tick);
    assert!(entity.contains::<ReplacedComponent>());
}

#[test]
fn remove_with_command() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .set_command_fns(replace, command_fns::default_remove::<ReplacedComponent>);

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn(ReplacedComponent);
    entity.apply_remove(fns_info, tick);
    assert!(!entity.contains::<ReplacedComponent>());
}

#[test]
fn write_without_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn(OriginalComponent);
    let data = entity.serialize(fns_info);
    entity.remove::<OriginalComponent>();
    entity.apply_write(&data, fns_info, tick);
    assert!(entity.contains::<OriginalComponent>());
}

#[test]
fn remove_without_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn(OriginalComponent);
    entity.apply_remove(fns_info, tick);
    assert!(!entity.contains::<OriginalComponent>());
}

#[test]
fn write_with_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn((OriginalComponent, ReplaceMarker));
    let data = entity.serialize(fns_info);
    entity.apply_write(&data, fns_info, tick);
    assert!(entity.contains::<ReplacedComponent>());
}

#[test]
fn remove_with_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app.world.spawn((ReplacedComponent, ReplaceMarker));
    entity.apply_remove(fns_info, tick);
    assert!(!entity.contains::<ReplacedComponent>());
}

#[test]
fn write_with_multiple_markers() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<DummyMarker>()
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<DummyMarker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app
        .world
        .spawn((OriginalComponent, ReplaceMarker, DummyMarker));
    let data = entity.serialize(fns_info);
    entity.apply_write(&data, fns_info, tick);
    assert!(
        entity.contains::<ReplacedComponent>(),
        "last marker should take priority"
    );
}

#[test]
fn remove_with_mutltiple_markers() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<DummyMarker>()
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<DummyMarker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app
        .world
        .spawn((ReplacedComponent, ReplaceMarker, DummyMarker));
    entity.apply_remove(fns_info, tick);
    assert!(
        !entity.contains::<ReplacedComponent>(),
        "last marker should take priority"
    );
}

#[test]
fn write_with_priority_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker_with::<ReplaceMarker>(MarkerConfig {
            priority: 1,
            ..Default::default()
        })
        .register_marker::<DummyMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<DummyMarker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app
        .world
        .spawn((OriginalComponent, ReplaceMarker, DummyMarker));
    let data = entity.serialize(fns_info);
    entity.apply_write(&data, fns_info, tick);
    assert!(entity.contains::<ReplacedComponent>());
}

#[test]
fn remove_with_priority_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker_with::<ReplaceMarker>(MarkerConfig {
            priority: 1,
            ..Default::default()
        })
        .register_marker::<DummyMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<DummyMarker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = **app.world.resource::<ServerTick>();
    let fns_info = app
        .world
        .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
            replication_fns.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
        });

    let mut entity = app
        .world
        .spawn((ReplacedComponent, ReplaceMarker, DummyMarker));
    entity.apply_remove(fns_info, tick);
    assert!(!entity.contains::<ReplacedComponent>());
}

#[test]
fn despawn() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let mut replication_fns = app.world.resource_mut::<ReplicationFns>();
    replication_fns.despawn = mark_despawned;

    let tick = **app.world.resource::<ServerTick>();
    let entity = app.world.spawn_empty();
    let id = entity.id(); // Take ID since despawn function consumes entity.
    entity.apply_despawn(tick);
    assert!(app.world.get::<Despawned>(id).is_some());
}

#[derive(Component, Deserialize, Serialize)]
struct OriginalComponent;

#[derive(Component, Deserialize, Serialize)]
struct ReplacedComponent;

#[derive(Component)]
struct Despawned;

#[derive(Component, Deserialize, Serialize)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct DummyMarker;

/// Deserializes [`OriginalComponent`], but ignores it and inserts [`ReplacedComponent`].
fn replace(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<OriginalComponent>,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    rule_fns.deserialize(ctx, cursor)?;
    ctx.commands.entity(entity.id()).insert(ReplacedComponent);

    Ok(())
}

/// Adds special [`Despawned`] marker instead of despawning an entity.
fn mark_despawned(_ctx: &DespawnCtx, mut entity: EntityWorldMut) {
    entity.insert(Despawned);
}
