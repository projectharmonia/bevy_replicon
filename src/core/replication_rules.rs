use std::cmp::Reverse;

use bevy::{
    ecs::{archetype::Archetype, component::ComponentId, entity::MapEntities},
    prelude::*,
    utils::HashSet,
};
use serde::{de::DeserializeOwned, Serialize};

use super::replication_fns::{
    self, ComponentFns, ComponentFnsId, DeserializeFn, RemoveFn, ReplicationFns, SerializeFn,
};

/// Replication functions for [`App`].
pub trait AppReplicationExt {
    /// Creates a replication rule for a single component.
    ///
    /// The component will be replicated if its entity contains [`Replication`](super::Replication) marker component.
    ///
    /// Component will be serialized and deserialized as is using bincode.
    /// To customize how component will be serialized, use [`Self::replicate_group`].
    ///
    /// If your component contains any [`Entity`] inside, use [`Self::replicate_mapped`].
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.replicate_with::<C>(
            replication_fns::serialize::<C>,
            replication_fns::deserialize::<C>,
            replication_fns::remove::<C>,
        );
        self
    }

    /// Same as [`Self::replicate`], but additionally maps server entities to client inside the component after receiving.
    ///
    /// Always use it for components that contain entities.
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapEntities,
    {
        self.replicate_with::<C>(
            replication_fns::serialize::<C>,
            replication_fns::deserialize_mapped::<C>,
            replication_fns::remove::<C>,
        );
        self
    }

    /**
    Same as [`Self::replicate`], but uses the specified functions for serialization, deserialization, and removal.

    Can be used to customize how the component will be replicated or
    for components that doesn't implement [`Serialize`] or [`DeserializeOwned`].

    # Examples

    ```
    use std::io::Cursor;

    use bevy::{prelude::*, ptr::Ptr};
    use bevy_replicon::{
        client::client_mapper::ServerEntityMap,
        core::{replication_fns, replicon_tick::RepliconTick},
        prelude::*,
    };

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.replicate_with::<Transform>(
        serialize_translation,
        deserialize_translation,
        replication_fns::remove::<Transform>,
    );

    /// Serializes only `translation` from [`Transform`].
    fn serialize_translation(component: Ptr, cursor: &mut Cursor<Vec<u8>>) -> bincode::Result<()> {
        // SAFETY: function called for registered `ComponentId`.
        let transform: &Transform = unsafe { component.deref() };
        bincode::serialize_into(cursor, &transform.translation)
    }

    /// Deserializes `translation` and creates [`Transform`] from it.
    fn deserialize_translation(
        entity: &mut EntityWorldMut,
        _entity_map: &mut ServerEntityMap,
        cursor: &mut Cursor<&[u8]>,
        _replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        let translation: Vec3 = bincode::deserialize_from(cursor)?;
        entity.insert(Transform::from_translation(translation));

        Ok(())
    }
    ```

    The used [`remove`](replication_fns::remove) is the default component removal,
    but you can replace it with your own as well.
    */
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveFn,
    ) -> &mut Self
    where
        C: Component;

    /**
    Creates a replication rule for a group of components.

    Group will only be replicated if all its components are present on the entity.

    If a group contains a single component, it will work exactly as [`Self::replicate`].

    If an entity matches multiple groups, functions from a group with bigger priority
    will take precedence for overlapping components. For example, a rule with [`Transform`]
    and user's `Player` marker will take precedence over single [`Transform`] rule.

    If you remove a single component from a group, only a single removal will be sent to clients.
    Other group components will continue to be present on both server and clients.
    Replication for them will be stopped, unless they match other rule.

    We provide blanket impls for tuples to replicate them as is, but user could manually implement the trait
    to customize how components will be serialized, deserialized and removed. For details see [`GroupReplication`].

    # Panics

    Panics if `debug_assertions` are enabled and any rule is a subset of another.

    # Examples

    Replicate [`Transform`] and user's `Player` marker only if both of them are present on an entity:

    ```
    use bevy::prelude::*;
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.replicate_group::<(Transform, Player)>();

    #[derive(Component, Deserialize, Serialize)]
    struct Player;
    # /// To avoid enabling `serialize` feature on Bevy.
    # #[derive(Component, Deserialize, Serialize)]
    # struct Transform;
    ```
    **/
    fn replicate_group<C: GroupReplication>(&mut self) -> &mut Self;
}

