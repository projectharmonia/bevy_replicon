/*!
ECS-focused high-level networking crate for the [Bevy game engine](https://bevyengine.org).

# Quick start

Replicon provides a [`prelude`] module, which exports most of the typically used traits and types.

The library doesn't provide any I/O, so you need to add a messaging backend.
We provide a first-party integration with [`bevy_renet`](https://docs.rs/bevy_renet)
via [`bevy_replicon_renet`](https://docs.rs/bevy_replicon_renet).

If you want to write an integration for a messaging backend,
see the documentation for [`RepliconServer`], [`RepliconClient`] and [`ServerEvent`].
You can also use `bevy_replicon_renet` as a reference.

Also depending on your game, you may want to use additional crates. For example, if your game
is fast-paced, you will need interpolation and rollback.
For details see [`goals`](https://github.com/projectharmonia/bevy_replicon#goals) and
[`related crates`](https://github.com/projectharmonia/bevy_replicon#related-crates).
Before adding advanced functionality, it's recommended to read the quick start guide
first to understand the basics.

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

If you are planning to separate client and server you can use
[`PluginGroupBuilder::disable()`] to disable [`ClientPlugin`] or [`ServerPlugin`] on [`RepliconPlugins`].
You will need to disable similar plugins on your messaing library of choice too.

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

Never initialize a client and server in the same app for single-player, it will cause a replication loop.
Use the described pattern in [system sets and conditions](#system-sets-and-conditions)
in combination with [network events](#network-events).

</div>

## System conditions

To run a system based on a network condition, use the [`core::common_conditions`] module.
This module is also available from [`prelude`].

For example, to display a "connecting" message, you can use [`client_connecting`].
But for gameplay systems, you most likely want to run them in both server and single-player
sessions. For example, damage registration or procedural generation systems. Use [`has_authority`]
condition for those cases.

If you want your systems to run only on frames when the server sends updates to clients,
use [`ServerSet::Send`].

## Replication

It's a process of sending changes from server to clients in order to
keep the world in sync.

To prevent cheating, we do not support replicating from the client. If you need to send
information from clients to the server, use [events](#network-events).

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

By default all components are serialized with [`bincode`] using [`DefaultOptions`](bincode::DefaultOptions).
If your component doesn't implement serde traits or you want to serialize it partially
(for example, only replicate the `translation` field from [`Transform`]),
you can use [`AppRuleExt::replicate_with`].

If you want a group of components to be replicated only if all of them are present on an entity,
you can use [`AppRuleExt::replicate_group`].

If you want to customize how the received component will be written or removed on clients based
on some marker component (for example, write into a different component), see [`AppMarkerExt`].
Useful for implementing rollback and interpolation.

In order to serialize Bevy components you need to enable the `serialize` feature on Bevy.

### Mapping to existing client entities

If you want the server to replicate an entity into a client entity that was already spawned on a client, see [`ClientEntityMap`].

This can be useful for certain types of game. For example, spawning bullets on the client immediately without
waiting on replication.

### "Blueprints" pattern

The idea was borrowed from [iyes_scene_tools](https://github.com/IyesGames/iyes_scene_tools#blueprints-pattern).
You don't want to replicate all components because not all of them are
necessary to send over the network. For example, components that are computed based on other
components (like [`GlobalTransform`]) can be inserted after replication.
This can be easily done using a system with query filter.
This way, you detect when such entities are spawned into the world, and you can
do any additional setup on them using code. For example, if you have a
character with mesh, you can replicate only your `Player` and [`Transform`] components and insert
necessary components after replication. To avoid one frame delay, put
your initialization systems in [`ClientSet::Receive`]:

```
# use bevy::{color::palettes::css::AZURE, prelude::*, sprite::Mesh2dHandle};
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.replicate::<Transform>()
    .replicate::<Player>()
    .add_systems(PreUpdate, init_player.after(ClientSet::Receive));

fn init_player(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    // Infer that the player was just added by the fact it's missing `GlobalTransform`.
    players: Query<Entity, (With<Player>, Without<GlobalTransform>)>,
) {
    for entity in &players {
        commands.entity(entity).insert((
            GlobalTransform::default(),
            VisibilityBundle::default(),
            Mesh2dHandle(meshes.add(Capsule2d::default())),
            materials.add(Color::from(AZURE)),
        ));
    }
}

/// Bundle to spawn a player.
///
/// All non-replicated components will be added inside [`init_player`]
/// after spawn, replication or even deserialization from disk.
#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    transform: Transform,
    replicated: Replicated,
}

#[derive(Component, Deserialize, Serialize)]
struct Player;
```

This pairs nicely with server state serialization and keeps saves clean.
You can use [`replicate_into`](scene::replicate_into) to
fill [`DynamicScene`] with replicated entities and their components.

**Performance note**: We used [`With<Player>`] and [`Without<GlobalTransform>`] to
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

## Network events

Network event replace RPCs (remote procedure calls) in other engines and,
unlike components, can be sent both from server to clients and from clients to
server.

### From client to server

To send specific events from client to server, you need to register the event
with [`ClientEventAppExt::add_client_event()`] instead of [`App::add_event()`].
The event must be registered on both the client and the server in the same order.

Events include [`ChannelKind`] to configure delivery guarantees (reliability and
ordering). You can alternatively pass in [`RepliconChannel`] with more advanced configuration.

These events will appear on server as [`FromClient`] wrapper event that
contains sender ID and the sent event. We consider server or a single-player session
also as a client with ID [`ClientId::SERVER`]. So you can send such events even on server
and [`FromClient`] will be emitted for them too. This way your game logic will work the same
on client, listen server and in single-player session.

For systems that receive events attach [`has_authority`] condition to receive a message
on non-client instances (server or single-player):

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_client_event::<DummyEvent>(ChannelKind::Ordered)
    .add_systems(Update, (send_events, receive_events.run_if(has_authority)));

/// Sends an event from client or listen server.
fn send_events(mut dummy_events: EventWriter<DummyEvent>) {
    dummy_events.send_default();
}

/// Receives events on server or single-player.
fn receive_events(mut dummy_events: EventReader<FromClient<DummyEvent>>) {
    for FromClient { client_id, event } in dummy_events.read() {
        info!("received event {event:?} from {client_id:?}");
    }
}

#[derive(Debug, Default, Deserialize, Event, Serialize)]
struct DummyEvent;
```

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
deserialization functions. This could be used for sending events that contain [`Box<dyn Reflect>`], which
require access to the [`AppTypeRegistry`] resource.

Don't forget to validate the contents of every [`Box<dyn Reflect>`] from a client, it could be anything!

### From server to client

A similar technique is used to send events from server to clients. To do this,
register the event with [`ServerEventAppExt::add_server_event()`] server event
and send it from server using [`ToClients`]. The event must be registered on
both the client and the server in the same order. This wrapper contains send parameters
and the event itself. Just like events sent from the client, you can send these events on the server or
in single-player and they will appear locally as regular events (if [`ClientId::SERVER`] is not excluded
from the send list):

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_server_event::<DummyEvent>(ChannelKind::Ordered)
    .add_systems(Update, (send_events.run_if(has_authority), receive_events));

/// Sends an event from server or single-player.
fn send_events(mut dummy_events: EventWriter<ToClients<DummyEvent>>) {
    dummy_events.send(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });
}

/// Receives events on client or listen server.
fn receive_events(mut dummy_events: EventReader<DummyEvent>) {
    for event in dummy_events.read() {
        info!("received event {event:?} from server");
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Event, Serialize)]
struct DummyEvent;
```

Just like with client events, if the event contains an entity, then
[`ServerEventAppExt::add_mapped_server_event()`] should be used instead.

For events that require special serialization and deserialization functions you can use
[`ServerEventAppExt::add_server_event_with()`].

## Client visibility

You can control which parts of the world are visible for each client by setting visibility policy
in [`ServerPlugin`] to [`VisibilityPolicy::Whitelist`] or [`VisibilityPolicy::Blacklist`].

In order to set which entity is visible, you need to use the [`ConnectedClients`] resource
to obtain the [`ConnectedClient`] for a specific client and get its [`ClientVisibility`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
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
    mut connected_clients: ResMut<ConnectedClients>,
    moved_players: Query<(&Transform, &Player), Changed<Transform>>,
    other_players: Query<(Entity, &Transform, &Player)>,
) {
    for (moved_transform, moved_player) in &moved_players {
        let client = connected_clients.client_mut(moved_player.0);
        for (entity, transform, _) in other_players
            .iter()
            .filter(|(.., player)| player.0 != moved_player.0)
        {
            const VISIBLE_DISTANCE: f32 = 100.0;
            let distance = moved_transform.translation.distance(transform.translation);
            client
                .visibility_mut()
                .set_visibility(entity, distance < VISIBLE_DISTANCE);
        }
    }
}

#[derive(Component, Deserialize, Serialize)]
struct Player(ClientId);
```

For a higher level API consider using [`bevy_replicon_attributes`](https://docs.rs/bevy_replicon_attributes).

## Eventual consistency

All events, inserts, removals and despawns will be applied to clients in the same order as on the server.

Entity component updates are grouped by entity, and component groupings may be applied to clients in a different order than on the server.
For example, if two entities are spawned in tick 1 on the server and their components are updated in tick 2,
then the client is guaranteed to see the spawns at the same time, but the component updates may appear in different client ticks.

If a component is dependent on other data, updates to the component will only be applied to the client when that data has arrived.
So if your component references another entity, updates to that component will only be applied when the referenced entity has been spawned on the client.

Updates for despawned entities will be discarded automatically, but events or components may reference despawned entities and should be handled with that in mind.

Clients should never assume their world state is the same as the server's on any given tick value-wise.
World state on the client is only "eventually consistent" with the server's.

## Limits

To reduce packet size there are the following limits per replication update:

- Up to [`u16::MAX`] entities that have added components with up to [`u16::MAX`] bytes of component data.
- Up to [`u16::MAX`] entities that have changed components with up to [`u16::MAX`] bytes of component data.
- Up to [`u16::MAX`] entities that have removed components with up to [`u16::MAX`] bytes of component data.
- Up to [`u16::MAX`] entities that were despawned.
*/

