use bevy::prelude::*;
use bevy_replicon::{
    core::replicated_archetypes::ReplicatedArchetypes, prelude::*,
    server::world_buffers::RemovalBuffer,
};
use serde::{Deserialize, Serialize};

#[test]
fn archetypes_update() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .replicate::<DummyComponent>();

    app.world.spawn((DummyComponent, Replication));
    app.world.spawn(DummyComponent);

    app.update();

    let replicated_archetypes = app.world.resource::<ReplicatedArchetypes>();
    assert_eq!(replicated_archetypes.archetypes().len(), 1);

    let replicated_archetype = replicated_archetypes.archetypes().first().unwrap();
    assert_eq!(replicated_archetype.components().len(), 1);

    let replicated_component = replicated_archetype.components().first().unwrap();
    let component_id = app.world.component_id::<DummyComponent>().unwrap();
    assert_eq!(replicated_component.component_id, component_id);
}

#[test]
fn removals() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .replicate::<DummyComponent>();

    app.world.resource_mut::<RepliconServer>().set_running(true);

    app.update();

    app.world
        .spawn((DummyComponent, Replication))
        .remove::<DummyComponent>();

    app.update();

    let removal_buffer = app.world.resource::<RemovalBuffer>();
    assert_eq!(removal_buffer.len(), 1);
}

#[test]
fn removals_ignore_despawn() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .replicate::<DummyComponent>();

    app.world.resource_mut::<RepliconServer>().set_running(true);

    app.update();

    app.world.spawn((DummyComponent, Replication)).despawn();

    app.update();

    let removal_buffer = app.world.resource::<RemovalBuffer>();
    assert!(
        removal_buffer.is_empty(),
        "despawns shouldn't be counted as removals"
    );
}

#[derive(Serialize, Deserialize, Component)]
struct DummyComponent;
