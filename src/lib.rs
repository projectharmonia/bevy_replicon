/*!
ECS-focused high-level networking crate for the [Bevy game engine](https://bevyengine.org).

# Quick start

We provide a [`prelude`] module, which exports most of the typically used traits and types.

The library doesn't provide any I/O, so you need to add a
[messaging backend](https://github.com/projectharmonia/bevy_replicon#messaging-backends).
If you want to write an integration for a messaging backend,
see the documentation for [`RepliconServer`], [`RepliconClient`] and [`ConnectedClient`].
You can also use `bevy_replicon_renet`, which we maintain, as a reference.

Also depending on your game, you may want to use additional crates. For example, if your game
is fast-paced, you will need interpolation and rollback.
For details see [`goals`](https://github.com/projectharmonia/bevy_replicon#goals) and
[`related crates`](https://github.com/projectharmonia/bevy_replicon#related-crates).
Before adding advanced functionality, it's recommended to read the quick start guide
first to understand the basics.

## API showcase

```
# use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

let mut app = App::new();
app.add_plugins((
    MinimalPlugins,
    RepliconPlugins,
    MyMessagingPlugins, // Plugins for your messaging backend of choice.
))
.add_systems(
    PreUpdate,
    (
        // Run systems that read events right after receiving them.
        // But it can be done in any other place.
        apply_movement
            .after(ClientSet::Receive)
            .run_if(client_connected),
        show_message
            .after(ServerSet::Receive)
            .run_if(server_running),
    ),
)
.add_systems(
    Update,
    (spawn_entities, update_health).run_if(server_running),
)
.add_systems(
    PostUpdate,
    (
        // Run systems that write events right before sending them.
        // But it can be done in any other place.
        send_movement
            .before(ClientSet::Send)
            .run_if(client_connected),
        send_message
            .before(ServerSet::Send)
            .run_if(server_running),
    ),
)
.replicate::<Health>() // Component that will be replicated.
.replicate_group::<(Transform, Player)>() // Replicate multiple components only if all of them are present.
.add_client_event::<MovementEvent>(ChannelKind::Ordered) // Bevy event that will replicated from clients to server.
.add_server_event::<MessageEvent>(ChannelKind::Unordered); // Bevy event that will replicated from server to client.

fn spawn_entities(mut commands: Commands) {
    // All entities with `Replicated` marker will be automatically replicated.
    commands.spawn((
        Replicated,
        Health(100),
        Transform::default(),
        Player,
        NotReplicatedComponent, // This component will be ignored since it's not replicated for replication.
    ));

    // `Transform` won't be replicated in this case since `Player` marker is missing.
    commands.spawn((Replicated, Health(100), Transform::default()));
}

fn update_health(mut players: Query<&mut Health, With<Player>>) {
    // Changed values on server will be automatically replicated to clients.
    for mut health in &mut players {
        health.0 += 1;
    }
}

fn send_movement(mut movement_events: EventWriter<MovementEvent>) {
    // This event will be available on server, but in form of
    // `FromClient<MovementEvent>` to include the sender ID.
    movement_events.send(MovementEvent(Vec2::ONE));
}

fn apply_movement(mut movement_events: EventReader<FromClient<MovementEvent>>) {
    for FromClient { client_entity, event } in movement_events.read() {
        // Apply user inputs to entities.
        // Since it runs on server, all changes will be replicated back to clients.
    }
}

fn send_message(mut message_events: EventWriter<ToClients<MessageEvent>>) {
    // This event will be available on clients, but in form of
    // just `MessageEvent`. On server we use `ToClients` wrapper to include `mode`.
    message_events.send(ToClients {
        mode: SendMode::Broadcast,
        event: MessageEvent("Hello from server".to_string()),
    });
}

fn show_message(mut message_events: EventReader<MessageEvent>) {
    for event in message_events.read() {
        // Process the message, show in UI, etc...
    }
}

#[derive(Component, Serialize, Deserialize)]
struct Health(u32);

#[derive(Component, Serialize, Deserialize)]
struct Player;

#[derive(Component)]
struct NotReplicatedComponent;

#[derive(Event, Serialize, Deserialize)]
struct MovementEvent(Vec2);

#[derive(Event, Serialize, Deserialize)]
struct MessageEvent(String);
#
# struct MyMessagingPlugins;
#
# impl PluginGroup for MyMessagingPlugins {
#     fn build(self) -> PluginGroupBuilder {
#         PluginGroupBuilder::start::<Self>()
#     }
# }
```

This example shows a server and client logic inside a single app managed by
[run conditions](#system-sets-and-conditions). But it's possible to split server and client into
multiple apps if needed. Seamless singleplayer and listen-server mode (when server is also a client)
are also supported by just adjusting the run conditions.

Below we describe each part and more advanced features in more detail.

## Initialization

You need to add [`RepliconPlugins`] and plugins for your chosen messaging backend to your app:

```
use bevy::prelude::*;
use bevy_replicon::prelude::*;
# use bevy::app::PluginGroupBuilder;

let mut app = App::new();
app.add_plugins((MinimalPlugins, RepliconPlugins, MyMessagingPlugins));
#
# struct MyMessagingPlugins;
#
# impl PluginGroup for MyMessagingPlugins {
#     fn build(self) -> PluginGroupBuilder {
#         PluginGroupBuilder::start::<Self>()
#     }
# }
```

If you want to separate the client and server, you can use the `client` and `server` features
(both enabled by default), which control enabled plugins.

It's also possible to do it at runtime via [`PluginGroupBuilder::disable()`].
For server disable [`ClientPlugin`] and [`ClientEventPlugin`].
For client disable [`ServerPlugin`] and [`ServerEventPlugin`].

You will need to disable similar features or plugins on your messaing library of choice too.

Typically updates are not sent every frame. Instead, they are sent at a certain interval
to save traffic. You can change the defaults with [`TickPolicy`] in the [`ServerPlugin`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
app.add_plugins(
    RepliconPlugins
        .build()
        .disable::<ClientPlugin>()
        .set(ServerPlugin {
            tick_policy: TickPolicy::MaxTickRate(60),
            ..Default::default()
        }),
);
```

Depending on the game, you may notice that the lower the interval, the less smooth the game feels.
To smooth updates, you will need to apply interpolation.

## Server and client creation

This part is customized based on your messaging backend. For `bevy_replicon_renet`
see [this](https://docs.rs/bevy_replicon_renet#server-and-client-creation) section.

The backend will automatically update the [`RepliconServer`] or [`RepliconClient`] resources, which
can be interacted with without knowing what backend is used. Those resources typically don't need to
be used directly, it is preferred to use more high-level abstractions described later.

<div class="warning">

Never initialize a client and server in the same app for singleplayer; doing so will cause a replication loop.
Use the described pattern in [system sets and conditions](#system-sets-and-conditions)
in combination with [network events](#network-events) instead.

</div>

On server connected clients represented as entities with [`ConnectedClient`] component.

## System sets and conditions

To run a system based on a network condition, use the [`core::common_conditions`] module.
This module is also available from [`prelude`].

This way you can run specific systems only on server ([`server_running`]) or
only on client ([`client_connected`]). To display a "connecting" message, you can use [`client_connecting`].

If your game needs singleplayer or listen-server mode (when server is also a client),
just use [`server_or_singleplayer`] instead of [`server_running`] and remove all [`client_connected`].
No other changes needed. We will describe later what replicon does internally to achieve it.

We also provide [`ClientSet`] and [`ServerSet`] to schedule your system at specific time in the frame.
For example, you most likely want to react on receive after [`ClientSet::Receive`] or [`ServerSet::Receive`].

## Replication

It's a process of sending changes from server to clients in order to
keep the world in sync.

To prevent cheating, we do not support replicating from the client. If you need to send
information from clients to the server, use [events](#network-events).

Replication is enabled by default for all connected clients via [`ReplicatedClient`] component.
It can be disabled via [`ServerPlugin::replicate_after_connect`] is set to `false`.

### Marking for replication

By default nothing is replicated. User needs to choose which entities
and components need to be replicated.

#### Entities

By default no entities are replicated. Add the [`Replicated`] marker
component on the server for entities you want to replicate.

On clients [`Replicated`] will be automatically inserted to newly-replicated entities.

If you remove the [`Replicated`] component from an entity on the server, it will be despawned on all clients.

#### Components

Components will be replicated only on entities marked for replication.
By default no components are replicated.

Use [`AppRuleExt::replicate()`] to enable replication for a component:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.replicate::<DummyComponent>();

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;
```

If your component contains an entity then it cannot be deserialized as is
because entity IDs are different on server and client. The client should do the
mapping. Therefore, to replicate such components properly, they need to implement
the [`MapEntities`](bevy::ecs::entity::MapEntities) trait and register
using [`AppRuleExt::replicate_mapped()`].

By default all components are serialized with [`postcard`].
If your component doesn't implement serde traits or you want to serialize it partially
(for example, only replicate the `translation` field from [`Transform`]),
you can use [`AppRuleExt::replicate_with`].

If you want a group of components to be replicated only if all of them are present on an entity,
you can use [`AppRuleExt::replicate_group`].

If you want to customize how the received component will be written or removed on clients based
on some marker component (for example, write into a different component), see [`AppMarkerExt`].
Useful for implementing rollback and interpolation.

In order to serialize Bevy components you need to enable the `serialize` feature on Bevy.

<div class="warning">

If you are planning to have separate apps for the client and server, make sure that the component
registration order is the same on both.

Typically, in this setup, you have a "shared" crate that contains type definitions and possibly some logic.
This is also where you want to add all component registrations.

</div>

### Mapping to existing client entities

If you want the server to replicate an entity into a client entity that was already spawned on a client, see [`ClientEntityMap`].

This can be useful for certain types of game. For example, spawning bullets on the client immediately without
waiting on replication.

### Required components

You don't want to replicate all components because not all of them are
necessary to send over the network. For example, 'background' components
can be automatically inserted after replication thanks to Bevy's required components.
For components that require world access you can create a special system that inserts such
components after entity spawn. To avoid one frame delay, put your initialization systems
in [`ClientSet::Receive`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
// Replicate only transform and player marker.
app.replicate::<Transform>()
    .replicate::<Player>()
    .add_systems(PreUpdate, init_player_mesh.after(ClientSet::Receive));

fn init_player_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    // Infer that the player was just spawned by the fact it's missing `Mesh2d`.
    players: Query<Entity, (With<Player>, Without<Mesh2d>)>,
) {
    for entity in &players {
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Capsule2d::default())),
        ));
    }
}

/// Main player component.
///
/// [`NotReplicatedComponent`] and [`Replicated`] will be implicitly inserted after spawning on
/// both the server and clients.
///
/// [`Replicated`] is always inserted on the client after replication, regardless of whether it is marked
/// as required. However, it may still be useful to mark it as required if you want to avoid
/// inserting it explicitly on the server.
#[derive(Component, Deserialize, Serialize)]
#[require(Replicated, NotReplicatedComponent)]
struct Player;

#[derive(Default, Component)]
struct NotReplicatedComponent;
```

This pairs nicely with server state serialization and keeps saves clean.
You can use [`replicate_into`](scene::replicate_into) to
fill [`DynamicScene`] with replicated entities and their components.
On deserialization all missing required components will be inserted, and initialization
systems will restore the correct game state.

**Performance note**: We used [`With<Player>`] and [`Without<Mesh2d>`] to
filter all non-initialized entities. It's possible to use [`Added`] / [`Changed`] too,
but they aren't true archetype-level filters like [`With`] or [`Without`].
See [the Bevy docs](https://docs.rs/bevy/latest/bevy/ecs/prelude/struct.Added.html#time-complexity)
for more details. There is also an [open Bevy ticket](https://github.com/bevyengine/bevy/issues/5097)
for improving the performance of [`Added`] / [`Changed`].

### Component relations

Sometimes components depend on each other. For example, [`Parent`] and
[`Children`] In this case, you can't just replicate the [`Parent`] because you
not only need to add it to the [`Children`] of the parent, but also remove it
from the [`Children`] of the old one. In this case, you need to create a third
component that correctly updates the other two when it changes, and only
replicate that one. This crate provides [`ParentSync`] component that replicates
Bevy hierarchy. For your custom components with relations you need to write your
own with a similar pattern.

## Network events and triggers

This replaces RPCs (remote procedure calls) in other engines and,
unlike components, can be sent both from server to clients and from clients to
server.

### From client to server

To send specific events from client to server, you need to register the event
with [`ClientEventAppExt::add_client_event()`] instead of [`App::add_event()`].

Events include [`ChannelKind`] to configure delivery guarantees (reliability and
ordering). You can alternatively pass in [`RepliconChannel`] with more advanced configuration.

These events will appear on server as [`FromClient`] wrapper event that
contains sender ID and the sent event.

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_client_event::<DummyEvent>(ChannelKind::Ordered)
    .add_systems(
        PreUpdate,
        receive_events
            .after(ServerSet::Receive)
            .run_if(server_running),
    )
    .add_systems(
        PostUpdate,
        send_events.before(ClientSet::Send).run_if(client_connected),
    );

fn send_events(mut dummy_events: EventWriter<DummyEvent>) {
    dummy_events.send_default();
}

fn receive_events(mut dummy_events: EventReader<FromClient<DummyEvent>>) {
    for FromClient { client_entity, event } in dummy_events.read() {
        info!("received event `{event:?}` from client `{client_entity}`");
    }
}

#[derive(Debug, Default, Deserialize, Event, Serialize)]
struct DummyEvent;
```

We consider the server or a singleplayer session also as a client with ID [`SERVER`].
So you can send such events even on server and [`FromClient`] will be emitted for them too.

If you remove [`client_connected`] condition and replace [`server_running`] with
[`server_or_singleplayer`], your game logic will work the same on client, listen server,
and in singleplayer session.

Just like components, if an event contains an entity, then the client should
map it before sending it to the server.
To do this, use [`ClientEventAppExt::add_mapped_client_event()`] and implement
[`MapEntities`](bevy::ecs::entity::MapEntities):

```
# use bevy::{prelude::*, ecs::entity::MapEntities};
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_mapped_client_event::<MappedEvent>(ChannelKind::Ordered);

#[derive(Debug, Deserialize, Event, Serialize, Clone)]
struct MappedEvent(Entity);

impl MapEntities for MappedEvent {
    fn map_entities<T: EntityMapper>(&mut self, entity_mapper: &mut T) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}
```

As shown above, mapped client events must also implement [`Clone`].

There is also [`ClientEventAppExt::add_client_event_with()`] to register an event with special serialization and
deserialization functions. This could be used for sending events that contain [`Box<dyn PartialReflect>`], which
require access to the [`AppTypeRegistry`] resource.

Don't forget to validate the contents of every [`Box<dyn PartialReflect>`] from a client, it could be anything!

Alternatively you can use triggers with similar API. First, you need to register the event
with [`ClientTriggerAppExt::add_client_trigger()`] and then use [`ClientTriggerExt::client_trigger`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_client_trigger::<DummyEvent>(ChannelKind::Ordered)
    .add_observer(receive_events)
    .add_systems(Update, send_events.run_if(client_connected));

fn send_events(mut commands: Commands) {
    commands.client_trigger(DummyEvent);
}

fn receive_events(trigger: Trigger<FromClient<DummyEvent>>) {
    info!("received event `{:?}` from client `{}`", **trigger, trigger.client_entity);
}
# #[derive(Event, Debug, Deserialize, Serialize)]
# struct DummyEvent;
```

### From server to client

A similar technique is used to send events from server to clients. To do this,
register the event with [`ServerEventAppExt::add_server_event()`]
and send it from server using [`ToClients`]. This wrapper contains send parameters
and the event itself.

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_server_event::<DummyEvent>(ChannelKind::Ordered)
    .add_systems(
        PreUpdate,
        receive_events
            .after(ClientSet::Receive)
            .run_if(client_connected),
    )
    .add_systems(
        PostUpdate,
        send_events.before(ServerSet::Send).run_if(server_running),
    );

fn send_events(mut dummy_events: EventWriter<ToClients<DummyEvent>>) {
    dummy_events.send(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });
}

fn receive_events(mut dummy_events: EventReader<DummyEvent>) {
    for event in dummy_events.read() {
        info!("received event {event:?} from server");
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Event, Serialize)]
struct DummyEvent;
```

Just like events sent from the client, you can send these events on the server or
in singleplayer and they will appear locally as regular events (if [`SERVER`] is not excluded
from the send list). So the same trick with run conditions will work.

If the event contains an entity, then
[`ServerEventAppExt::add_mapped_server_event()`] should be used instead.

For events that require special serialization and deserialization functions you can use
[`ServerEventAppExt::add_server_event_with()`].

Trigger-based API available for server events as well. First, you need to register the event
with [`ServerTriggerAppExt::add_server_trigger()`] and then use [`ServerTriggerExt::server_trigger`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_server_trigger::<DummyEvent>(ChannelKind::Ordered)
    .add_observer(receive_events)
    .add_systems(Update, send_events.run_if(server_running));

fn send_events(mut commands: Commands) {
    commands.server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });
}

fn receive_events(trigger: Trigger<DummyEvent>) {
    info!("received event {:?} from server", *trigger);
}
# #[derive(Event, Debug, Deserialize, Serialize)]
# struct DummyEvent;
```

<div class="warning">

Just like with components, all networked events should be registered in the same order.

</div>

## Client visibility

You can control which parts of the world are visible for each client by setting visibility policy
in [`ServerPlugin`] to [`VisibilityPolicy::Whitelist`] or [`VisibilityPolicy::Blacklist`].

In order to set which entity is visible, you need to use the [`ClientVisibility`] component
on replicated clients.

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
app.add_plugins((
    MinimalPlugins,
    RepliconPlugins.set(ServerPlugin {
        visibility_policy: VisibilityPolicy::Whitelist, // Makes all entities invisible for clients by default.
        ..Default::default()
    }),
))
.add_systems(Update, update_visibility.run_if(server_running));

/// Disables the visibility of other players' entities that are further away than the visible distance.
fn update_visibility(
    mut clients: Query<&mut ClientVisibility>,
    moved_players: Query<(&Transform, &PlayerOwner), Changed<Transform>>,
    other_players: Query<(Entity, &Transform, &PlayerOwner)>,
) {
    for (moved_transform, &owner) in &moved_players {
        let mut visibility = clients.get_mut(*owner).unwrap();
        for (entity, transform, _) in other_players
            .iter()
            .filter(|(.., &other_owner)| *other_owner != *owner)
        {
            const VISIBLE_DISTANCE: f32 = 100.0;
            let distance = moved_transform.translation.distance(transform.translation);
            visibility.set_visibility(entity, distance < VISIBLE_DISTANCE);
        }
    }
}

/// Points to client entity.
#[derive(Component, Deref, Clone, Copy)]
struct PlayerOwner(Entity);
```

For a higher level API consider using [`bevy_replicon_attributes`](https://docs.rs/bevy_replicon_attributes).

# Eventual consistency

All events, inserts, removals and despawns will be applied to clients in the same order as on the server.

Entity component mutations are grouped by entity, and component groupings may be applied to clients in a different order than on the server.
For example, if two entities are spawned in tick 1 on the server and their components are mutated in tick 2,
then the client is guaranteed to see the spawns at the same time, but the component mutations may appear in different client ticks.

If a component is dependent on other data, mutations to the component will only be applied to the client when that data has arrived.
So if your component references another entity, mutations to that component will only be applied when the referenced entity has been spawned on the client.

Mutations for despawned entities will be discarded automatically, but events or components may reference despawned entities and should be handled with that in mind.

Clients should never assume their world state is the same as the server's on any given tick value-wise.
World state on the client is only "eventually consistent" with the server's.

# Troubleshooting

If you face any issue, try to enable logging to see what is going on.
To enable logging, you can temporarily set `RUST_LOG` environment variable to `bevy_replicon=debug`
(or `bevy_replicon=trace` for more noisy output) like this:

```bash
RUST_LOG=bevy_replicon=debug cargo run
```

The exact method depends on the OS shell.

Alternatively you can configure [`LogPlugin`](bevy::log::LogPlugin) to make it permanent.

For deserialization errors on client we use `error` level which should be visible by default.
But on server we use `debug` for it to avoid flooding server logs with errors caused by clients.
*/
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

