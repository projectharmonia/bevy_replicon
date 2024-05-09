use std::io::Cursor;

use bevy::{ecs::entity::MapEntities, prelude::*};
use bevy_replicon::{
    client::server_entity_map::ServerEntityMap,
    core::replication_fns::{command_fns, ctx::WriteCtx, rule_fns::RuleFns},
    prelude::*,
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};

#[test]
fn table_storage() {
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
        .replicate::<TableComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert(TableComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    client_app
        .world
        .query_filtered::<(), With<TableComponent>>()
        .single(&client_app.world);
}

#[test]
fn sparse_set_storage() {
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
        .replicate::<SparseSetComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert(SparseSetComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    client_app
        .world
        .query_filtered::<(), With<SparseSetComponent>>()
        .single(&client_app.world);
}

#[test]
fn mapped_existing_entity() {
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

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let server_entity = server_app.world.spawn(Replicated).id();
    let server_map_entity = server_app.world.spawn_empty().id();
    let client_map_entity = client_app.world.spawn_empty().id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_map_entity, client_map_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert(MappedComponent(server_map_entity));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mapped_component = client_app
        .world
        .query::<&MappedComponent>()
        .single(&client_app.world);
    assert_eq!(mapped_component.0, client_map_entity);
}

#[test]
fn mapped_new_entity() {
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

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let server_entity = server_app.world.spawn(Replicated).id();
    let server_map_entity = server_app.world.spawn_empty().id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert(MappedComponent(server_map_entity));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mapped_component = client_app
        .world
        .query::<&MappedComponent>()
        .single(&client_app.world);
    assert!(client_app.world.get_entity(mapped_component.0).is_some());
    assert_eq!(client_app.world.entities().len(), 2);
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

    let server_entity = server_app.world.spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert(OriginalComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<ReplacedComponent>, Without<OriginalComponent>)>()
        .single(&client_app.world);
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

    let server_entity = server_app.world.spawn(Replicated).id();
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

    server_app
        .world
        .entity_mut(server_entity)
        .insert(OriginalComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<OriginalComponent>());
    assert!(client_entity.contains::<ReplacedComponent>());
}

#[test]
fn group() {
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
        .replicate_group::<(GroupComponentA, GroupComponentB)>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert((GroupComponentA, GroupComponentB));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<GroupComponentA>, With<GroupComponentB>)>()
        .single(&client_app.world);
}

#[test]
fn not_replicated() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ));
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world
        .entity_mut(server_entity)
        .insert(NotReplicatedComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let not_replicated_components = client_app
        .world
        .query_filtered::<(), With<NotReplicatedComponent>>()
        .iter(&client_app.world)
        .count();
    assert_eq!(not_replicated_components, 0);
}

#[test]
fn after_removal() {
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
        .replicate::<TableComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world.spawn((Replicated, TableComponent)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Insert and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>()
        .insert(TableComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    client_app
        .world
        .query_filtered::<(), With<TableComponent>>()
        .single(&client_app.world);
}

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapEntities for MappedComponent {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}

#[derive(Component, Deserialize, Serialize)]
struct TableComponent;

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSetComponent;

#[derive(Component, Deserialize, Serialize)]
struct GroupComponentA;

#[derive(Component, Deserialize, Serialize)]
struct GroupComponentB;

#[derive(Component, Deserialize, Serialize)]
struct NotReplicatedComponent;

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct OriginalComponent;

#[derive(Component, Deserialize, Serialize)]
struct ReplacedComponent;

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
