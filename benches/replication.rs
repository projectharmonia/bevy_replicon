#[path = "../tests/connect/mod.rs"]
mod connect;

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use criterion::{criterion_group, criterion_main, Criterion};
use serde::{Deserialize, Serialize};
use spin_sleep::{SpinSleeper, SpinStrategy};

#[derive(Component, Clone, Copy, Serialize, Deserialize)]
struct DummyComponent(usize);

fn replication(c: &mut Criterion) {
    const ENTITIES: u32 = 1000;
    const SOCKET_WAIT: Duration = Duration::from_millis(5); // Sometimes it takes time for socket to receive all data.

    // Use spinner to keep CPU hot in the schedule for stable benchmark results.
    let sleeper = SpinSleeper::new(1_000_000_000).with_spin_strategy(SpinStrategy::SpinLoopHint);

    for clients in [1, 20] {
        c.bench_function(&format!("init send, {clients} client(s)"), |b| {
            b.iter_custom(|iter| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iter {
                    let mut server_app = create_app();
                    let mut client_apps = Vec::new();
                    for _ in 0..clients {
                        client_apps.push(create_app());
                    }
                    connect::multiple_clients(&mut server_app, &mut client_apps);

                    server_app
                        .world
                        .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);

                    let instant = Instant::now();
                    server_app.update();
                    elapsed += instant.elapsed();

                    sleeper.sleep(SOCKET_WAIT);
                    for app in &mut client_apps {
                        app.update();
                        assert_eq!(app.world.entities().len(), ENTITIES);
                    }
                }

                elapsed
            })
        });

        c.bench_function(&format!("update send, {clients} client(s)"), |b| {
            b.iter_custom(|iter| {
                let mut server_app = create_app();
                let mut client_apps = Vec::new();
                for _ in 0..clients {
                    client_apps.push(create_app());
                }
                connect::multiple_clients(&mut server_app, &mut client_apps);

                server_app
                    .world
                    .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);
                let mut query = server_app.world.query::<&mut DummyComponent>();

                server_app.update();
                sleeper.sleep(SOCKET_WAIT);
                for app in &mut client_apps {
                    app.update();
                    assert_eq!(app.world.entities().len(), ENTITIES);
                }

                let mut elapsed = Duration::ZERO;
                for _ in 0..iter {
                    for mut dummy_component in query.iter_mut(&mut server_app.world) {
                        dummy_component.0 += 1;
                    }

                    sleeper.sleep(SOCKET_WAIT);
                    let instant = Instant::now();
                    server_app.update();
                    elapsed += instant.elapsed();

                    sleeper.sleep(SOCKET_WAIT);
                    for app in &mut client_apps {
                        app.update();
                        assert_eq!(app.world.entities().len(), ENTITIES);
                    }
                }

                elapsed
            })
        });
    }

    c.bench_function("init receive", |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                let mut server_app = create_app();
                let mut client_app = create_app();
                connect::single_client(&mut server_app, &mut client_app);

                server_app
                    .world
                    .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);

                server_app.update();
                sleeper.sleep(SOCKET_WAIT);

                let instant = Instant::now();
                client_app.update();
                elapsed += instant.elapsed();
                assert_eq!(client_app.world.entities().len(), ENTITIES);
            }

            elapsed
        })
    });

    c.bench_function("update receive", |b| {
        b.iter_custom(|iter| {
            let mut server_app = create_app();
            let mut client_app = create_app();
            connect::single_client(&mut server_app, &mut client_app);

            server_app
                .world
                .spawn_batch([(Replication, DummyComponent(0)); ENTITIES as usize]);
            let mut query = server_app.world.query::<&mut DummyComponent>();

            server_app.update();
            sleeper.sleep(SOCKET_WAIT);
            client_app.update();
            assert_eq!(client_app.world.entities().len(), ENTITIES);

            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                for mut dummy_component in query.iter_mut(&mut server_app.world) {
                    dummy_component.0 += 1;
                }

                sleeper.sleep(SOCKET_WAIT);
                server_app.update();
                sleeper.sleep(SOCKET_WAIT);

                let instant = Instant::now();

                client_app.update();
                elapsed += instant.elapsed();
                assert_eq!(client_app.world.entities().len(), ENTITIES);
            }

            elapsed
        })
    });
}

fn create_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        ReplicationPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ))
    .replicate::<DummyComponent>();

    app
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = replication
}
criterion_main!(benches);
