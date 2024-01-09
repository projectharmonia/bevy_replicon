#[path = "../tests/connect/mod.rs"]
mod connect;

use std::{
    any,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use criterion::{criterion_group, criterion_main, Criterion};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spin_sleep::{SpinSleeper, SpinStrategy};

#[derive(Clone, Component, Default, Deserialize, Serialize)]
struct UsizeComponent(usize);

#[derive(Component, Clone, Serialize, Deserialize)]
struct StringComponent(String);

impl Default for StringComponent {
    fn default() -> Self {
        // Max size string before we hit Renet's message rate limit of 60Kb
        Self(".".repeat(54))
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

fn replication<C: Component + Default + Serialize + DeserializeOwned + Clone>(c: &mut Criterion) {
    const ENTITIES: u32 = 1000;
    const SOCKET_WAIT: Duration = Duration::from_millis(5); // Sometimes it takes time for socket to receive all data.

    // Use spinner to keep CPU hot in the schedule for stable benchmark results.
    let sleeper = SpinSleeper::new(1_000_000_000).with_spin_strategy(SpinStrategy::SpinLoopHint);
    let name = any::type_name::<C>();

    for clients in [1, 20] {
        c.bench_function(&format!("{name}, init send, {clients} client(s)"), |b| {
            b.iter_custom(|iter| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iter {
                    let mut server_app = create_app::<C>();
                    let mut client_apps = Vec::new();
                    for _ in 0..clients {
                        client_apps.push(create_app::<C>());
                    }
                    connect::multiple_clients(&mut server_app, &mut client_apps);

                    server_app
                        .world
                        .spawn_batch(vec![(Replication, C::default()); ENTITIES as usize]);

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

        c.bench_function(&format!("{name}, update send, {clients} client(s)"), |b| {
            b.iter_custom(|iter| {
                let mut server_app = create_app::<C>();
                let mut client_apps = Vec::new();
                for _ in 0..clients {
                    client_apps.push(create_app::<C>());
                }
                connect::multiple_clients(&mut server_app, &mut client_apps);

                server_app
                    .world
                    .spawn_batch(vec![(Replication, C::default()); ENTITIES as usize]);
                let mut query = server_app.world.query::<&mut C>();

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

    c.bench_function(&format!("{name}, init receive"), |b| {
        b.iter_custom(|iter| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iter {
                let mut server_app = create_app::<C>();
                let mut client_app = create_app::<C>();
                connect::single_client(&mut server_app, &mut client_app);

                server_app
                    .world
                    .spawn_batch(vec![(Replication, C::default()); ENTITIES as usize]);

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

    c.bench_function(&format!("{name}, update receive"), |b| {
        b.iter_custom(|iter| {
            let mut server_app = create_app::<C>();
            let mut client_app = create_app::<C>();
            connect::single_client(&mut server_app, &mut client_app);

            server_app
                .world
                .spawn_batch(vec![(Replication, C::default()); ENTITIES as usize]);
            let mut query = server_app.world.query::<&mut C>();

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

fn create_app<C: Component + Serialize + DeserializeOwned>() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        ReplicationPlugins.set(ServerPlugin {
            tick_policy: TickPolicy::EveryFrame,
            ..Default::default()
        }),
    ))
    .replicate::<C>();

    app
}

criterion_group! {
    name = int_benches;
    config = Criterion::default().sample_size(20);
    targets = replication::<UsizeComponent>
}
criterion_group! {
    name = string_benches;
    config = Criterion::default().sample_size(20);
    targets = replication::<StringComponent>
}
criterion_group! {
    name = struct_benches;
    config = Criterion::default().sample_size(20);
    targets = replication::<StructComponent>
}
criterion_main!(int_benches, string_benches, struct_benches);