#[cfg(feature = "client")]
pub mod client;
pub mod core;
#[cfg(feature = "parent_sync")]
pub mod parent_sync;
#[cfg(feature = "scene")]
pub mod scene;
#[cfg(feature = "server")]
pub mod server;
#[cfg(all(feature = "server", feature = "client"))]
pub mod test_app;

pub mod prelude {
    pub use super::{
        core::{
            channels::{ChannelKind, RepliconChannel, RepliconChannels},
            common_conditions::*,
            connected_client::{ConnectedClient, NetworkStats},
            event::{
                client_event::{ClientEventAppExt, FromClient},
                client_trigger::{ClientTriggerAppExt, ClientTriggerExt},
                server_event::{SendMode, ServerEventAppExt, ToClients},
                server_trigger::{ServerTriggerAppExt, ServerTriggerExt},
            },
            replication::{
                command_markers::AppMarkerExt, replication_rules::AppRuleExt, Replicated,
            },
            replicon_client::{RepliconClient, RepliconClientStatus},
            replicon_server::RepliconServer,
            RepliconCorePlugin, SERVER,
        },
        RepliconPlugins,
    };

    #[cfg(feature = "client")]
    pub use super::client::{
        event::ClientEventPlugin, ClientPlugin, ClientReplicationStats, ClientSet,
    };

