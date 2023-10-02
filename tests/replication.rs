mod common;

use bevy::{ecs::world::EntityMut, prelude::*};
use bevy_replicon::{
    prelude::*,
    replicon_core::replication_rules::{self, serialize_component},
    server,
};

use bevy_renet::renet::transport::NetcodeClientTransport;
use serde::{Deserialize, Serialize};

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
    assert!(!acked_ticks.acked_ticks().contains_key(&client_id));
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
    let client_id = client_app
        .world
        .resource::<NetcodeClientTransport>()
        .client_id();
    let acked_tick = acked_ticks.acked_ticks()[&client_id];
    assert_eq!(acked_tick, NetworkTick::new(0));
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
    assert_eq!(
        entity_map.to_client().get(&server_entity),
        Some(&client_entity),
        "server entity should be mapped to a replicated entity on client"
    );
    assert_eq!(
        entity_map.to_server().get(&client_entity),
        Some(&server_entity),
        "replicated entity on client should be mapped to a server entity"
    );
}

#[test]
fn insert_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();

    use bevy_replicon::prelude::*;
    fn custom_deserialize(
        entity: &mut EntityMut,
        _entity_map: &mut NetworkEntityMap,
        cursor: &mut std::io::Cursor<bevy_renet::renet::Bytes>,
        tick: NetworkTick,
    ) -> Result<(), bincode::Error> {
        let mut component: CustomComponent =
            bincode::Options::deserialize_from(bincode::DefaultOptions::new(), cursor)?;
        component.0 = tick;
        entity.insert(component);

        Ok(())
    }

    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ))
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<IgnoredComponent>()
        .replicate_with::<CustomComponent>(
            serialize_component::<CustomComponent>,
            custom_deserialize,
        )
        .replicate_mapped::<MappedComponent>();
    }
    // setting this, to verify it was incremented and included in the replication data
    // and made available to custom deserializers.
    server_app.world.resource_mut::<NetworkTick>().0 = 99;

    common::connect(&mut server_app, &mut client_app);

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let server_map_entity = server_app.world.spawn_empty().id();
    let client_map_entity = client_app.world.spawn_empty().id();

    let client_entity = client_app.world.spawn_empty().id();
    let server_entity = server_app
        .world
        .spawn((
            Replication,
            TableComponent,
            SparseSetComponent,
            NonReplicatingComponent,
            MappedComponent(server_map_entity),
            IgnoredComponent,
            Ignored::<IgnoredComponent>::default(),
            CustomComponent(NetworkTick(0)),
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
    assert!(!client_entity.contains::<NonReplicatingComponent>());
    assert!(!client_entity.contains::<IgnoredComponent>());
    // a positive number of NetworkTick increments should have happened before replication
    assert!(*client_entity.get::<CustomComponent>().unwrap().0 >= 100);
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
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    server_app.update();

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>();

    let client_entity = client_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    client_app
        .world
        .resource_mut::<NetworkEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
    assert!(client_entity.contains::<NonReplicatingComponent>());
}

#[test]
fn custom_removal_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();

    // custom component removal. stores a copy in WrapperComponent<T> and then removes.
    fn custom_removal_fn<T: Component + Clone>(entity: &mut EntityMut, tick: NetworkTick) {
        let oldval: &T = entity.get::<T>().unwrap();
        entity
            .insert(WrapperComponent(oldval.clone(), tick))
            .remove::<T>();
    }

    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
        ))
        .replicate_and_remove_with::<TableComponent>(
            replication_rules::serialize_component::<TableComponent>,
            replication_rules::deserialize_component::<TableComponent>,
            custom_removal_fn::<TableComponent>,
        );
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    server_app.update();

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>();

    let client_entity = client_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    client_app
        .world
        .resource_mut::<NetworkEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
    assert!(client_entity.contains::<WrapperComponent<TableComponent>>());
    assert!(client_entity.contains::<NonReplicatingComponent>());
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

    let server_child_entity = server_app.world.spawn(Replication).id();
    let server_entity = server_app
        .world
        .spawn(Replication)
        .push_children(&[server_child_entity])
        .id();

    server_app.update();

    server_app.world.despawn(server_entity);
    server_app.world.despawn(server_child_entity);

    let client_child_entity = client_app.world.spawn_empty().id();
    let client_entity = client_app
        .world
        .spawn_empty()
        .push_children(&[client_child_entity])
        .id();

    let mut entity_map = client_app.world.resource_mut::<NetworkEntityMap>();
    entity_map.insert(server_entity, client_entity);
    entity_map.insert(server_child_entity, client_child_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.get_entity(client_entity).is_none());
    assert!(client_app.world.get_entity(client_child_entity).is_none());

    let entity_map = client_app.world.resource::<NetworkEntityMap>();
    assert!(entity_map.to_client().is_empty());
    assert!(entity_map.to_server().is_empty());
}

#[test]
fn replication_into_scene() {
    let mut app = App::new();
    app.add_plugins(ReplicationPlugins)
        .register_type::<ReflectedComponent>()
        .replicate::<ReflectedComponent>();

    app.world.spawn(ReflectedComponent);
    let reflect_entity = app.world.spawn((Replication, ReflectedComponent)).id();
    let empty_entity = app
        .world
        .spawn((
            Replication,
            ReflectedComponent,
            Ignored::<ReflectedComponent>::default(),
        ))
        .id();

    let mut scene = DynamicScene::default();
    server::replicate_into_scene(&mut scene, &app.world);

    assert!(scene.resources.is_empty());

    let [reflect, empty] = &scene.entities[..] else {
        panic!("scene should only contain entities marked for replication");
    };

    assert_eq!(reflect.entity, reflect_entity);
    assert_eq!(reflect.components.len(), 1);

    assert_eq!(empty.entity, empty_entity);
    assert!(empty.components.is_empty());
}

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapNetworkEntities for MappedComponent {
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T) {
        self.0 = mapper.map(self.0);
    }
}

#[derive(Component, Deserialize, Serialize, Clone)]
struct TableComponent;

#[derive(Component, Deserialize, Serialize)]
struct WrapperComponent<T: Component>(T, NetworkTick);

#[derive(Component, Deserialize, Serialize)]
struct CustomComponent(NetworkTick);

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSetComponent;

#[derive(Component)]
struct NonReplicatingComponent;

#[derive(Component, Deserialize, Serialize)]
struct IgnoredComponent;

#[derive(Component, Default, Deserialize, Reflect, Serialize)]
#[reflect(Component)]
struct ReflectedComponent;
