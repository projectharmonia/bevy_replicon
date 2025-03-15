use bevy::{ecs::entity::MapEntities, prelude::*, time::TimePlugin};
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
            .add_client_trigger::<DummyEvent>(ChannelKind::Ordered)
            .finish();
    }
    server_app.init_resource::<TriggerReader<DummyEvent>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(DummyEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<DummyEvent>>();
    assert_eq!(reader.entities, [Entity::PLACEHOLDER]);
}

#[test]
fn sending_receiving_with_target() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .add_client_trigger::<DummyEvent>(ChannelKind::Ordered)
            .finish();
    }
    server_app.init_resource::<TriggerReader<DummyEvent>>();

    server_app.connect_client(&mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    client_app
        .world_mut()
        .client_trigger_targets(DummyEvent, client_entity);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<DummyEvent>>();
    assert_eq!(reader.entities, [server_entity]);
}

#[test]
fn mapping_and_sending_receiving() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .add_mapped_client_trigger::<EntityEvent>(ChannelKind::Ordered)
            .finish();
    }
    server_app.init_resource::<TriggerReader<EntityEvent>>();

    server_app.connect_client(&mut client_app);

    let client_entity = Entity::from_raw(0);
    let server_entity = Entity::from_raw(client_entity.index() + 1);
    client_app
        .world_mut()
        .resource_mut::<ServerEntityMap>()
        .insert(server_entity, client_entity);

    client_app
        .world_mut()
        .client_trigger(EntityEvent(client_entity));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<EntityEvent>>();
    let mapped_entities: Vec<_> = reader.events.iter().map(|event| event.0).collect();
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
        .add_client_trigger::<DummyEvent>(ChannelKind::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerEventPlugin>(),
        ))
        .add_client_trigger::<DummyEvent>(ChannelKind::Ordered)
        .finish();
    server_app.init_resource::<TriggerReader<DummyEvent>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(DummyEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<DummyEvent>>();
    assert_eq!(reader.entities.len(), 1);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, RepliconPlugins))
        .add_client_trigger::<DummyEvent>(ChannelKind::Ordered)
        .finish();
    app.init_resource::<TriggerReader<DummyEvent>>();

    app.world_mut().client_trigger(DummyEvent);

    // Requires 2 updates because local resending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<TriggerReader<DummyEvent>>();
    assert_eq!(reader.entities.len(), 1);
}

#[derive(Deserialize, Event, Serialize, Clone)]
struct DummyEvent;

#[derive(Deserialize, Event, Serialize, Clone)]
struct EntityEvent(Entity);

impl MapEntities for EntityEvent {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.get_mapped(self.0);
    }
}

#[derive(Resource)]
struct TriggerReader<E: Event> {
    events: Vec<FromClient<E>>,
    entities: Vec<Entity>,
}

impl<E: Event + Clone> FromWorld for TriggerReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(
            |trigger: Trigger<FromClient<E>>, mut counter: ResMut<Self>| {
                counter.events.push(trigger.event().clone());
                counter.entities.push(trigger.target());
            },
        );

        Self {
            events: Default::default(),
            entities: Default::default(),
        }
    }
}