pub mod client;
pub mod core;
pub mod parent_sync;
pub mod scene;
pub mod server;
pub mod test_app;

pub mod prelude {
    #[allow(deprecated)]
    pub use super::core::Replication;

    pub use super::{
        client::{
            diagnostics::{ClientDiagnosticsPlugin, ClientStats},
            events::{ClientEventAppExt, ClientEventsPlugin, FromClient},
            replicon_client::{RepliconClient, RepliconClientStatus},
            ClientPlugin, ClientSet,
        },
        core::{
            channels::{ChannelKind, RepliconChannel, RepliconChannels},
            command_markers::AppMarkerExt,
            common_conditions::*,
            replication_rules::AppRuleExt,
            ClientId, Replicated, RepliconCorePlugin,
        },
        parent_sync::{ParentSync, ParentSyncPlugin},
        server::{
            client_entity_map::{ClientEntityMap, ClientMapping},
            connected_clients::{
                client_visibility::ClientVisibility, ConnectedClient, ConnectedClients,
            },
            events::{SendMode, ServerEventAppExt, ServerEventsPlugin, ToClients},
            replicon_server::RepliconServer,
            ServerEvent, ServerPlugin, ServerSet, TickPolicy, VisibilityPolicy,
        },
        RepliconPlugins,
    };
}

pub use bincode;

use bevy::{app::PluginGroupBuilder, prelude::*};
use prelude::*;

/// Plugin Group for all replicon plugins.
pub struct RepliconPlugins;

impl PluginGroup for RepliconPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(RepliconCorePlugin)
            .add(ParentSyncPlugin)
            .add(ClientPlugin)
            .add(ServerPlugin::default())
            .add(ClientEventsPlugin)
            .add(ServerEventsPlugin)
    }
}
