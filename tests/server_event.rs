use bevy::{
    ecs::{entity::MapEntities, event::Events},
    prelude::*,
    time::TimePlugin,
};
use bevy_replicon::{
    client::ServerUpdateTick,
    prelude::*,
    server::server_tick::ServerTick,
    shared::{
        event::remote_event_registry::RemoteEventRegistry, server_entity_map::ServerEntityMap,
    },
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};

#[test]
fn channels() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        RepliconPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ))
    .add_event::<NonRemoteEvent>()
    .add_server_event::<DummyEvent>(Channel::Ordered)
    .finish();

    let event_registry = app.world().resource::<RemoteEventRegistry>();
    assert_eq!(event_registry.server_channel::<NonRemoteEvent>(), None);
    assert_eq!(event_registry.server_channel::<DummyEvent>(), Some(2));
}

#[test]
fn sending_receiving() {
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(SERVER), 0),
        (SendMode::Direct(test_client_entity), 1),
        (SendMode::BroadcastExcept(SERVER), 1),
        (SendMode::BroadcastExcept(test_client_entity), 0),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: DummyEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let mut events = client_app.world_mut().resource_mut::<Events<DummyEvent>>();
        assert_eq!(
            events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
}

#[test]
fn sending_receiving_and_mapping() {
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
        .add_mapped_server_event::<EntityEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(server_entity),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mapped_entities: Vec<_> = client_app
        .world_mut()
        .resource_mut::<Events<EntityEvent>>()
        .drain()
        .map(|event| event.0)
        .collect();
    assert_eq!(mapped_entities, [client_entity]);
}

#[test]
fn sending_receiving_without_plugins() {
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerEventPlugin>(),
        ))
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();

    server_app.connect_client(&mut client_app);

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(SERVER), 0),
        (SendMode::Direct(test_client_entity), 1),
        (SendMode::BroadcastExcept(SERVER), 1),
        (SendMode::BroadcastExcept(test_client_entity), 0),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: DummyEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let mut events = client_app.world_mut().resource_mut::<Events<DummyEvent>>();
        assert_eq!(
            events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
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
    .add_server_event::<DummyEvent>(Channel::Ordered)
    .finish();

    const DUMMY_CLIENT_ID: Entity = Entity::from_raw(1);
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(SERVER), 1),
        (SendMode::Direct(DUMMY_CLIENT_ID), 0),
        (SendMode::BroadcastExcept(SERVER), 0),
        (SendMode::BroadcastExcept(DUMMY_CLIENT_ID), 1),
    ] {
        app.world_mut().send_event(ToClients {
            mode,
            event: DummyEvent,
        });

        app.update();

        let server_events = app.world().resource::<Events<ToClients<DummyEvent>>>();
        assert!(server_events.is_empty());

        let mut dummy_events = app.world_mut().resource_mut::<Events<DummyEvent>>();
        assert_eq!(
            dummy_events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
}

#[test]
fn event_buffering() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::Manual, // To artificially delay replication after sending.
                ..Default::default()
            }),
        ))
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<DummyEvent>>();
    assert!(events.is_empty(), "event should be buffered on server");

    // Trigger replication.
    server_app
        .world_mut()
        .resource_mut::<ServerTick>()
        .increment();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<DummyEvent>>();
    assert_eq!(events.len(), 1);
}

#[test]
fn event_queue() {
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick to force the next received event to be queued.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Events<DummyEvent>>();
    assert!(events.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    assert_eq!(client_app.world().resource::<Events<DummyEvent>>().len(), 1);
}

#[test]
fn event_queue_and_mapping() {
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
        .add_mapped_server_event::<EntityEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn an entity to trigger world change.
    let server_entity = server_app.world_mut().spawn(Replicated).id();
    let client_entity = client_app.world_mut().spawn_empty().id();
    assert_ne!(server_entity, client_entity);

    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick to force the next received event to be queued.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(server_entity),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Events<EntityEvent>>();
    assert!(events.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    let mapped_entities: Vec<_> = client_app
        .world_mut()
        .resource_mut::<Events<EntityEvent>>()
        .drain()
        .map(|event| event.0)
        .collect();
    assert_eq!(mapped_entities, [client_entity]);
}

#[test]
fn multiple_event_queues() {
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .add_server_event::<EntityEvent>(Channel::Ordered) // Use as a regular event with a different serialization size.
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick to force the next received event to be queued.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(Entity::PLACEHOLDER),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Events<DummyEvent>>();
    assert!(events.is_empty());

    let mapped_events = client_app.world().resource::<Events<EntityEvent>>();
    assert!(mapped_events.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    assert_eq!(client_app.world().resource::<Events<DummyEvent>>().len(), 1);
    assert_eq!(
        client_app.world().resource::<Events<EntityEvent>>().len(),
        1
    );
}

#[test]
fn independent() {
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .make_independent::<DummyEvent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick.
    // Normal events would be queued and not triggered yet,
    // but our independent event should be triggered immediately.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = Default::default();

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(SERVER), 0),
        (SendMode::Direct(test_client_entity), 1),
        (SendMode::BroadcastExcept(SERVER), 1),
        (SendMode::BroadcastExcept(test_client_entity), 0),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: DummyEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        // Event should have already been triggered, even without resetting the tick,
        // since it's independent.
        let mut events = client_app.world_mut().resource_mut::<Events<DummyEvent>>();
        assert_eq!(
            events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    for mode in [
        SendMode::Broadcast,
        SendMode::BroadcastExcept(SERVER),
        SendMode::Direct(test_client_entity),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: DummyEvent,
        });
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<DummyEvent>>();
    assert!(events.is_empty());
}

#[test]
fn independent_before_started_replication() {
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
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .make_independent::<DummyEvent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert_eq!(client_app.world().resource::<Events<DummyEvent>>().len(), 1);
}

#[test]
fn different_ticks() {
    let mut server_app = App::new();
    let mut client_app1 = App::new();
    let mut client_app2 = App::new();
    for app in [&mut server_app, &mut client_app1, &mut client_app2] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .add_server_event::<DummyEvent>(Channel::Ordered)
        .finish();
    }

    // Connect client 1 first.
    server_app.connect_client(&mut client_app1);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    // Update client 1 to initialize their replicon tick.
    server_app.update();
    server_app.exchange_with_client(&mut client_app1);
    client_app1.update();
    server_app.exchange_with_client(&mut client_app1);

    // Connect client 2 later to make it have a higher replicon tick than client 1,
    // since only client 1 will receive a update message here.
    server_app.connect_client(&mut client_app2);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });

    // If any client does not have a replicon tick >= the update tick associated with this event,
    // then they will not receive the event until their replicon tick is updated.
    server_app.update();
    server_app.exchange_with_client(&mut client_app1);
    server_app.exchange_with_client(&mut client_app2);
    client_app1.update();
    client_app2.update();

    assert_eq!(
        client_app1.world().resource::<Events<DummyEvent>>().len(),
        1
    );
    assert_eq!(
        client_app2.world().resource::<Events<DummyEvent>>().len(),
        1
    );
}

#[derive(Event)]
struct NonRemoteEvent;

#[derive(Deserialize, Event, Serialize)]
struct DummyEvent;

#[derive(Deserialize, Event, Serialize)]
struct EntityEvent(Entity);

impl MapEntities for EntityEvent {
    fn map_entities<T: EntityMapper>(&mut self, entity_mapper: &mut T) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}
