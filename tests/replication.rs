mod common;

use std::ops::DerefMut;

use bevy::{prelude::*, utils::Duration};
use bevy_replicon::{prelude::*, scene};

use bevy_renet::renet::{
    transport::{NetcodeClientTransport, NetcodeServerTransport},
    ClientId,
};
use serde::{Deserialize, Serialize};

#[test]
fn reset() {
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

    common::connect(&mut server_app, &mut client_app);

    client_app.world.resource_mut::<RenetClient>().disconnect();

    client_app.update();
    server_app.update();

    client_app.update();
    server_app.update();

    client_app.world.remove_resource::<RenetClient>();
    server_app.world.remove_resource::<RenetServer>();

    server_app.update();
    client_app.update();

    assert_eq!(server_app.world.resource::<RepliconTick>().get(), 0);
    assert_eq!(client_app.world.resource::<RepliconTick>().get(), 0);
}

#[test]
fn spawn_replication() {
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

    common::connect(&mut server_app, &mut client_app);

    server_app.world.spawn(Replication); // Empty entity that won't be replicated.
    let server_entity = server_app.world.spawn((Replication, TableComponent)).id();

    server_app.update();
    client_app.update();

    let client_entity = client_app
        .world
        .query_filtered::<Entity, (With<Replication>, With<TableComponent>)>()
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
    assert_eq!(
        client_app.world.entities().len(),
        1,
        "empty entity shouldn't be replicated"
    );
}

#[test]
fn client_spawn_replication() {
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

    common::connect(&mut server_app, &mut client_app);

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
fn insert_replication() {
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
        .replicate::<IgnoredComponent>()
        .replicate_mapped::<MappedComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    // Make client and server have different entity IDs.
    server_app.world.spawn_empty();

    let server_map_entity = server_app.world.spawn_empty().id();
    let client_map_entity = client_app.world.spawn_empty().id();

    let client_entity = client_app.world.spawn(Replication).id();
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
        ))
        .id();

    let mut entity_map = client_app.world.resource_mut::<ServerEntityMap>();
    entity_map.insert(server_map_entity, client_map_entity);
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(client_entity.contains::<SparseSetComponent>());
    assert!(client_entity.contains::<TableComponent>());
    assert!(!client_entity.contains::<NonReplicatingComponent>());
    assert!(!client_entity.contains::<IgnoredComponent>());
    assert_eq!(
        client_entity.get::<MappedComponent>().unwrap().0,
        client_map_entity
    );
}

#[test]
fn despawn_replication() {
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

    let client_child_entity = client_app.world.spawn(Replication).id();
    let client_entity = client_app
        .world
        .spawn(Replication)
        .push_children(&[client_child_entity])
        .id();

    let mut entity_map = client_app.world.resource_mut::<ServerEntityMap>();
    entity_map.insert(server_entity, client_entity);
    entity_map.insert(server_child_entity, client_child_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.get_entity(client_entity).is_none());
    assert!(client_app.world.get_entity(client_child_entity).is_none());

    let entity_map = client_app.world.resource::<ServerEntityMap>();
    assert!(entity_map.to_client().is_empty());
    assert!(entity_map.to_server().is_empty());
}

#[test]
fn removal_replication() {
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
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    client_app.update();

    let client_entity = client_app.world.entity(client_entity);
    assert!(!client_entity.contains::<TableComponent>());
    assert!(client_entity.contains::<NonReplicatingComponent>());
}

#[test]
fn old_entities_replication() {
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

    common::connect(&mut server_app, &mut client_app);

    assert_eq!(client_app.world.entities().len(), 1);
}

#[test]
fn update_replication() {
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
        .replicate::<BoolComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    client_app.update();

    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    client_app.update();

    let component = client_app
        .world
        .query::<&BoolComponent>()
        .single(&client_app.world);
    assert!(component.0);
}

#[test]
fn big_entity_update_replication() {
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
        .replicate::<VecComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, VecComponent::default()))
        .id();

    server_app.update();
    client_app.update();

    // To exceed packed size.
    const BIG_DATA: &[u8] = &[0; 1200];
    let mut component = server_app
        .world
        .get_mut::<VecComponent>(server_entity)
        .unwrap();
    component.0 = BIG_DATA.to_vec();

    server_app.update();
    client_app.update();

    let component = client_app
        .world
        .query::<&VecComponent>()
        .single(&client_app.world);
    assert_eq!(component.0, BIG_DATA);
}

