use bevy::{ecs::entity::MapEntities, prelude::*};
use bevy_replicon::{
    client::confirm_history::{ConfirmHistory, EntityReplicated},
    prelude::*,
    server::server_tick::ServerTick,
    shared::{
        replication::{
            deferred_entity::DeferredEntity,
            replication_registry::{command_fns, ctx::WriteCtx, rule_fns::RuleFns},
        },
        server_entity_map::ServerEntityMap,
    },
    test_app::{ServerTestAppExt, TestClientEntity},
};
use bytes::Bytes;
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
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(DummyComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&DummyComponent>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(SparseSetComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&SparseSetComponent>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();
    let server_map_entity = server_app.world_mut().spawn_empty().id();
    let client_map_entity = client_app.world_mut().spawn_empty().id();
    assert_ne!(server_map_entity, client_map_entity);

    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_map_entity, client_map_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(MappedComponent(server_map_entity));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mapped_component = client_app
        .world_mut()
        .query::<&MappedComponent>()
        .single(client_app.world());
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();
    let server_map_entity = server_app.world_mut().spawn_empty().id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(MappedComponent(server_map_entity));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mapped_component = client_app
        .world_mut()
        .query::<&MappedComponent>()
        .single(client_app.world());
    assert!(client_app.world().get_entity(mapped_component.0).is_ok());

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).count(), 2);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(OriginalComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app
        .world_mut()
        .query_filtered::<&ReplacedComponent, Without<OriginalComponent>>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();
    let client_entity = client_app.world_mut().spawn(ReplaceMarker).id();
    assert_ne!(server_entity, client_entity);

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(test_client_entity)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(OriginalComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert((GroupComponentA, GroupComponentB));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut groups = client_app
        .world_mut()
        .query::<(&GroupComponentA, &GroupComponentB)>();
    assert_eq!(groups.iter(client_app.world()).count(), 1);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(DummyComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&DummyComponent>();
    assert_eq!(components.iter(client_app.world()).count(), 0);
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
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, DummyComponent))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Insert and remove at the same time.
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<DummyComponent>()
        .insert(DummyComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&DummyComponent>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn before_started_replication() {
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
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, DummyComponent));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&DummyComponent>();
    assert_eq!(
        components.iter(client_app.world()).count(),
        0,
        "no entities should have been sent to the client"
    );

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(test_client_entity)
        .insert(ReplicatedClient);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn after_started_replication() {
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
        .replicate::<DummyComponent>();
    }

    server_app.connect_client(&mut client_app);

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(test_client_entity)
        .insert(ReplicatedClient);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, DummyComponent));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&DummyComponent>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn confirm_history() {
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(DummyComponent);

    // Clear previous events.
    client_app
        .world_mut()
        .resource_mut::<Events<EntityReplicated>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let (client_entity, confirm_history) = client_app
        .world_mut()
        .query::<(Entity, &ConfirmHistory)>()
        .single(client_app.world());
    assert!(confirm_history.contains(tick));

    let mut replicated_events = client_app
        .world_mut()
        .resource_mut::<Events<EntityReplicated>>();
    let [event] = replicated_events
        .drain()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(event.entity, client_entity);
    assert_eq!(event.tick, tick);
}

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapEntities for MappedComponent {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSetComponent;

#[derive(Component, Deserialize, Serialize)]
struct GroupComponentA;

#[derive(Component, Deserialize, Serialize)]
struct GroupComponentB;

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
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> postcard::Result<()> {
    rule_fns.deserialize(ctx, message)?;
    ctx.commands.entity(entity.id()).insert(ReplacedComponent);

    Ok(())
}
