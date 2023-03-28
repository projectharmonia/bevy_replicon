#![warn(clippy::doc_markdown)]
#![doc = include_str!("../README.md")]

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
        client::{map_entity::ReflectMapEntity, ClientPlugin, ClientState},
        network_event::{
            client_event::{ClientEventAppExt, FromClient},
            server_event::{SendMode, ServerEventAppExt, ToClients},
        },
        parent_sync::{ParentSync, ParentSyncPlugin},
        renet::{RenetClient, RenetServer},
        replication_core::{AppReplicationExt, NetworkChannels, Replication, ReplicationRules},
        server::{ServerPlugin, ServerSet, ServerState, SERVER_ID},
        ReplicationPlugins,
    };
}

use bevy::{app::PluginGroupBuilder, prelude::*};
pub use bevy_renet::renet;
use prelude::*;
use replication_core::ReplicationCorePlugin;

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

#[cfg(test)]
mod tests {
    use bevy::{
        ecs::entity::{EntityMap, MapEntities, MapEntitiesError},
        utils::HashMap,
    };
    use bevy_renet::renet::RenetClient;

    use super::*;
    use crate::{
        client::map_entity::{NetworkEntityMap, ReflectMapEntity},
        replication_core::{AppReplicationExt, Replication},
        server::{despawn_tracker::DespawnTracker, removal_tracker::RemovalTracker, AckedTicks},
        test_network::TestNetworkPlugin,
    };

    #[test]
    fn acked_ticks_cleanup() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .add_plugin(TestNetworkPlugin);

        let mut client = app.world.resource_mut::<RenetClient>();
        client.disconnect();
        let client_id = client.client_id();

        let mut acked_ticks = app.world.resource_mut::<AckedTicks>();
        acked_ticks.insert(client_id, 0);

        app.update();

        let acked_ticks = app.world.resource::<AckedTicks>();
        assert!(!acked_ticks.contains_key(&client_id));
    }

    #[test]
    fn tick_acks_receiving() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .add_plugin(TestNetworkPlugin);

        for _ in 0..10 {
            app.update();
        }

        let acked_ticks = app.world.resource::<AckedTicks>();
        let client = app.world.resource::<RenetClient>();
        assert!(matches!(acked_ticks.get(&client.client_id()), Some(&last_tick) if last_tick > 0));
    }

    #[test]
    fn spawn_replication() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .replicate::<TableComponent>()
            .add_plugin(TestNetworkPlugin);

        // Wait two ticks to send and receive acknowledge.
        app.update();
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
    fn insert_replicaiton() {
        let mut app = App::new();
        app.add_plugins(ReplicationPlugins)
            .replicate::<TableComponent>()
            .replicate::<SparseSetComponent>()
            .replicate::<IgnoredComponent>()
            .not_replicate_if_present::<IgnoredComponent, ExclusionComponent>()
            .add_plugin(TestNetworkPlugin);

        app.update();
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
        app.add_plugins(ReplicationPlugins)
            .replicate::<MappedComponent>()
            .add_plugin(TestNetworkPlugin);

        app.update();
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
        app.add_plugins(ReplicationPlugins)
            .register_type::<NonReflectedComponent>()
            .add_plugin(TestNetworkPlugin);

        app.update();
        app.update();

        // Mark components as removed.
        const REMOVAL_TICK: u32 = 1; // Should be more then 0 since both client and server starts with 0 tick and think that everything is replicated at this point.
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
        app.add_plugins(ReplicationPlugins)
            .add_plugin(TestNetworkPlugin);

        app.update();
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
    #[reflect(Component, MapEntity)]
    struct MappedComponent(Entity);

    impl MapEntities for MappedComponent {
        fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapEntitiesError> {
            self.0 = entity_map.get(self.0)?;
            Ok(())
        }
    }

    impl FromWorld for MappedComponent {
        fn from_world(_world: &mut World) -> Self {
            Self(Entity::from_raw(u32::MAX))
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
