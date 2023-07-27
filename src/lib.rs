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
        .set(ServerPlugin { tick_policy: TickPolicy::MaxTickRate(60) }),
);
```

The plugin handles Renet initialization, you don't need to add its plugins.

## Component replication

It's a process of sending component changes from server to clients in order to
keep the world in sync.

### Marking for replication

By default, no components are replicated. To start replication, you need two
things:

1. Mark component type for replication. Component should implement [`Reflect`],
have `#[reflect(Component)]` and all its fields should be registered. You can
use [`AppReplicationExt::replicate()`] to mark the component for replication:

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

This also automatically registers the specified type, so you don't need to call
[`App::register_type()`] if you replicating the type.

If your component contains [`Entity`] then it cannot be deserialized as is
because entity IDs are different on server and client. The client should do the
mapping. Therefore, to replicate such components properly, they need implement
[`bevy::ecs::entity::MapEntities`] and have `#[reflect(MapEntities)]`:

```rust
# use bevy::{
#     ecs::{
#         entity::{EntityMapper, MapEntities},
#         reflect::ReflectMapEntities,
#     },
#     prelude::*,
# };
# use bevy_replicon::prelude::*;
#[derive(Component, Reflect)]
#[reflect(Component, MapEntities)]
struct MappedComponent(Entity);

impl MapEntities for MappedComponent {
    fn map_entities(&mut self, entity_mapper: &mut EntityMapper) {
        self.0 = entity_mapper.get_or_reserve(self.0);
    }
}

// We need to impl either `FromWorld` or `Default` so `MappedComponent` can
// be registered as `Reflect`. This is because `Reflect` deserialize by
// creating an instance and apply a patch on top. However `MappedComponent`
// should only ever be created with a real user-defined entity, so it's better
// to implement `FromWorld`.
// Bevy uses the same pattern to reflect components with `Entity`.
impl FromWorld for MappedComponent {
    fn from_world(_world: &mut World) -> Self {
        Self(Entity::PLACEHOLDER)
    }
}
```

2. You need to choose entities you want to replicate using [`Replication`]
component. Just insert it to the entity you want to replicate. Only components
marked for replication through [`AppReplicationExt::replicate()`]
will be replicated.

If you need more control, you add special rules. For example, if you don't want
to replicate [`Transform`] on entities marked for replication if your special
component is present, you can do the following:

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