    #[cfg(feature = "server")]
    pub use super::server::{
        client_entity_map::ClientEntityMap, client_visibility::ClientVisibility,
        event::ServerEventPlugin, ReplicatedClient, ServerPlugin, ServerSet, TickPolicy,
        VisibilityPolicy,
    };

    #[cfg(feature = "client_diagnostics")]
    pub use super::client::diagnostics::ClientDiagnosticsPlugin;
    #[cfg(feature = "parent_sync")]
    pub use super::parent_sync::{ParentSync, ParentSyncPlugin};
}

pub use bytes;
pub use postcard;

use bevy::{app::PluginGroupBuilder, prelude::*};
use prelude::*;

/// Plugin group for all replicon plugins.
///
/// Contains the following:
/// * [`RepliconCorePlugin`].
/// * [`ServerPlugin`] - with feature `server`.
/// * [`ServerEventPlugin`] - with feature `server`.
/// * [`ClientPlugin`] - with feature `client`.
/// * [`ClientEventPlugin`] - with feature `client`.
/// * [`ParentSyncPlugin`] - with feature `parent_sync`.
/// * [`ClientDiagnosticsPlugin`] - with feature `client_diagnostics`.
pub struct RepliconPlugins;

impl PluginGroup for RepliconPlugins {
    fn build(self) -> PluginGroupBuilder {
        let mut group = PluginGroupBuilder::start::<Self>();
        group = group.add(RepliconCorePlugin);

        #[cfg(feature = "server")]
        {
            group = group.add(ServerPlugin::default()).add(ServerEventPlugin);
        }

        #[cfg(feature = "client")]
        {
            group = group.add(ClientPlugin).add(ClientEventPlugin);
        }

        #[cfg(feature = "parent_sync")]
        {
            group = group.add(ParentSyncPlugin);
        }

        #[cfg(feature = "client_diagnostics")]
        {
            group = group.add(ClientDiagnosticsPlugin);
        }

        group
    }
}
