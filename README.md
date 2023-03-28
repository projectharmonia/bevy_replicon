# Bevy Replicon

[![codecov](https://codecov.io/gh/lifescapegame/bevy_replicon/branch/master/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/lifescapegame/bevy_replicon)

Write the same logic that works for both multiplayer and single-player. The crate provides synchronization of components and network events between the server and clients using the [Renet](https://github.com/lucaspoffo/renet) library for the [Bevy game engine](https://bevyengine.org).

## Initialization

You need to add [`ReplicationPlugins`] to your app:

```rust
use bevy::prelude::*;
use bevy_replicon::prelude::*;

let mut app = App::new();
app.add_plugins(MinimalPlugins)
    .add_plugins(ReplicationPlugins);
```

This group contains necessary replication stuff and setups server and client plugins to let you host and join games from the same application. If you planning to separate client and server you can use [`PluginGroupBuilder::disable()`] to disable [`ClientPlugin`] or [`ServerPlugin`]. You can also configure how often updates are sent from server to clients:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
app.add_plugins(MinimalPlugins).add_plugins(
    ReplicationPlugins
        .build()
        .disable::<ClientPlugin>()
        .set(ServerPlugin { tick_rate: 60 }),
);
```

The plugin handles Renet initialization, you don't need to add its plugins.

## Component replication

It's a process of sending component changes from server to clients in order to keep the world in sync.

### Marking for replication

By default, no components are replicated. To start replication, you need two things:

1. Mark component type for replication. Component should implement [`Reflect`], have `#[reflect(Component)]`. You can use [`AppReplicationExt::replicate()`] to mark the component for replication:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate::<DummyComponent>();

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct DummyComponent;
```

This also automatically registers the specified type, so you don't need to call [`App::register_type()`] if you replicating the type.

If your component contains [`Entity`] then it cannot be deserialized as is because entity IDs are different on server and client. The client should do the mapping. Therefore, to replicate such components properly, they need implement [`bevy::ecs::entity::MapEntities`] and have `#[reflect(MapEntity)]`:

```rust
# use bevy::{prelude::*, ecs::entity::{EntityMap, MapEntities, MapEntitiesError}};
# use bevy_replicon::prelude::*;
#[derive(Component, Reflect)]
#[reflect(Component, MapEntity)]
struct MappedComponent(Entity);

impl MapEntities for MappedComponent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapEntitiesError> {
        self.0 = entity_map.get(self.0)?;
        Ok(())
    }
}

// We need to impl either `FromWorld` or `Default` so `MappedComponent` can be registered as `Reflect`.
// This is because `Reflect` deserialize by creating an instance and apply a patch on top.
// However `MappedComponent` should only ever be created with a real user-defined entity,
// so it's better to implement `FromWorld`. Bevy uses the same pattern to reflect components with `Entity`.
impl FromWorld for MappedComponent {
    fn from_world(_world: &mut World) -> Self {
        Self(Entity::from_raw(u32::MAX))
    }
}
```

2. You need to choose entities you want to replicate using [`Replication`] component. Just insert it to the entity you want to replicate. Only components marked for replication through [`AppReplicationExt::replicate()`] will be replicated.

If you need more control, you add special rules. For example, if you don't want to replicate [`Transform`] on entities marked for replication if your special component is present, you can do the following:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate::<Visibility>()
    .replicate::<DummyComponent>()
    .not_replicate_if_present::<Visibility, DummyComponent>();

# #[derive(Component, Default, Reflect)]
# #[reflect(Component)]
# struct DummyComponent;
```

Could be called any number times.

### "Blueprints" pattern

The idea was borrowed from [iyes_scene_tools](https://github.com/IyesGames/iyes_scene_tools#blueprints-pattern). You don't want to replicate all components because not all of them are necessary to send over the network. Components that computed based on other components (like [`GlobalTransform`]) can be inserted after replication. This is easily done using a system with an [`Added`] query filter. This way, you detect when such entities are spawned into the world, and you can do any additional setup on them using code. For example, if you have a character with mesh, you can replicate only your `Player` component and insert necessary components after replication:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate::<Transform>()
    .replicate::<Visibility>()
    .replicate::<Player>()
    .add_system(player_init_system);

fn player_init_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    spawned_players: Query<Entity, Added<Player>>,
) {
    for entity in &spawned_players {
        commands.entity(entity).insert((
            GlobalTransform::default(),
            ComputedVisibility::default(),
            meshes.add(Mesh::from(shape::Capsule::default())),
            materials.add(Color::AZURE.into()),
        ));
    }
}

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Player;
```

If your game have save states you probably want to re-use the same logic to keep you saves clean. Also, although things like `Handle<T>` can technically be serialized, they won't be valid after deserialization.

### Component relations

Sometimes components depend on each other. For example, [`Parent`] and [`Children`]. In this case, you can't just replicate the [`Parent`] because you not only need to add it to the [`Children`] of the parent, but also remove it from the [`Children`] of the old one. In this case, you need to create a third component that correctly updates the other two when it changes, and only replicate that one. This crate provides [`ParentSync`] component that does just that for Bevy hierarchy. For your custom components with relations you need to write your own with a similar pattern.

## Network events

Network event replace RPCs (remote procedure calls) in other engines and, unlike components, can be sent both from server to clients and from clients to server.

### From client to server

To send specific events from server to client, you need to register the event with [`ClientEventAppExt::add_client_event()`] instead of [`App::add_event()`]. These events will appear on server as [`FromClient`] wrapper event that contains sender ID and the sent event. We consider authority machine (a single-player session or you are server) and as a client with ID [`SERVER_ID`], so in this case the [`FromClient`]  will will be emitted too. This way your game logic will work the same on client, server and in single-player session.

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_client_event::<DummyEvent>()
    .add_system(event_sending_system);

fn event_sending_system(mut dummy_events: EventWriter<DummyEvent>) {
    dummy_events.send_default()
}

fn event_receiving_system(mut dummy_events: EventReader<FromClient<DummyEvent>>) {
    for FromClient { client_id, event } in dummy_events.iter() {
        info!("received event {event:?} from client {client_id}");
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct DummyEvent;
```

Just like components, if an event contains [`Entity`], then the client should map it before sending it to the server. To do this, use [`ClientEventAppExt::add_mapped_client_event()`]:

```rust
# use bevy_replicon::prelude::*;
# use bevy::{prelude::*, ecs::entity::{EntityMap, MapEntities, MapEntitiesError}};
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_mapped_client_event::<MappedEvent>();

#[derive(Deserialize, Serialize, Debug)]
struct MappedEvent(Entity);

impl MapEntities for MappedEvent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapEntitiesError> {
        self.0 = entity_map.get(self.0)?;
        Ok(())
    }
}
```

### From server to client

A similar technique is used to send events from server to clients. To do this, register the event with [`ServerEventAppExt::add_server_event()`] server event and send it from server using [`ToClients`]. This wrapper contains send parameters and the event itself. Just like events sent from the client, they will be emitted locally on the server (if [`SERVER_ID`] is not excluded from the send list):

```rust
# use bevy::prelude::*;
# use bevy_replicon::{prelude::*, renet::RenetConnectionConfig};
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_server_event::<DummyEvent>()
    .add_system(event_sending_system);

fn event_sending_system(mut dummy_events: EventWriter<ToClients<DummyEvent>>) {
    dummy_events.send(ToClients {
        mode: SendMode::Broadcast,
        event: DummyEvent,
    });
}

fn event_receiving_system(mut dummy_events: EventReader<DummyEvent>) {
    for event in dummy_events.iter() {
        info!("received event {event:?} from server");
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
struct DummyEvent;
```

Just like with client events, if the event contains [`Entity`], then [`ServerEventAppExt::add_mapped_server_event()`] should be used instead.

## Server and client creation

To connect to the server or create it, you need to initialize the [`renet::RenetClient`] or [`renet::RenetServer`] resource from Renet. All Renet API is re-exported from this plugin.

The only part of it that handled by this plugin is channels that used for events and component replication. These channels should be obtained from the [`NetworkChannels`] resource. So when creating server you need to initialize [`renet::RenetConnectionConfig`] like this:

```rust
# use bevy::prelude::*;
# use bevy_replicon::{prelude::*, renet::RenetConnectionConfig};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
let network_channels = app.world.resource::<NetworkChannels>();
let connection_config = RenetConnectionConfig {
    send_channels_config: network_channels.server_channels(),
    receive_channels_config: network_channels.client_channels(),
    ..Default::default()
};
```

For client you need to swap [`NetworkChannels::server_channels()`] and [`NetworkChannels::client_channels()`].

For full example of how to initialize server or client see the example in the repository.

## System sets and states

When configuring systems for multiplayer game, you often want to run some systems only on when you have authority over the world simulation (on server or in single-player session). For example, damage registration or procedural level generation systems. For this just add your systems to the [`ServerSet::Authority`] system set. If you want your systems to run only on frames when server send updates to clients use [`ServerSet::Tick`].

We also have states for server and client: [`ServerState`] or [`ClientState`]. They rarely used for gameplay systems (since you write the same logic for multiplayer and single-player!), but could be used for server creation / connection systems and corresponding UI.

## Bevy compatibility

| bevy | bevy_replicon |
|------|---------------|
| 0.10 | 0.1           |