The idea was borrowed from [iyes_scene_tools](https://github.com/IyesGames/iyes_scene_tools#blueprints-pattern).
You don't want to replicate all components because not all of them are
necessary to send over the network. Components that computed based on other
components (like [`GlobalTransform`]) can be inserted after replication.
This is easily done using a system with an [`Added`] query filter.
This way, you detect when such entities are spawned into the world, and you can
do any additional setup on them using code. For example, if you have a
character with mesh, you can replicate only your `Player` component and insert
necessary components after replication:

```rust
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.replicate::<Transform>()
    .replicate::<Visibility>()
    .replicate::<Player>()
    .add_systems(Update, player_init_system);

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

To send specific events from server to client, you need to register the event
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
To do this, use [`ClientEventAppExt::add_mapped_client_event()`] and implement [`MapEventEntities`]:

```rust
# use bevy::{
#     ecs::{
#         entity::{EntityMap, MapEntities},
#         reflect::ReflectMapEntities,
#     },
#     prelude::*,
# };
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(ReplicationPlugins);
app.add_mapped_client_event::<MappedEvent>(SendPolicy::Ordered);

#[derive(Debug, Deserialize, Event, Serialize)]
struct MappedEvent(Entity);

impl MapEventEntities for MappedEvent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapError> {
        self.0 = entity_map.get(self.0).ok_or(MapError(self.0))?;
        Ok(())
    }
}
```

There is also [`ClientEventAppExt::add_client_reflect_event()`] and [`ClientEventAppExt::add_mapped_client_reflect_event()`]
for events that require reflection for serialization and deserialization (for example, events that contain `Box<dyn Reflect>`).
To serialize such event you need to write serializer and deserializer manually because for such types you need acess to [`AppTypeRegistry`].
It's pretty straigtforward but requires some boilerplate. See [`BuildEventSerializer`], [`BuildEventDeserializer`] and module
`network_event::test_events` (used for unit tests) as example.
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
frames when server send updates to clients use [`ServerSet::Tick`].

To check if you running server or client, you can use conditions based on
[`RenetClient`] and [`RenetServer`] resources.
They rarely used for gameplay systems (since you write the same logic for
multiplayer and single-player!), but could be used for server
creation / connection systems and corresponding UI.
*/

pub mod client;
pub mod network_event;
pub mod parent_sync;
pub mod replication_core;
pub mod server;
#[cfg(test)]
mod test_network;
mod world_diff;

pub mod prelude {
    pub use super::{
        client::ClientPlugin,
        has_authority,
        network_event::{
            client_event::{ClientEventAppExt, FromClient},
            server_event::{SendMode, ServerEventAppExt, ToClients},
            BuildEventDeserializer, BuildEventSerializer, MapError, MapEventEntities, SendPolicy,
        },
        parent_sync::{ParentSync, ParentSyncPlugin},
        renet::{RenetClient, RenetServer},
        replication_core::{
            AppReplicationExt, NetworkChannels, Replication, ReplicationCorePlugin,
            ReplicationRules,
        },
        server::{ServerPlugin, ServerSet, TickPolicy, SERVER_ID},
        ReplicationPlugins,
    };
}

use bevy::{app::PluginGroupBuilder, prelude::*};
pub use bevy_renet::*;
use prelude::*;

const REPLICATION_CHANNEL_ID: u8 = 0;

pub struct ReplicationPlugins;

impl PluginGroup for ReplicationPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(ReplicationCorePlugin)
            .add(ParentSyncPlugin)
            .add(ClientPlugin)
            .add(ServerPlugin::default())
    }
}

/// Condition that returns `true` if server is present or in singleplayer.
pub fn has_authority() -> impl FnMut(Option<Res<RenetClient>>) -> bool + Clone {
    move |client| client.is_none()
}

#[cfg(test)]
mod tests {
    use bevy::{
        ecs::{
            component::Tick,
            entity::{EntityMapper, MapEntities},
            reflect::ReflectMapEntities,
        },
        utils::HashMap,
    };
    use bevy_renet::renet::transport::NetcodeClientTransport;

    use super::*;
    use crate::{
        client::NetworkEntityMap,
        replication_core::{AppReplicationExt, Replication},
        server::{despawn_tracker::DespawnTracker, removal_tracker::RemovalTracker, AckedTicks},
    };

    #[test]
    fn acked_ticks_cleanup() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins));

        test_network::setup(&mut app);

        let mut client_transport = app.world.resource_mut::<NetcodeClientTransport>();
        client_transport.disconnect();
        let client_id = client_transport.client_id();

        let mut acked_ticks = app.world.resource_mut::<AckedTicks>();
        acked_ticks.insert(client_id, Tick::new(0));

        app.update();
        app.update();

        let acked_ticks = app.world.resource::<AckedTicks>();
        assert!(!acked_ticks.contains_key(&client_id));
    }

    #[test]
    fn tick_acks_receiving() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins));

        test_network::setup(&mut app);

        for _ in 0..10 {
            app.update();
        }

        let acked_ticks = app.world.resource::<AckedTicks>();
        let client_transport = app.world.resource::<NetcodeClientTransport>();
        assert!(
            matches!(acked_ticks.get(&client_transport.client_id()), Some(&last_tick) if last_tick.get() > 0)
        );
    }

    #[test]
    fn spawn_replication() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins))
            .replicate::<TableComponent>();

        test_network::setup(&mut app);

        app.update();

        let server_entity = app.world.spawn((TableComponent, Replication)).id();

        app.update();

        // Remove server entity before client replicates it,
        // since in test client and server in the same world.
        app.world.entity_mut(server_entity).despawn();

        app.update();

        let client_entity = app
            .world
            .query_filtered::<Entity, (With<TableComponent>, With<Replication>)>()
            .get_single(&app.world)
            .expect("server entity should be replicated to client");
        let entity_map = app.world.resource::<NetworkEntityMap>();
        let mapped_entity = entity_map
            .to_client()
            .get(server_entity)
            .expect("server entity should be mapped on client");
        assert_eq!(
            mapped_entity, client_entity,
            "mapped entity should correspond to the replicated entity on client"
        );
    }

    #[test]
    fn insert_replication() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins))
            .replicate::<TableComponent>()
            .replicate::<SparseSetComponent>()
            .replicate::<IgnoredComponent>()
            .not_replicate_if_present::<IgnoredComponent, ExclusionComponent>();

        test_network::setup(&mut app);

        app.update();

        let replicated_entity = app
            .world
            .spawn((
                Replication,
                TableComponent,
                SparseSetComponent,
                NonReflectedComponent,
            ))
            .id();

        // Mark as already spawned.
        app.world
            .resource_mut::<NetworkEntityMap>()
            .insert(replicated_entity, replicated_entity);

        app.update();

        // Remove components before client replicates it,
        // since in test client and server in the same world.
        let mut replicated_entity = app.world.entity_mut(replicated_entity);
        replicated_entity.remove::<SparseSetComponent>();
        replicated_entity.remove::<TableComponent>();
        replicated_entity.remove::<NonReflectedComponent>();
        let replicated_entity = replicated_entity.id();

        app.update();

        let replicated_entity = app.world.entity(replicated_entity);
        assert!(replicated_entity.contains::<SparseSetComponent>());
        assert!(replicated_entity.contains::<TableComponent>());
        assert!(!replicated_entity.contains::<NonReflectedComponent>());
    }

    #[test]
    fn entity_mapping() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins))
            .replicate::<MappedComponent>();

        test_network::setup(&mut app);

        app.update();

        let client_parent = app.world.spawn_empty().id();
        let server_parent = app.world.spawn_empty().id();
        let replicated_entity = app
            .world
            .spawn((Replication, MappedComponent(server_parent)))
            .id();

        let mut entity_map = app.world.resource_mut::<NetworkEntityMap>();
        entity_map.insert(replicated_entity, replicated_entity);
        entity_map.insert(server_parent, client_parent);

        app.update();
        app.update();

        let parent_sync = app.world.get::<MappedComponent>(replicated_entity).unwrap();
        assert_eq!(parent_sync.0, client_parent);
    }

    #[test]
    fn removal_replication() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins))
            .register_type::<NonReflectedComponent>();

        test_network::setup(&mut app);

        app.update();

        // Mark components as removed.
        const REMOVAL_TICK: Tick = Tick::new(1); // Should be more then 0 since both client and server starts with 0 tick and think that everything is replicated at this point.
        let replication_id = app.world.init_component::<Replication>();
        let removal_tracker = RemovalTracker(HashMap::from([(replication_id, REMOVAL_TICK)]));
        let replicated_entity = app
            .world
            .spawn((removal_tracker, Replication, NonReflectedComponent))
            .id();

        app.world
            .resource_mut::<NetworkEntityMap>()
            .insert(replicated_entity, replicated_entity);

        app.update();
        app.update();

        let replicated_entity = app.world.entity(replicated_entity);
        assert!(!replicated_entity.contains::<Replication>());
        assert!(replicated_entity.contains::<NonReflectedComponent>());
    }

    #[test]
    fn despawn_replication() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ReplicationPlugins));

        test_network::setup(&mut app);

        app.update();

        let children_entity = app.world.spawn_empty().id();
        let despawned_entity = app
            .world
            .spawn_empty()
            .push_children(&[children_entity])
            .id();
        let current_tick = app.world.read_change_tick();
        let mut despawn_tracker = app.world.resource_mut::<DespawnTracker>();
        despawn_tracker
            .despawns
            .push((despawned_entity, current_tick));

        app.world
            .resource_mut::<NetworkEntityMap>()
            .insert(despawned_entity, despawned_entity);

        app.update();
        app.update();

        assert!(app.world.get_entity(despawned_entity).is_none());
        assert!(app.world.get_entity(children_entity).is_none());
        assert!(app
            .world
            .resource::<NetworkEntityMap>()
            .to_client()
            .is_empty());
    }

    #[derive(Component, Reflect)]
    #[reflect(Component, MapEntities)]
    struct MappedComponent(Entity);

    impl MapEntities for MappedComponent {
        fn map_entities(&mut self, entity_map: &mut EntityMapper) {
            self.0 = entity_map.get_or_reserve(self.0);
        }
    }

    impl FromWorld for MappedComponent {
        fn from_world(_world: &mut World) -> Self {
            Self(Entity::PLACEHOLDER)
        }
    }

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct TableComponent;

    #[derive(Component, Default, Reflect)]
    #[component(storage = "SparseSet")]
    #[reflect(Component)]
    struct SparseSetComponent;

    #[derive(Component, Reflect)]
    struct NonReflectedComponent;

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct IgnoredComponent;

    #[derive(Component, Reflect)]
    struct ExclusionComponent;
}
