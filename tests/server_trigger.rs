use bevy::{ecs::entity::MapEntities, prelude::*, time::TimePlugin};
use bevy_replicon::{
    prelude::*, shared::server_entity_map::ServerEntityMap, test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};

#[test]
fn regular() {
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
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.entities, [Entity::PLACEHOLDER]);
}

#[test]
fn with_target() {
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
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.world_mut().server_trigger_targets(
        ToClients {
            mode: SendMode::Broadcast,
            event: TestEvent,
        },
        server_entity,
    );

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.entities, [client_entity]);
}

#[test]
fn mapped() {
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
        .add_mapped_server_trigger::<EntityEvent>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<TriggerReader<EntityEvent>>();

    server_app.connect_client(&mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(server_entity),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<EntityEvent>>();
    let mapped_entities: Vec<_> = reader.events.iter().map(|event| event.0).collect();
    assert_eq!(mapped_entities, [client_entity]);
}

#[test]
fn without_plugins() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    server_app
        .add_plugins((
            MinimalPlugins,
            RepliconPlugins
                .build()
                .set(ServerPlugin {
                    tick_policy: TickPolicy::EveryFrame,
                    ..Default::default()
                })
                .disable::<ClientPlugin>()
                .disable::<ClientEventPlugin>(),
        ))
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerEventPlugin>(),
        ))
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        RepliconPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ))
    .add_server_trigger::<TestEvent>(Channel::Ordered)
    .finish();
    app.init_resource::<TriggerReader<TestEvent>>();

    app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    // Requires 2 updates because local resending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.events.len(), 1);
}

#[derive(Event, Serialize, Deserialize, Clone)]
struct TestEvent;

#[derive(Event, Deserialize, Serialize, Clone, MapEntities)]
struct EntityEvent(#[entities] Entity);

#[derive(Resource)]
struct TriggerReader<E: Event> {
    events: Vec<E>,
    entities: Vec<Entity>,
}

impl<E: Event + Clone> FromWorld for TriggerReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|trigger: Trigger<E>, mut counter: ResMut<Self>| {
            counter.events.push(trigger.event().clone());
            counter.entities.push(trigger.target());
        });

        Self {
            events: Default::default(),
            entities: Default::default(),
        }
    }
}
