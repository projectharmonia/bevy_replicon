mod connect;

use bevy::prelude::*;
use bevy_replicon::{
    prelude::*,
    renet::{transport::NetcodeClientTransport, ClientId},
};
use serde::{Deserialize, Serialize};

#[test]
fn spawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ));
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn(Replication).id();

    server_app.update();
    client_app.update();

    let client_entity = client_app
        .world
        .query_filtered::<Entity, With<Replication>>()
        .single(&client_app.world);

    let entity_map = client_app.world.resource::<ServerEntityMap>();
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
fn spawn_with_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    server_app.world.spawn((Replication, TableComponent));

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<TableComponent>)>()
        .single(&client_app.world);
}

#[test]
fn spawn_with_old_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    // Spawn an entity with replicated component, but without a marker.
    let server_entity = server_app.world.spawn(TableComponent).id();

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());

    // Enable replication for previously spawned entity
    server_app
        .world
        .entity_mut(server_entity)
        .insert(Replication);

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<TableComponent>)>()
        .single(&client_app.world);
}

#[test]
fn spawn_before_connection() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    // Spawn an entity before client connected.
    server_app.world.spawn((Replication, TableComponent));

    connect::single_client(&mut server_app, &mut client_app);

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<TableComponent>)>()
        .single(&client_app.world);
}

#[test]
fn client_spawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let client_entity = client_app.world.spawn_empty().id();
    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();

    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = ClientId::from_raw(client_transport.client_id());

    let mut entity_map = server_app.world.resource_mut::<ClientEntityMap>();
    entity_map.insert(
        client_id,
        ClientMapping {
            server_entity,
            client_entity,
        },
    );

    server_app.update();
    client_app.update();

    let entity_map = client_app.world.resource::<ServerEntityMap>();
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

    let client_entity = client_app.world.entity(client_entity);
    assert!(
        client_entity.contains::<Replication>(),
        "server should confirm replication of client entity"
    );
    assert!(
        client_entity.contains::<TableComponent>(),
        "component from server should be replicated"
    );

    assert_eq!(
        client_app.world.entities().len(),
        1,
        "new entity shouldn't be spawned on client"
    );
}

#[test]
fn despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ));
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn(Replication).id();
    let client_entity = client_app.world.spawn(Replication).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.world.despawn(server_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());

    let entity_map = client_app.world.resource::<ServerEntityMap>();
    assert!(entity_map.to_client().is_empty());
    assert!(entity_map.to_server().is_empty());
}

#[test]
fn despawn_with_heirarchy() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ));
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_child_entity = server_app.world.spawn(Replication).id();
    let server_entity = server_app
        .world
        .spawn(Replication)
        .push_children(&[server_child_entity])
        .id();

    let client_child_entity = client_app.world.spawn(Replication).id();
    let client_entity = client_app
        .world
        .spawn(Replication)
        .push_children(&[client_child_entity])
        .id();

    let mut entity_map = client_app.world.resource_mut::<ServerEntityMap>();
    entity_map.insert(server_entity, client_entity);
    entity_map.insert(server_child_entity, client_child_entity);

    server_app.world.despawn(server_entity);
    server_app.world.despawn(server_child_entity);

    server_app.update();
    client_app.update();

    server_app.world.despawn(server_entity);
    server_app.world.despawn(server_child_entity);

    assert!(client_app.world.entities().is_empty());
}

#[test]
fn despawn_after_spawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    // Insert and remove `Replication` to trigger spawn and despawn for client at the same time.
    server_app
        .world
        .spawn((Replication, TableComponent))
        .remove::<Replication>();

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());
}

#[test]
fn spawn_after_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    // Remove and insert `Replication` to trigger despawn and spawn for client at the same time.
    server_app
        .world
        .spawn((Replication, TableComponent))
        .remove::<Replication>()
        .insert(Replication);

    server_app.update();
    client_app.update();

    client_app
        .world
        .query_filtered::<(), (With<Replication>, With<TableComponent>)>()
        .single(&client_app.world);
}

#[test]
fn insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<NotReplicatedComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let server_map_entity = server_app.world.spawn_empty().id();
    let client_map_entity = client_app.world.spawn_empty().id();

    let server_entity = server_app
        .world
        .spawn(Replication)
        .dont_replicate::<NotReplicatedComponent>()
        .id();
    let client_entity = client_app.world.spawn(Replication).id();

    let mut entity_map = client_app.world.resource_mut::<ServerEntityMap>();
    entity_map.insert(server_map_entity, client_map_entity);
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    server_app.world.entity_mut(server_entity).insert((
        Replication,
        TableComponent,
        SparseSetComponent,
        NonReplicatingComponent,
        MappedComponent(server_map_entity),
        NotReplicatedComponent,
    ));

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<SparseSetComponent>());
    assert!(client_entity.contains::<TableComponent>());
    assert!(!client_entity.contains::<NonReplicatingComponent>());
    assert!(!client_entity.contains::<NotReplicatedComponent>());
    assert_eq!(
        client_entity.get::<MappedComponent>().unwrap().0,
        client_map_entity
    );
}

#[test]
fn dont_replicate_after_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn(Replication).id();
    let client_entity = client_app.world.spawn(Replication).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    // Insert and disable replication.
    server_app
        .world
        .entity_mut(server_entity)
        .insert(TableComponent)
        .dont_replicate::<TableComponent>();

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
}

#[test]
fn dont_replicate_after_reinsertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();
    let client_entity = client_app.world.spawn((Replication, TableComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    // Reinsert and disable replication.
    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>()
        .insert(TableComponent)
        .dont_replicate::<TableComponent>();

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
}

#[test]
fn removal() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();
    let client_entity = client_app
        .world
        .spawn((Replication, TableComponent, NonReplicatingComponent))
        .id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>();

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
    assert!(client_entity.contains::<NonReplicatingComponent>());
}

#[test]
fn removal_after_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<NotReplicatedComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();
    let client_entity = client_app.world.spawn((Replication, TableComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    // Insert and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .insert(TableComponent)
        .remove::<TableComponent>();

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
}

#[test]
fn insertion_after_removal() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<NotReplicatedComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();
    let client_entity = client_app.world.spawn((Replication, TableComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    // Insert and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>()
        .insert(TableComponent);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<TableComponent>());
}

#[test]
fn removal_with_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<TableComponent>()
        .replicate::<SparseSetComponent>()
        .replicate::<NotReplicatedComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    connect::single_client(&mut server_app, &mut client_app);

    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();
    let client_entity = client_app.world.spawn((Replication, TableComponent)).id();

    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    // Un-replicate and remove at the same time.
    server_app
        .world
        .entity_mut(server_entity)
        .remove::<TableComponent>()
        .remove::<Replication>();

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());
}

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapNetworkEntities for MappedComponent {
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T) {
        self.0 = mapper.map(self.0);
    }
}

#[derive(Component, Deserialize, Serialize)]
struct TableComponent;

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSetComponent;

#[derive(Component)]
struct NonReplicatingComponent;

#[derive(Component, Deserialize, Serialize)]
struct NotReplicatedComponent;