impl AppReplicationExt for App {
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveFn,
    ) -> &mut Self
    where
        C: Component,
    {
        let component_id = self.world.init_component::<C>();
        let mut replication_fns = self.world.resource_mut::<ReplicationFns>();
        let fns_id = replication_fns.register_fns(ComponentFns {
            serialize,
            deserialize,
            remove,
        });

        let mut rules = self.world.resource_mut::<ReplicationRules>();
        rules.insert(ReplicationRule::new(vec![(component_id, fns_id)]));

        self
    }

    fn replicate_group<C: GroupReplication>(&mut self) -> &mut Self {
        let rule = self
            .world
            .resource_scope(|world, mut replicaiton_fns: Mut<ReplicationFns>| {
                C::register(world, &mut replicaiton_fns)
            });

        self.world.resource_mut::<ReplicationRules>().insert(rule);
        self
    }
}

/// All registered rules for components replication.
#[derive(Default, Deref, Resource)]
pub struct ReplicationRules(Vec<ReplicationRule>);

impl ReplicationRules {
    /// Inserts a new rule, maintaining sorting by their priority in descending order.
    pub fn insert(&mut self, rule: ReplicationRule) {
        match self.binary_search_by_key(&Reverse(rule.priority), |rule| Reverse(rule.priority)) {
            Ok(index) => self.0.insert(index, rule),
            Err(index) => self.0.insert(index, rule),
        };
    }
}

/// Describes a replicated group of components that.
pub struct ReplicationRule {
    /// Functions priority.
    ///
    /// Usually equal to the number of serialized components,
    /// but can be adjusted by user.
    pub priority: usize,

    /// Rule components and their serialization and deserialization.
    pub components: Vec<(ComponentId, ComponentFnsId)>,
}

impl ReplicationRule {
    /// Creates a new rule with priority equal to the number of serialized components.
    pub fn new(components: Vec<(ComponentId, ComponentFnsId)>) -> Self {
        Self {
            priority: components.len(),
            components,
        }
    }

    /// Determines whether an archetype contains all components required by the rule.
    pub(crate) fn matches(&self, archetype: &Archetype) -> bool {
        self.components
            .iter()
            .all(|&(component_id, _)| archetype.contains(component_id))
    }

    /// Determines whether the rule is applicable to an archetype with removals included and contains at least one removal.
    pub(crate) fn matches_removals(
        &self,
        archetype: &Archetype,
        removed_components: &HashSet<ComponentId>,
    ) -> bool {
        let mut matches = false;
        for &(component_id, _) in &self.components {
            if removed_components.contains(&component_id) {
                matches = true;
            } else if !archetype.contains(component_id) {
                return false;
            }
        }

        matches
    }
}