#[test]
fn many_entities_update_replication() {
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
        .replicate::<BoolComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    // Spawn many entities to cover message splitting.
    const ENTITIES_COUNT: u32 = 300;
    server_app
        .world
        .spawn_batch([(Replication, BoolComponent(false)); ENTITIES_COUNT as usize]);

    server_app.update();
    client_app.update();

    assert_eq!(client_app.world.entities().len(), ENTITIES_COUNT);

    for mut component in server_app
        .world
        .query::<&mut BoolComponent>()
        .iter_mut(&mut server_app.world)
    {
        component.0 = true;
    }

    server_app.update();
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
fn insert_update_replication() {
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
        .replicate::<BoolComponent>()
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    client_app.update();

    let mut server_entity = server_app.world.entity_mut(server_entity);
    server_entity.get_mut::<BoolComponent>().unwrap().0 = true;
    server_entity.insert(TableComponent);

    server_app.update();
    client_app.update();

    let component = client_app
        .world
        .query_filtered::<&BoolComponent, With<TableComponent>>()
        .single(&client_app.world);
    assert!(component.0);
}

#[test]
fn despawn_update_replication() {
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
        .replicate::<BoolComponent>()
        .replicate::<TableComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    client_app.update();

    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    // Update without client to send update message.
    server_app.update();

    server_app.world.despawn(server_entity);

    server_app.update();
    client_app.update();

    assert!(client_app.world.entities().is_empty());
}

#[test]
fn update_replication_buffering() {
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
        .replicate::<BoolComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    let old_tick = *server_app.world.resource::<RepliconTick>();

    server_app.update();
    client_app.update();

    // Artificially rollback the client by 1 tick to force next received update to be buffered.
    *client_app.world.resource_mut::<RepliconTick>() = old_tick;
    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    client_app.update();

    let (client_entity, component) = client_app
        .world
        .query::<(Entity, &BoolComponent)>()
        .single(&client_app.world);
    assert!(!component.0, "client should buffer the update");

    // Move tick forward to let the buffered update apply.
    client_app.world.resource_mut::<RepliconTick>().increment();

    server_app.update();
    client_app.update();

    let component = client_app
        .world
        .get::<BoolComponent>(client_entity)
        .unwrap();
    assert!(component.0, "buffered update should be applied");
}

#[test]
fn update_replication_cleanup() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            ReplicationPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                update_timeout: Duration::ZERO, // Will cause dropping updates after each frame.
            }),
        ))
        .replicate::<BoolComponent>();
    }

    common::connect(&mut server_app, &mut client_app);

    let server_entity = server_app
        .world
        .spawn((Replication, BoolComponent(false)))
        .id();

    server_app.update();
    client_app.update();

    let mut component = server_app
        .world
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    client_app.update();

    let component = client_app
        .world
        .query::<Ref<BoolComponent>>()
        .single(&client_app.world);
    let tick1 = component.last_changed();

    // Take and drop received message to make systems miss it.
    let client_transport = client_app.world.resource::<NetcodeClientTransport>();
    let client_id = ClientId::from_raw(client_transport.client_id());
    let delta = server_app.world.resource::<Time>().delta();
    server_app
        .world
        .resource_scope(|world, mut server_transport: Mut<NetcodeServerTransport>| {
            let mut server = world.resource_mut::<RenetServer>();
            server_transport.update(delta, &mut server).unwrap();
            server
                .receive_message(client_id, ReplicationChannel::Reliable)
                .unwrap();
        });

    server_app.update();
    client_app.update();

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
    scene::replicate_into(&mut scene, &app.world);

    assert!(scene.resources.is_empty());

    let [reflect, empty] = &scene.entities[..] else {
        panic!("scene should only contain entities marked for replication");
    };

    assert_eq!(reflect.entity, reflect_entity);
    assert_eq!(reflect.components.len(), 1);

    assert_eq!(empty.entity, empty_entity);
    assert!(empty.components.is_empty());
}

#[test]
fn diagnostics() {
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
    client_app.add_plugins(ClientDiagnosticsPlugin);

    common::connect(&mut server_app, &mut client_app);

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

    server_app.world.spawn(Replication).despawn();

    server_app.update();
    client_app.update();

    // Trigger change detection.
    server_app
        .world
        .get_mut::<TableComponent>(server_entity)
        .unwrap()
        .deref_mut();

    server_app.update();
    client_app.update();

    let stats = client_app.world.resource::<ClientStats>();
    assert_eq!(stats.entities_changed, 2);
    assert_eq!(stats.components_changed, 2);
    assert_eq!(stats.mappings, 1);
    assert_eq!(stats.despawns, 1);
    assert_eq!(stats.packets, 2);
    assert_eq!(stats.bytes, 31);
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
struct IgnoredComponent;

#[derive(Clone, Component, Copy, Deserialize, Serialize)]
struct BoolComponent(bool);

#[derive(Component, Default, Deserialize, Serialize)]
struct VecComponent(Vec<u8>);

#[derive(Component, Default, Deserialize, Reflect, Serialize)]
#[reflect(Component)]
struct ReflectedComponent;
