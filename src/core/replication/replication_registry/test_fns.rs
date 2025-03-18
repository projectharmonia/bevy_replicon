use bevy::{ecs::world::CommandQueue, prelude::*};
use bytes::Bytes;

use super::{
    FnsId, ReplicationRegistry,
    ctx::{DespawnCtx, RemoveCtx, SerializeCtx, WriteCtx},
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

entity.apply_write(data, fns_id, tick);
assert!(entity.contains::<DummyComponent>());

entity.apply_remove(fns_id, tick);
assert!(!entity.contains::<DummyComponent>());

entity.apply_despawn(tick);
let mut replicated = app.world_mut().query::<&DummyComponent>();
assert_eq!(replicated.iter(app.world()).len(), 0);

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
    /// See also [`AppMarkerExt`](crate::core::replication::command_markers::AppMarkerExt).
    fn apply_write(
        &mut self,
        bytes: impl Into<Bytes>,
        fns_id: FnsId,
        message_tick: RepliconTick,
    ) -> &mut Self;

    /// Removes a component using a registered function for it.
    ///
    /// See also [`AppMarkerExt`](crate::core::replication::command_markers::AppMarkerExt).
    fn apply_remove(&mut self, fns_id: FnsId, message_tick: RepliconTick) -> &mut Self;

    /// Despawns an entity using [`ReplicationRegistry::despawn`].
    fn apply_despawn(self, message_tick: RepliconTick);
}

impl TestFnsEntityExt for EntityWorldMut<'_> {
    fn serialize(&mut self, fns_id: FnsId, server_tick: RepliconTick) -> Vec<u8> {
        let type_registry = self.world().resource::<AppTypeRegistry>();
        let registry = self.world().resource::<ReplicationRegistry>();
        let (component_id, component_fns, rule_fns) = registry.get(fns_id);
        let mut message = Vec::new();
        let ctx = SerializeCtx {
            server_tick,
            component_id,
            type_registry: &type_registry.read(),
        };
        let ptr = self.get_by_id(component_id).unwrap_or_else(|_| {
            let components = self.world().components();
            let component_name = components
                .get_name(component_id)
                .expect("function should require valid component ID");
            panic!("serialization function require entity to have {component_name}");
        });

        unsafe {
            component_fns
                .serialize(&ctx, rule_fns, ptr, &mut message)
                .expect("serialization into memory should never fail");
        }

        message
    }

    fn apply_write(
        &mut self,
        data: impl Into<Bytes>,
        fns_id: FnsId,
        message_tick: RepliconTick,
    ) -> &mut Self {
        let mut entity_markers = self.world_scope(EntityMarkers::from_world);
        let command_markers = self.world().resource::<CommandMarkers>();
        entity_markers.read(command_markers, &*self);

        let entity = self.id();
        self.world_scope(|world| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                    world.resource_scope(|world, type_registry: Mut<AppTypeRegistry>| {
                        let mut queue = CommandQueue::default();
                        let mut entity = DeferredEntity::new(world, entity);
                        let mut commands = entity.commands(&mut queue);

                        let (component_id, component_fns, rule_fns) = registry.get(fns_id);
                        let mut ctx = WriteCtx {
                            commands: &mut commands,
                            entity_map: &mut entity_map,
                            type_registry: &type_registry.read(),
                            component_id,
                            message_tick,
                            ignore_mapping: false,
                        };

                        unsafe {
                            component_fns
                                .write(
                                    &mut ctx,
                                    rule_fns,
                                    &entity_markers,
                                    &mut entity,
                                    &mut data.into(),
                                )
                                .expect("writing data into an entity shouldn't fail");
                        }

                        queue.apply(world);
                    })
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
                let mut queue = CommandQueue::default();
                let mut entity = DeferredEntity::new(world, entity);
                let mut commands = entity.commands(&mut queue);

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