/**
Describes how component group should be serialized, deserialized and removed.

Can be implemented on any struct to create a custom replication group.

# Examples

```
use std::io::Cursor;

use bevy::{prelude::*, ptr::Ptr};
use bevy_replicon::{
    client::client_mapper::ServerEntityMap,
    core::{
        replication_rules::{self, GroupReplication, ReplicationRule},
        replication_fns::{self, ReplicationFns, ComponentFns},
        replicon_tick::RepliconTick,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};

# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.replicate_group::<PlayerBundle>();

#[derive(Bundle)]
struct PlayerBundle {
    transform: Transform,
    player: Player,
    replication: Replication,
}

#[derive(Component, Deserialize, Serialize)]
struct Player;

impl GroupReplication for PlayerBundle {
    fn register(world: &mut World, replication_fns: &mut ReplicationFns) -> ReplicationRule {
        // Customize serlialization to serialize only `translation`.
        let transform_id = world.init_component::<Transform>();
        let transform_fns_id = replication_fns.register_fns(ComponentFns {
            // For function definitions see the example from `AppReplicationExt::replicate_with`.
            serialize: serialize_translation,
            deserialize: deserialize_translation,
            remove: replication_fns::remove::<Transform>, // Use default removal function.
        });

        // Serialize `player` as usual.
        let visibility_id = world.init_component::<Player>();
        let visibility_fns_id = replication_fns.register_fns(ComponentFns {
            serialize: replication_fns::serialize::<Player>,
            deserialize: replication_fns::deserialize::<Player>,
            remove: replication_fns::remove::<Player>,
        });

        // We skip `replication` registration since it's a special component.
        // It automatically inserted on client after replicaiton and
        // deserialization from scenes.

        let components = vec![
            (transform_id, transform_fns_id),
            (visibility_id, visibility_fns_id),
        ];

        ReplicationRule::new(components)
    }
}

# fn serialize_translation(_: Ptr, _: &mut Cursor<Vec<u8>>) -> bincode::Result<()> { unimplemented!() }
# fn deserialize_translation(_: &mut EntityWorldMut, _: &mut ServerEntityMap, _: &mut Cursor<&[u8]>, _: RepliconTick) -> bincode::Result<()> { unimplemented!() }
```
**/
pub trait GroupReplication {
    /// Creates the associated replication rules and register its functions in [`ReplicationFns`].
    fn register(world: &mut World, replication_fns: &mut ReplicationFns) -> ReplicationRule;
}

macro_rules! impl_registrations {
    ($($type:ident),*) => {
        impl<$($type: Component + Serialize + DeserializeOwned),*> GroupReplication for ($($type,)*) {
            fn register(world: &mut World, replication_fns: &mut ReplicationFns) -> ReplicationRule {
                // TODO: initialize with capacity after stabilization: https://github.com/rust-lang/rust/pull/122808
                let mut components = Vec::new();
                $(
                    let component_id = world.init_component::<$type>();
                    let fns_id = replication_fns.register_fns(ComponentFns {
                        serialize: replication_fns::serialize::<$type>,
                        deserialize: replication_fns::deserialize::<$type>,
                        remove: replication_fns::remove::<$type>,
                    });
                    components.push((component_id, fns_id));
                )*

                ReplicationRule::new(components)
            }
        }
    }
}

impl_registrations!(A);
impl_registrations!(A, B);
impl_registrations!(A, B, C);
impl_registrations!(A, B, C, D);
impl_registrations!(A, B, C, D, E);
impl_registrations!(A, B, C, D, E, F);
impl_registrations!(A, B, C, D, E, F, G);
impl_registrations!(A, B, C, D, E, F, G, H);
impl_registrations!(A, B, C, D, E, F, G, H, I);
impl_registrations!(A, B, C, D, E, F, G, H, I, J);

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{core::replication_fns::ReplicationFns, AppReplicationExt};

    #[test]
    fn sorting() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationFns>()
            .replicate::<ComponentA>()
            .replicate::<ComponentB>()
            .replicate_group::<(ComponentA, ComponentB)>()
            .replicate_group::<(ComponentB, ComponentC)>()
            .replicate::<ComponentC>()
            .replicate::<ComponentD>();

        let replication_rules = app.world.resource::<ReplicationRules>();
        let lens: Vec<_> = replication_rules.iter().map(|rule| rule.priority).collect();
        assert_eq!(lens, [2, 2, 1, 1, 1, 1]);
    }

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentA;

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentB;

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentC;

    #[derive(Serialize, Deserialize, Component)]
    struct ComponentD;
}
