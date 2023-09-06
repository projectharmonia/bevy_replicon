mod common;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use bevy::ecs::{
    entity::{EntityMapper, MapEntities},
    reflect::ReflectMapEntities,
};
use bevy_renet::renet::transport::NetcodeClientTransport;

#[test]
fn acked_ticks_cleanup() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ));
    }

    common::connect(&mut server_app, &mut client_app);

    let mut client_transport = client_app.world.resource_mut::<NetcodeClientTransport>();
    client_transport.disconnect();
    let client_id = client_transport.client_id();

    client_app.update();
    server_app.update();
    server_app.update();

    let acked_ticks = server_app.world.resource::<AckedTicks>();
    assert!(!acked_ticks.contains_key(&client_id));
}

#[test]
fn tick_acks_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ));
    }

    common::connect(&mut server_app, &mut client_app);

    client_app.update();
    server_app.update();

    let acked_ticks = server_app.world.resource::<AckedTicks>();
    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    assert!(
        matches!(acked_ticks.get(&client_transport.client_id()), Some(&last_tick) if last_tick.get() > 0)
    );
}

#[test]
fn spawn_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ))
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((TableComponent, Replication)).id();

    server_app.update();
    client_app.update();

    let client_entity = client_app
        .world
        .query_filtered::<Entity, (With<TableComponent>, With<Replication>)>()
        .single(&client_app.world);
    let entity_map = client_app.world.resource::<NetworkEntityMap>();
    let mapped_entity = entity_map
        .to_client()
        .get(server_entity)
        .expect("server entity should be mapped on client");
    assert_eq!(
        mapped_entity, client_entity,
        "mapped entity should correspond to the replicated entity on client"
    );
}

#[test]
fn insert_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ))
        .register_type::<NonReflectedComponent>()
        .register_type::<ExclusionComponent>()
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<IgnoredComponent>()
        .replicate::<MappedComponent>()
        .not_replicate_if_present::<IgnoredComponent, ExclusionComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_map_entity = server_app.world.spawn_empty().id();
    let client_map_entity = client_app.world.spawn_empty().id();

    let client_entity = client_app.world.spawn_empty().id();
    let server_entity = server_app
        .world
        .spawn((
            Replication,
            TableComponent,
            SparseSetComponent,
            NonReflectedComponent,
            MappedComponent(server_map_entity),
            IgnoredComponent,
            ExclusionComponent,
        ))
        .id();

    let mut entity_map = client_app.world.resource_mut::<NetworkEntityMap>();
    entity_map.insert(server_map_entity, client_map_entity);
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<SparseSetComponent>());
    assert!(client_entity.contains::<TableComponent>());
    assert!(!client_entity.contains::<NonReflectedComponent>());
    assert!(!client_entity.contains::<ExclusionComponent>());
    assert_eq!(
        client_entity.get::<MappedComponent>().unwrap().0,
        client_map_entity
    );
}

#[test]
fn removal_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ))
        .register_type::<NonReflectedComponent>()
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, TableComponent, NonReflectedComponent))
        .id();

    server_app.update();

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>();

    let client_entity = client_app
        .world
        .spawn((Replication, TableComponent, NonReflectedComponent))
        .id();

    client_app
        .world
        .resource_mut::<NetworkEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
    assert!(client_entity.contains::<NonReflectedComponent>());
}

#[test]
fn despawn_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ));
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn(Replication).id();

    server_app.update();

    server_app.world.despawn(server_entity);

    let child_entity = client_app.world.spawn_empty().id();
    let client_entity = client_app
        .world
        .spawn_empty()
        .push_children(&[child_entity])
        .id();

    client_app
        .world
        .resource_mut::<NetworkEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.get_entity(client_entity).is_none());
    assert!(client_app.world.get_entity(child_entity).is_none());

    let entity_map = client_app.world.resource::<NetworkEntityMap>();
    assert!(entity_map.to_client().is_empty());
}

#[derive(Component, Reflect)]
#[reflect(Component, MapEntities)]
struct MappedComponent(Entity);

impl MapEntities for MappedComponent {
    fn map_entities(&mut self, entity_map: &mut EntityMapper) {
        self.0 = entity_map.get_or_reserve(self.0);
    }
}

impl FromWorld for MappedComponent {
    fn from_world(_world: &mut World) -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct TableComponent;

#[derive(Component, Default, Reflect)]
#[component(storage = "SparseSet")]
#[reflect(Component)]
struct SparseSetComponent;

#[derive(Component, Reflect)]
struct NonReflectedComponent;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct IgnoredComponent;

#[derive(Component, Reflect)]
struct ExclusionComponent;
