/*!
# Quick start

Write the same logic that works for both multiplayer and single-player.
The crate provides synchronization of components and network events between
server and clients using the [Renet](https://github.com/lucaspoffo/renet)
library for the [Bevy game engine](https://bevyengine.org).

## Initialization

You need to add [`ReplicationPlugins`] to your app:

```rust
use bevy::prelude::*;
use bevy_replicon::prelude::*;

let mut app = App::new();
app.add_plugins(MinimalPlugins)
    .add_plugins(ReplicationPlugins);
```

This group contains necessary replication stuff and setups server and client
plugins to let you host and join games from the same application. If you
planning to separate client and server you can use
[`PluginGroupBuilder::disable()`] to disable [`ClientPlugin`] or
[`ServerPlugin`]. You can also configure how often updates are sent from
server to clients with [`ServerPlugin`]'s [`TickPolicy`].:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
app.add_plugins(MinimalPlugins).add_plugins(
    ReplicationPlugins
        .build()
        .disable::<ClientPlugin>()
        .set(ServerPlugin::new(TickPolicy::MaxTickRate(60))),
);
```

The plugin handles Renet initialization, you don't need to add its plugins.

## Component replication

It's a process of sending component changes from server to clients in order to
keep the world in sync.

### Marking for replication

By default, no components are replicated. To start replication, you need two
things:

1. Register component type for replication. Component should implement
[`serde::Serialize`] and [`serde::Deserialize`].
You can use [`AppReplicationExt::replicate()`] to register the component for replication:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate::<DummyComponent>();

#[derive(Component, Deserialize, Serialize)]
struct DummyComponent;
```

If your component contains [`Entity`] then it cannot be deserialized as is
because entity IDs are different on server and client. The client should do the
mapping. Therefore, to replicate such components properly, they need implement
[`MapNetworkEntities`] and registered using [`AppReplicationExt::replicate_mapped()`]:

```rust
# use bevy::{prelude::*, utils::HashMap};
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(Entity);

impl MapNetworkEntities for MappedComponent {
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T) {
        self.0 = mapper.map(self.0);
    }
}
```

If your component doesn't implement serde traits or you want to serialize it partially
you can use [`AppReplicationExt::replicate_with`]:

```rust
use bevy::{ecs::world::EntityMut, prelude::*, ptr::Ptr, utils::HashMap};
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate_with::<Transform>(serialize_transform, deserialize_transform);

/// Serializes only translation.
fn serialize_transform(component: Ptr) -> Vec<u8> {
    // SAFETY: Function called for registered `ComponentId`.
    let transform: &Transform = unsafe { component.deref() };
    bincode::serialize(&transform.translation)
        .unwrap_or_else(|e| panic!("Vec3 should be serialzable: {e}"))
}

/// Deserializes translation and creates [`Transform`] from it.
fn deserialize_transform(
    entity: &mut EntityMut,
    _entity_map: &mut HashMap<Entity, Entity>,
    component: &[u8],
) {
    let translation: Vec3 = bincode::deserialize(component)
        .unwrap_or_else(|e| panic!("bytes from server should be Vec3: {e}"));
    entity.insert(Transform::from_translation(translation));
}
```

2. You need to choose entities you want to replicate using [`Replication`]
component. Just insert it to the entity you want to replicate. Only components
marked for replication through [`AppReplicationExt::replicate()`]
will be replicated.

If you need to disable replication for specific component for specific entity,
you can insert [`Ignored<T>`] component and replication will be skipped for `T`.

### "Blueprints" pattern

The idea was borrowed from [iyes_scene_tools](https://github.com/IyesGames/iyes_scene_tools#blueprints-pattern).
You don't want to replicate all components because not all of them are
necessary to send over the network. Components that computed based on other
components (like [`GlobalTransform`]) can be inserted after replication.
This is easily done using a system with an [`Added`] query filter.
This way, you detect when such entities are spawned into the world, and you can
do any additional setup on them using code. For example, if you have a
character with mesh, you can replicate only your `Player` component and insert
necessary components after replication. If you want to avoid one frame delay, put
your initialization systems to [`ClientSet::Receive`]:

```rust
use bevy::{ecs::world::EntityMut, prelude::*, ptr::Ptr, utils::HashMap};
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate_with::<Transform>(serialize_transform, deserialize_transform)
    .replicate::<Player>()
    .add_systems(PreUpdate, player_init_system.after(ClientSet::Receive));

fn player_init_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    spawned_players: Query<Entity, Added<Player>>,
) {
    for entity in &spawned_players {
        commands.entity(entity).insert((
            GlobalTransform::default(),
            Visibility::default(),
            ComputedVisibility::default(),
            meshes.add(Mesh::from(shape::Capsule::default())),
            materials.add(Color::AZURE.into()),
        ));
    }
}

#[derive(Component, Deserialize, Serialize)]
struct Player;
# fn serialize_transform(_: Ptr) -> Vec<u8> { unimplemented!() }
# fn deserialize_transform(_: &mut EntityMut, _: &mut HashMap<Entity, Entity>, _: &[u8]) {}
```

If your game have save states you probably want to re-use the same logic to
keep you saves clean. Also, although things like `Handle<T>` can technically be
serialized, they won't be valid after deserialization.

### Component relations

Sometimes components depend on each other. For example, [`Parent`] and
[`Children`]. In this case, you can't just replicate the [`Parent`] because you
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
These events will appear on server as [`FromClient`] wrapper event that
contains sender ID and the sent event. We consider the authority machine
(a single-player session or you are server) to be a client with ID
[`SERVER_ID`], so in this case the [`FromClient`] will be emitted too.
This way your game logic will work the same on client, server and in
single-player session.

Events include `[SendPolicy]` to configure delivery guarantees (reliability and
ordering). You can alternatively pass in `[bevy_renet::SendType]` directly if you
need custom configuration for a reliable policy's `resend_time`.

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_client_event::<DummyEvent>(SendPolicy::Ordered)
    .add_systems(Update, event_sending_system);

fn event_sending_system(mut dummy_events: EventWriter<DummyEvent>) {
    dummy_events.send_default()
}

fn event_receiving_system(mut dummy_events: EventReader<FromClient<DummyEvent>>) {
    for FromClient { client_id, event } in &mut dummy_events {
        info!("received event {event:?} from client {client_id}");
    }
}

#[derive(Debug, Default, Deserialize, Event, Serialize)]
struct DummyEvent;
```

Just like components, if an event contains [`Entity`], then the client should
map it before sending it to the server.
To do this, use [`ClientEventAppExt::add_mapped_client_event()`] and implement [`MapNetworkEntities`]:

```rust
use bevy::{prelude::*, utils::HashMap};
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_mapped_client_event::<MappedEvent>(SendPolicy::Ordered);

#[derive(Debug, Deserialize, Event, Serialize)]
struct MappedEvent(Entity);

impl MapNetworkEntities for MappedEvent {
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T) {
        self.0 = mapper.map(self.0);
    }
}
```

There is also [`ClientEventAppExt::add_client_reflect_event()`] and [`ClientEventAppExt::add_mapped_client_reflect_event()`]
for events that require reflection for serialization and deserialization (for example, events that contain `Box<dyn Reflect>`).
To serialize such event you need to write serializer and deserializer manually because for such types you need access to [`AppTypeRegistry`].
It's pretty straigtforward but requires some boilerplate. See [`BuildEventSerializer`], [`BuildEventDeserializer`] and module
`common` module in integration tests as example.
Don't forget to check what inside every `Box<dyn Reflect>` from a client, it could be anything!

### From server to client

A similar technique is used to send events from server to clients. To do this,
register the event with [`ServerEventAppExt::add_server_event()`] server event
and send it from server using [`ToClients`]. This wrapper contains send
parameters and the event itself. Just like events sent from the client,
they will be emitted locally on the server (if [`SERVER_ID`] is not excluded
from the send list):

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_server_event::<DummyEvent>(SendPolicy::Ordered)
    .add_systems(Update, event_sending_system);

fn event_sending_system(mut dummy_events: EventWriter<ToClients<DummyEvent>>) {
    dummy_events.send(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });
}

fn event_receiving_system(mut dummy_events: EventReader<DummyEvent>) {
    for event in &mut dummy_events {
        info!("received event {event:?} from server");
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Event, Serialize)]
struct DummyEvent;
```

Just like with client events, if the event contains [`Entity`], then
[`ServerEventAppExt::add_mapped_server_event()`] should be used instead.

And for events with `Box<dyn Reflect>` you can use [`ServerEventAppExt::add_server_reflect_event()`] and [`ServerEventAppExt::add_mapped_server_reflect_event()`].

## Server and client creation

To connect to the server or create it, you need to initialize the
[`renet::RenetClient`] and [`renet::transport::NetcodeClientTransport`] **or**
[`renet::RenetServer`] and [`renet::transport::NetcodeServerTransport`] resources from Renet.
All Renet API is re-exported from this plugin.

Never create client and server resources in the same app for single-player, it will cause replication loop.
Use the described pattern instead.

The only part of it that handled by this plugin is channels that used for
events and component replication. These channels should be obtained from the
[`NetworkChannels`] resource. So when creating server you need to initialize
[`renet::ConnectionConfig`] like this:

```rust
# use bevy::prelude::*;
# use bevy_replicon::{prelude::*, renet::ConnectionConfig};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
let network_channels = app.world.resource::<NetworkChannels>();
let connection_config = ConnectionConfig {
    server_channels_config: network_channels.server_channels(),
    client_channels_config: network_channels.client_channels(),
    ..Default::default()
};
```

For full example of how to initialize server or client see the example in the
repository.

## System sets and conditions

When configuring systems for multiplayer game, you often want to run some
systems only on when you have authority over the world simulation
(on server or in single-player session). For example, damage registration or
procedural level generation systems. For this just add [`has_authority()`]
condition on such system. If you want your systems to run only on
frames when server send updates to clients use [`ServerSet::Send`].

To check if you running server or client, you can use conditions based on
[`RenetClient`] and [`RenetServer`] resources.
They rarely used for gameplay systems (since you write the same logic for
multiplayer and single-player!), but could be used for server
creation / connection systems and corresponding UI.
*/

pub mod client;
pub mod network_event;
pub mod parent_sync;
pub mod replicon_core;
pub mod server;

pub mod prelude {
    pub use super::{
        client::{ClientPlugin, ClientSet, NetworkEntityMap},
        network_event::{
            client_event::{ClientEventAppExt, FromClient},
            server_event::{SendMode, ServerEventAppExt, ToClients},
            BuildEventDeserializer, BuildEventSerializer, SendPolicy,
        },
        parent_sync::{ParentSync, ParentSyncPlugin},
        renet::{RenetClient, RenetServer},
        replicon_core::{
            AppReplicationExt, Ignored, MapNetworkEntities, Mapper, NetworkChannels, Replication,
            ReplicationRules, RepliconCorePlugin,
        },
        server::{has_authority, AckedTicks, ServerPlugin, ServerSet, TickPolicy, SERVER_ID},
        ReplicationPlugins,
    };
}

use bevy::{app::PluginGroupBuilder, prelude::*};
pub use bevy_renet::*;
pub use bincode;
use prelude::*;

pub struct ReplicationPlugins;

impl PluginGroup for ReplicationPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(RepliconCorePlugin)
            .add(ParentSyncPlugin)
            .add(ClientPlugin)
            .add(ServerPlugin::default())
    }
}
