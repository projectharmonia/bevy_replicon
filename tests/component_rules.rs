use bevy::prelude::*;
use bevy_replicon::{prelude::*, server::world_buffers::RemovalBuffer};
use serde::{Deserialize, Serialize};

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
