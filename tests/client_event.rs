use bevy::{
    ecs::{entity::MapEntities, event::Events},
    prelude::*,
    time::TimePlugin,
};
use bevy_replicon::{
    core::server_entity_map::ServerEntityMap, prelude::*, test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};

#[test]
fn sending_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .add_client_event::<DummyEvent>(ChannelKind::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app.world_mut().send_event(DummyEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let client_events = server_app
        .world()
        .resource::<Events<FromClient<DummyEvent>>>();
    assert_eq!(client_events.len(), 1);
}

#[test]
fn mapping_and_sending_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .add_mapped_client_event::<MappedEvent>(ChannelKind::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    client_app
        .world_mut()
        .send_event(MappedEvent(client_entity));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let mapped_entities: Vec<_> = server_app
        .world_mut()
        .resource_mut::<Events<FromClient<MappedEvent>>>()
        .drain()
        .map(|event| event.event.0)
        .collect();
    assert_eq!(mapped_entities, [server_entity]);
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
                .disable::<ClientPlugin>()
                .disable::<ClientEventPlugin>(),
        ))
        .add_client_event::<DummyEvent>(ChannelKind::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerEventPlugin>(),
        ))
        .add_client_event::<DummyEvent>(ChannelKind::Ordered)
        .finish();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().send_event(DummyEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let client_events = server_app
        .world()
        .resource::<Events<FromClient<DummyEvent>>>();
    assert_eq!(client_events.len(), 1);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, RepliconPlugins))
        .add_client_event::<DummyEvent>(ChannelKind::Ordered)
        .finish();

    app.world_mut().send_event(DummyEvent);

    app.update();

    let dummy_events = app.world().resource::<Events<DummyEvent>>();
    assert!(dummy_events.is_empty());

    let client_events = app.world().resource::<Events<FromClient<DummyEvent>>>();
    assert_eq!(client_events.len(), 1);
}

#[derive(Deserialize, Event, Serialize)]
struct DummyEvent;

#[derive(Deserialize, Event, Serialize, Clone)]
struct MappedEvent(Entity);

impl MapEntities for MappedEvent {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}
