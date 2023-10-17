mod common;

use bevy::{ecs::event::Events, prelude::*, time::TimePlugin};
use bevy_replicon::prelude::*;

use common::DummyEvent;

#[test]
fn without_server_plugin() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(ReplicationPlugins.build().disable::<ServerPlugin>())
        .add_client_event_with::<DummyEvent, _, _>(SendPolicy::Ordered, || {}, || {})
        .update();
}

#[test]
fn without_client_plugin() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(ReplicationPlugins.build().disable::<ClientPlugin>())
        .add_client_event_with::<DummyEvent, _, _>(SendPolicy::Ordered, || {}, || {})
        .update();
}

#[test]
fn sending_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, ReplicationPlugins))
            .add_client_event::<DummyEvent>(SendPolicy::Ordered);
    }

    common::connect(&mut server_app, &mut client_app);

    client_app
        .world
        .resource_mut::<Events<DummyEvent>>()
        .send(DummyEvent(Entity::PLACEHOLDER));

    client_app.update();
    server_app.update();

    let client_events = server_app
        .world
        .resource::<Events<FromClient<DummyEvent>>>();
    assert_eq!(client_events.len(), 1);
}

#[test]
fn mapping_and_sending_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, ReplicationPlugins))
            .add_mapped_client_event::<DummyEvent>(SendPolicy::Ordered);
    }

    common::connect(&mut server_app, &mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    client_app
        .world
        .resource_mut::<Events<DummyEvent>>()
        .send(DummyEvent(client_entity));

    client_app.update();
    server_app.update();

    let mapped_entities: Vec<_> = server_app
        .world
        .resource_mut::<Events<FromClient<DummyEvent>>>()
        .drain()
        .map(|event| event.event.0)
        .collect();
    assert_eq!(mapped_entities, [server_entity]);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, ReplicationPlugins))
        .add_client_event::<DummyEvent>(SendPolicy::Ordered);

    app.world
        .resource_mut::<Events<DummyEvent>>()
        .send(DummyEvent(Entity::PLACEHOLDER));

    app.update();

    let dummy_events = app.world.resource::<Events<DummyEvent>>();
    assert!(dummy_events.is_empty());

    let client_events = app.world.resource::<Events<FromClient<DummyEvent>>>();
    assert_eq!(client_events.len(), 1);
}
