#[path = "../tests/connect/mod.rs"]
mod connect;

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use criterion::{criterion_group, criterion_main, Criterion};
use serde::{Deserialize, Serialize};
use spin_sleep::{SpinSleeper, SpinStrategy};

#[derive(Component, Clone, Serialize, Deserialize)]
struct UintComponent(usize);

impl Default for UintComponent {
    fn default() -> Self {
        Self(0)
    }
}

#[derive(Component, Clone, Serialize, Deserialize)]
struct StringComponent(String);

impl Default for StringComponent {
    fn default() -> Self {
        // note: this is the max size string before we hit renet's message rate limit of 60Kb
        Self(String::from(
            ".......................................................",
        ))
    }
}

#[derive(Component, Clone, Serialize, Deserialize)]
struct StructComponent {
    x: u32,
    y: u32,
    b: f32,
    a: f32,
    n: String,
}

impl Default for StructComponent {
    fn default() -> Self {
        Self {
            x: 22u32,
            y: 22u32,
            b: 1.5f32,
            a: 20.0f32,
            n: String::from("abcdef123"),
        }
    }
}

fn replication<T: Component + Default + Serialize + for<'de> Deserialize<'de> + Clone>(
    c: &mut Criterion,
    t: &'static str,
) {
    const ENTITIES: u32 = 1000;
    const SOCKET_WAIT: Duration = Duration::from_millis(5); // Sometimes it takes time for socket to receive all data.

    // Use spinner to keep CPU hot in the schedule for stable benchmark results.
    let sleeper = SpinSleeper::new(1_000_000_000).with_spin_strategy(SpinStrategy::SpinLoopHint);

    for clients in [1, 20] {
        c.bench_function(&format!("[{t}]: init send, {clients} client(s)"), |b| {
            b.iter_custom(|iter| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iter {
                    let mut server_app = create_app::<T>();
                    let mut client_apps = Vec::new();
                    for _ in 0..clients {
                        client_apps.push(create_app::<T>());
                    }
                    connect::multiple_clients(&mut server_app, &mut client_apps);

                    server_app
                        .world
                        .spawn_batch(vec![(Replication, T::default()); ENTITIES as usize]);

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

        c.bench_function(&format!("[{t}]: update send, {clients} client(s)"), |b| {
            b.iter_custom(|iter| {
                let mut server_app = create_app::<T>();
                let mut client_apps = Vec::new();
                for _ in 0..clients {
                    client_apps.push(create_app::<T>());
                }
                connect::multiple_clients(&mut server_app, &mut client_apps);

                server_app
                    .world
                    .spawn_batch(vec![(Replication, T::default()); ENTITIES as usize]);
                let mut query = server_app.world.query::<&mut T>();

                server_app.update();
                sleeper.sleep(SOCKET_WAIT);
                for app in &mut client_apps {
                    app.update();
                    assert_eq!(app.world.entities().len(), ENTITIES);
                }

                let mut elapsed = Duration::ZERO;
                for _ in 0..iter {
                    for component in query.iter_mut(&mut server_app.world) {
                        component.into_inner();
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

    c.bench_function(&format!("[{t}]: init receive"), |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                let mut server_app = create_app::<T>();
                let mut client_app = create_app::<T>();
                connect::single_client(&mut server_app, &mut client_app);

                server_app
                    .world
                    .spawn_batch(vec![(Replication, T::default()); ENTITIES as usize]);

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

    c.bench_function(&format!("[{t}]: update receive"), |b| {
        b.iter_custom(|iter| {
            let mut server_app = create_app::<T>();
            let mut client_app = create_app::<T>();
            connect::single_client(&mut server_app, &mut client_app);

            server_app
                .world
                .spawn_batch(vec![(Replication, T::default()); ENTITIES as usize]);
            let mut query = server_app.world.query::<&mut T>();

            server_app.update();
            sleeper.sleep(SOCKET_WAIT);
            client_app.update();
            assert_eq!(client_app.world.entities().len(), ENTITIES);

            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                for component in query.iter_mut(&mut server_app.world) {
                    component.into_inner();
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

fn create_app<T: Component + Default + Serialize + for<'de> Deserialize<'de> + Clone>() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        ReplicationPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ))
    .replicate::<T>();

    app
}

fn int_replication(c: &mut Criterion) {
    replication::<UintComponent>(c, "uint");
}

fn string_replication(c: &mut Criterion) {
    replication::<StringComponent>(c, "string");
}

fn struct_replication(c: &mut Criterion) {
    replication::<StructComponent>(c, "struct");
}

criterion_group! {
    name = int_benches;
    config = Criterion::default().sample_size(20);
    targets = int_replication
}
criterion_group! {
    name = string_benches;
    config = Criterion::default().sample_size(20);
    targets = string_replication
}
criterion_group! {
    name = struct_benches;
    config = Criterion::default().sample_size(20);
    targets = struct_replication
}
criterion_main!(int_benches, string_benches, struct_benches);
