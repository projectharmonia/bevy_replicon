use std::io::Cursor;

use bevy::{ecs::world::CommandQueue, prelude::*};

use super::{
    ctx::{DespawnCtx, RemoveCtx, SerializeCtx, WriteCtx},
    FnsId, ReplicationRegistry,
};
use crate::core::{
    replication::{
        command_markers::{CommandMarkers, EntityMarkers},
        deferred_entity::DeferredEntity,
    },
    replicon_tick::RepliconTick,
    server_entity_map::ServerEntityMap,
};

/**
Extension for [`EntityWorldMut`] to call registered replication functions for [`FnsId`].

See also [`ReplicationRegistry::register_rule_fns`].

# Example

This example shows how to call registered functions on an entity:

```
use bevy::prelude::*;
use bevy_replicon::{
    core::{
        replication::replication_registry::{
            rule_fns::RuleFns, test_fns::TestFnsEntityExt, ReplicationRegistry,
        },
        replicon_tick::RepliconTick,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};

let mut app = App::new();
app.add_plugins((MinimalPlugins, RepliconPlugins));

let tick = RepliconTick::default();

// Register rule functions manually to obtain `FnsId`.
let (_, fns_id) = app
    .world_mut()
    .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
        registry.register_rule_fns(world, RuleFns::<DummyComponent>::default())
    });

let mut entity = app.world_mut().spawn(DummyComponent);
let data = entity.serialize(fns_id, tick);
entity.remove::<DummyComponent>();

entity.apply_write(&data, fns_id, tick);
assert!(entity.contains::<DummyComponent>());

entity.apply_remove(fns_id, tick);
assert!(!entity.contains::<DummyComponent>());

entity.apply_despawn(tick);
assert!(app.world().entities().is_empty());

#[derive(Component, Serialize, Deserialize)]
struct DummyComponent;
```
**/
pub trait TestFnsEntityExt {
    /// Returns a component serialized using a registered function for it.
    ///
    /// See also [`ReplicationRegistry::register_rule_fns`].
    #[must_use]
    fn serialize(&mut self, fns_id: FnsId, server_tick: RepliconTick) -> Vec<u8>;

    /// Deserializes a component using a registered function for it and
    /// writes it into an entity using a write function based on markers.
    ///
    /// See also [`AppMarkerExt`](crate::core::command_markers::AppMarkerExt).
    fn apply_write(&mut self, data: &[u8], fns_id: FnsId, message_tick: RepliconTick) -> &mut Self;

    /// Remvoes a component using a registered function for it.
    ///
    /// See also [`AppMarkerExt`](crate::core::command_markers::AppMarkerExt).
    fn apply_remove(&mut self, fns_id: FnsId, message_tick: RepliconTick) -> &mut Self;

    /// Despawns an entity using [`ReplicationRegistry::despawn`].
    fn apply_despawn(self, message_tick: RepliconTick);
}

impl TestFnsEntityExt for EntityWorldMut<'_> {
    fn serialize(&mut self, fns_id: FnsId, server_tick: RepliconTick) -> Vec<u8> {
        let registry = self.world().resource::<ReplicationRegistry>();
        let (component_id, component_fns, rule_fns) = registry.get(fns_id);
        let mut cursor = Cursor::default();
        let ctx = SerializeCtx {
            server_tick,
            component_id,
        };
        let ptr = self.get_by_id(component_id).unwrap_or_else(|| {
            let components = self.world().components();
            let component_name = components
                .get_name(component_id)
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

    fn apply_write(&mut self, data: &[u8], fns_id: FnsId, message_tick: RepliconTick) -> &mut Self {
        let mut entity_markers = self.world_scope(EntityMarkers::from_world);
        let command_markers = self.world().resource::<CommandMarkers>();
        entity_markers.read(command_markers, &*self);

        let entity = self.id();
        self.world_scope(|world| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                    let world_cell = world.as_unsafe_world_cell();
                    // SAFETY: have write access and the cell used only to get entities.
                    let mut entity = unsafe { DeferredEntity::new(world_cell, entity) };
                    let mut queue = CommandQueue::default();
                    let mut commands =
                        Commands::new_from_entities(&mut queue, world_cell.entities());

                    let (component_id, component_fns, rule_fns) = registry.get(fns_id);
                    let mut cursor = Cursor::new(data);
                    let mut ctx =
                        WriteCtx::new(&mut commands, &mut entity_map, component_id, message_tick);

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

                    queue.apply(world);
                })
            })
        });

        self
    }

    fn apply_remove(&mut self, fns_id: FnsId, message_tick: RepliconTick) -> &mut Self {
        let mut entity_markers = self.world_scope(EntityMarkers::from_world);
        let command_markers = self.world().resource::<CommandMarkers>();
        entity_markers.read(command_markers, &*self);

        let entity = self.id();
        self.world_scope(|world| {
            world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                let world_cell = world.as_unsafe_world_cell();
                // SAFETY: have write access and the cell used only to get entities.
                let mut entity = unsafe { DeferredEntity::new(world_cell, entity) };
                let mut queue = CommandQueue::default();
                let mut commands = Commands::new_from_entities(&mut queue, world_cell.entities());

                let (component_id, component_fns, _) = registry.get(fns_id);
                let mut ctx = RemoveCtx {
                    commands: &mut commands,
                    message_tick,
                    component_id,
                };

                component_fns.remove(&mut ctx, &entity_markers, &mut entity);

                queue.apply(world);
            })
        });

        self
    }

    fn apply_despawn(self, message_tick: RepliconTick) {
        let registry = self.world().resource::<ReplicationRegistry>();
        let ctx = DespawnCtx { message_tick };
        (registry.despawn)(&ctx, self);
    }
}
