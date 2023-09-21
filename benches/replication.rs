#[path = "../tests/common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use bevy::{app::MainScheduleOrder, ecs::schedule::ExecutorKind, prelude::*};
use bevy_replicon::prelude::*;
use criterion::{criterion_group, criterion_main, Criterion};
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Copy, Serialize, Deserialize)]
struct DummyComponent(usize);

const ENTITIES: u32 = 200;

fn replication(c: &mut Criterion) {
    c.bench_function("entities send", |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                let mut server_app = App::new();
                let mut client_app = App::new();
                for app in [&mut server_app, &mut client_app] {
                    setup_app(app);
                }
                common::connect(&mut server_app, &mut client_app);

                server_app
                    .world
                    .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);

                let instant = Instant::now();
                server_app.update();
                elapsed += instant.elapsed();

                client_app.update();
                assert_eq!(client_app.world.entities().len(), ENTITIES);
            }

            elapsed
        })
    });

    c.bench_function("entities receive", |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                let mut server_app = App::new();
                let mut client_app = App::new();
                for app in [&mut server_app, &mut client_app] {
                    setup_app(app);
                }
                common::connect(&mut server_app, &mut client_app);

                server_app
                    .world
                    .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);

                server_app.update();

                let instant = Instant::now();
                client_app.update();
                elapsed += instant.elapsed();
                assert_eq!(client_app.world.entities().len(), ENTITIES);
            }

            elapsed
        })
    });

    c.bench_function("entities update send", |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            let mut server_app = App::new();
            let mut client_app = App::new();
            for app in [&mut server_app, &mut client_app] {
                setup_app(app);
            }
            common::connect(&mut server_app, &mut client_app);

            server_app
                .world
                .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);
            let mut query = server_app.world.query::<&mut DummyComponent>();

            server_app.update();
            client_app.update();
            assert_eq!(client_app.world.entities().len(), ENTITIES);

            for _ in 0..iter {
                for mut dummy_component in query.iter_mut(&mut server_app.world) {
                    dummy_component.0 += 1;
                }

                let instant = Instant::now();
                server_app.update();
                elapsed += instant.elapsed();

                client_app.update();
                assert_eq!(client_app.world.entities().len(), ENTITIES);
            }

            elapsed
        })
    });

    c.bench_function("entities update receive", |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            let mut server_app = App::new();
            let mut client_app = App::new();
            for app in [&mut server_app, &mut client_app] {
                setup_app(app);
            }
            common::connect(&mut server_app, &mut client_app);

            server_app
                .world
                .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);
            let mut query = server_app.world.query::<&mut DummyComponent>();

            server_app.update();
            client_app.update();
            assert_eq!(client_app.world.entities().len(), ENTITIES);

            for _ in 0..iter {
                for mut dummy_component in query.iter_mut(&mut server_app.world) {
                    dummy_component.0 += 1;
                }

                server_app.update();

                let instant = Instant::now();
                client_app.update();
                elapsed += instant.elapsed();
                assert_eq!(client_app.world.entities().len(), ENTITIES);
            }

            elapsed
        })
    });
}

fn setup_app(app: &mut App) {
    app.add_plugins((
        MinimalPlugins,
        ReplicationPlugins.set(ServerPlugin::new(TickPolicy::Manual)),
    ))
    .replicate::<DummyComponent>();

    // TODO 0.12: Probably won't be needed since `multi-threaded` feature will be disabled by default.
    let labels = app.world.resource::<MainScheduleOrder>().labels.clone();
    for label in labels {
        app.edit_schedule(label, |schedule| {
            schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        });
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = replication
}
criterion_main!(benches);