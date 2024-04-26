use std::io::Cursor;

use bevy::{
    ecs::system::{CommandQueue, SystemState},
    prelude::*,
};

use super::{
    ctx::{RemoveDespawnCtx, WriteDeserializeCtx},
    FnsInfo,
};
use crate::{
    client::server_entity_map::ServerEntityMap,
    core::{
        command_markers::CommandMarkers,
        replication_fns::{ctx::SerializeCtx, ReplicationFns},
        replicon_tick::RepliconTick,
    },
};

/**
Extension for [`EntityWorldMut`] to call registered replication functions for [`FnsInfo`].

See also [`ReplicationFns::register_rule_fns`].

# Example

This example shows how to call registered functions on an entity:

```
use bevy::prelude::*;
use bevy_replicon::{
    core::{
        replication_fns::{rule_fns::RuleFns, test_fns::TestFnsEntityExt, ReplicationFns},
        replicon_tick::RepliconTick,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};

let mut app = App::new();
app.add_plugins((MinimalPlugins, RepliconPlugins));

let replicon_tick = *app.world.resource::<RepliconTick>();

// Register rule functions manually to obtain `FnsInfo`.
let fns_info = app
    .world
    .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
        replication_fns.register_rule_fns(world, RuleFns::<DummyComponent>::default())
    });

let mut entity = app.world.spawn(DummyComponent);
let data = entity.serialize(fns_info);
entity.remove::<DummyComponent>();

entity.apply_write(&data, fns_info, replicon_tick);
assert!(entity.contains::<DummyComponent>());

entity.apply_remove(fns_info, replicon_tick);
assert!(!entity.contains::<DummyComponent>());

entity.apply_despawn(replicon_tick);
assert!(app.world.entities().is_empty());

#[derive(Component, Serialize, Deserialize)]
struct DummyComponent;
```
**/
pub trait TestFnsEntityExt {
    /// Returns a component serialized using a registered function for it.
    ///
    /// See also [`ReplicationFns::register_rule_fns`].
    fn serialize(&mut self, fns_info: FnsInfo) -> Vec<u8>;

    /// Deserializes a component using a registered function for it and
    /// writes it into an entity using a write function based on markers.
    ///
    /// See also [`AppMarkerExt`](crate::core::command_markers::AppMarkerExt).
    fn apply_write(
        &mut self,
        data: &[u8],
        fns_info: FnsInfo,
        message_tick: RepliconTick,
    ) -> &mut Self;

    /// Remvoes a component using a registered function for it.
    ///
    /// See also [`AppMarkerExt`](crate::core::command_markers::AppMarkerExt).
    fn apply_remove(&mut self, fns_info: FnsInfo, message_tick: RepliconTick) -> &mut Self;

    /// Despawns an entity using [`ReplicationFns::despawn`].
    fn apply_despawn(self, message_tick: RepliconTick);
}

impl TestFnsEntityExt for EntityWorldMut<'_> {
    fn serialize(&mut self, fns_info: FnsInfo) -> Vec<u8> {
        let replication_fns = self.world().resource::<ReplicationFns>();
        let (component_fns, rule_fns) = replication_fns.get(fns_info.fns_id());
        let replicon_tick = *self.world().resource::<RepliconTick>();
        let mut cursor = Cursor::default();
        let ctx = SerializeCtx { replicon_tick };
        let ptr = self.get_by_id(fns_info.component_id()).unwrap_or_else(|| {
            let components = self.world().components();
            let component_name = components
                .get_name(fns_info.component_id())
                .expect("function should require valid component ID");
            panic!("serialization function require entity to have {component_name}");
        });

        unsafe {
            component_fns
                .serialize(&ctx, rule_fns, ptr, &mut cursor)
                .expect("serialization into memory should never fail");
        }

        cursor.into_inner()
    }

    fn apply_write(
        &mut self,
        data: &[u8],
        fns_info: FnsInfo,
        message_tick: RepliconTick,
    ) -> &mut Self {
        let entity = self.id();
        self.world_scope(|world| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, command_markers: Mut<CommandMarkers>| {
                    world.resource_scope(|world, replication_fns: Mut<ReplicationFns>| {
                        let mut state = SystemState::<(Commands, Query<EntityMut>)>::new(world);
                        let (mut commands, mut query) = state.get_mut(world);
                        let mut entity = query.get_mut(entity).unwrap();
                        let entity_markers: Vec<_> =
                            command_markers.iter_contains(&entity).collect();

                        let (component_fns, rule_fns) = replication_fns.get(fns_info.fns_id());
                        let mut cursor = Cursor::new(data);
                        let mut ctx = WriteDeserializeCtx {
                            commands: &mut commands,
                            entity_map: &mut entity_map,
                            message_tick,
                        };

                        unsafe {
                            component_fns
                                .write(
                                    &mut ctx,
                                    rule_fns,
                                    &entity_markers,
                                    &mut entity,
                                    &mut cursor,
                                )
                                .expect("writing data into an entity shouldn't fail");
                        }

                        state.apply(world);
                    })
                })
            })
        });

        self
    }

    fn apply_remove(&mut self, fns_info: FnsInfo, message_tick: RepliconTick) -> &mut Self {
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, self.world());
        let entity = commands.entity(self.id());

        let command_markers = self.world().resource::<CommandMarkers>();
        let entity_markers: Vec<_> = command_markers.iter_contains(&*self).collect();

        let replication_fns = self.world().resource::<ReplicationFns>();
        let (component_fns, _) = replication_fns.get(fns_info.fns_id());
        let ctx = RemoveDespawnCtx { message_tick };

        component_fns.remove(&ctx, &entity_markers, entity);

        self.world_scope(|world| {
            queue.apply(world);
        });

        self
    }

    fn apply_despawn(self, message_tick: RepliconTick) {
        let replication_fns = self.world().resource::<ReplicationFns>();
        let ctx = RemoveDespawnCtx { message_tick };
        (replication_fns.despawn)(&ctx, self);
    }
}
