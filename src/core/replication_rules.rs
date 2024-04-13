use std::cmp::Reverse;

use bevy::{
    ecs::{archetype::Archetype, component::ComponentId, entity::MapEntities},
    prelude::*,
    utils::HashSet,
};
use serde::{de::DeserializeOwned, Serialize};

use super::replication_fns::{
    serde_fns::{self, DeserializeFn, DeserializeInPlaceFn, SerializeFn},
    FnsInfo, ReplicationFns,
};

/// Replication functions for [`App`].
pub trait AppRuleExt {
    /// Creates a replication rule for a single component.
    ///
    /// The component will be replicated if its entity contains the [`Replication`](super::Replication)
    /// marker component.
    ///
    /// Component will be serialized and deserialized as-is using bincode.
    /// To customize it, use [`Self::replicate_group`].
    ///
    /// If your component contains any [`Entity`] inside, use [`Self::replicate_mapped`].
    ///
    /// See also [`Self::replicate_with`].
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.replicate_with::<C>(
            serde_fns::serialize::<C>,
            serde_fns::deserialize::<C>,
            serde_fns::deserialize_in_place::<C>,
        )
    }

    /// Same as [`Self::replicate`], but additionally maps server entities to client inside the component after receiving.
    ///
    /// Always use it for components that contain entities.
    ///
    /// See also [`Self::replicate`].
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapEntities,
    {
        // SAFETY: default functions for the same component.
        self.replicate_with::<C>(
            serde_fns::serialize::<C>,
            serde_fns::deserialize_mapped::<C>,
            serde_fns::deserialize_in_place::<C>,
        )
    }

    /**
    Same as [`Self::replicate`], but uses the specified functions for serialization and deserialization.

    Can be used to customize how the component will be passed over the network or
    for components that don't implement [`Serialize`] or [`DeserializeOwned`].

    `deserialize_in_place` will be called on receive if a component is already present.
    By default it just calls the registered `deserialize` function.
    But it could be used to optimize deserialization of components that require allocations.

    You can also override how the component will be written,
    see [`AppMarkerExt`](super::command_markers::AppMarkerExt).

    # Examples

    ```
    use std::io::Cursor;

    use bevy::prelude::*;
    use bevy_replicon::{
        client::client_mapper::ClientMapper, core::replication_fns::serde_fns, prelude::*,
    };

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.replicate_with::<Transform>(
        serialize_translation,
        deserialize_translation,
        serde_fns::deserialize_in_place, // Use default function which just calls `deserialize_translation`.
    );

    /// Serializes only `translation` from [`Transform`].
    fn serialize_translation(
        transform: &Transform,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        bincode::serialize_into(cursor, &transform.translation)
    }

    /// Deserializes `translation` and creates [`Transform`] from it.
    fn deserialize_translation(
        cursor: &mut Cursor<&[u8]>,
        _mapper: &mut ClientMapper,
    ) -> bincode::Result<Transform> {
        let translation: Vec3 = bincode::deserialize_from(cursor)?;
        Ok(Transform::from_translation(translation))
    }
    ```
    */
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> &mut Self
    where
        C: Component;

    /**
    Creates a replication rule for a group of components.

    A group will only be replicated if all its components are present on the entity.

    If a group contains a single component, it will work the same as [`Self::replicate`].

    If an entity matches multiple groups, functions from a group with higher priority
    will take precedence for overlapping components. For example, a rule with [`Transform`]
    and a `Player` marker will take precedence over a single [`Transform`] rule.

    If you remove a single component from a group, only a single removal will be sent to clients.
    Other group components will continue to be present on both server and clients.
    Replication for them will be stopped, unless they match other rules.

    We provide blanket impls for tuples to replicate them as-is, but a user could manually implement the trait
    to customize how components will be serialized, deserialized, written and removed. For details see [`GroupReplication`].

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
    ```
    **/
    fn replicate_group<C: GroupReplication>(&mut self) -> &mut Self;
}

impl AppRuleExt for App {
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> &mut Self
    where
        C: Component,
    {
        let rule = self
            .world
            .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
                let fns_info = replication_fns.register_serde_fns(
                    world,
                    serialize,
                    deserialize,
                    deserialize_in_place,
                );
                ReplicationRule::new(vec![fns_info])
            });

        self.world.resource_mut::<ReplicationRules>().insert(rule);
        self
    }

    fn replicate_group<C: GroupReplication>(&mut self) -> &mut Self {
        let rule = self
            .world
            .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
                C::register(world, &mut replication_fns)
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

/// Describes a replicated component or a group of components.
pub struct ReplicationRule {
    /// Priority for this rule.
    ///
    /// Usually equal to the number of serialized components,
    /// but can be adjusted by the user.
    priority: usize,

    /// Rule components and their serialization/deserialization/removal functions.
    components: Vec<FnsInfo>,
}

impl ReplicationRule {
    /// Creates a new rule with priority equal to the number of serializable components.
    pub fn new(components: Vec<FnsInfo>) -> Self {
        Self::with_priority(components.len(), components)
    }

    /// Like [`Self::new`], but allows to set a priority different from the number of serializable components.
    pub fn with_priority(priority: usize, components: Vec<FnsInfo>) -> Self {
        Self {
            priority,
            components,
        }
    }

    /// Returns associated components and functions IDs.
    pub(crate) fn components(&self) -> &[FnsInfo] {
        &self.components
    }

    /// Returns associated priority.
    pub(crate) fn priority(&self) -> usize {
        self.priority
    }

    /// Determines whether an archetype contains all components required by the rule.
    pub(crate) fn matches(&self, archetype: &Archetype) -> bool {
        self.components
            .iter()
            .all(|fns_info| archetype.contains(fns_info.component_id()))
    }

    /// Determines whether the rule is applicable to an archetype with removals included and contains at least one removal.
    ///
    /// Returns `true` if all components in this rule are found in either `removed_components` or the
    /// `post_removal_archetype`, and at least one component is found in `removed_components`.
    /// Returning true means the entity with this archetype satisfied this
    /// rule in the previous tick, but then a component within this rule was removed from the entity.
    pub(crate) fn matches_removals(
        &self,
        post_removal_archetype: &Archetype,
        removed_components: &HashSet<ComponentId>,
    ) -> bool {
        let mut matches = false;
        for fns_info in &self.components {
            if removed_components.contains(&fns_info.component_id()) {
                matches = true;
            } else if !post_removal_archetype.contains(fns_info.component_id()) {
                return false;
            }
        }

        matches
    }
}

/**
Describes how a component group should be serialized, deserialized, written, and removed.

Can be implemented on any struct to create a custom replication group.

# Examples

```
use std::io::Cursor;

use bevy::prelude::*;
use bevy_replicon::{
    client::client_mapper::ClientMapper,
    core::{
        replication_fns::{serde_fns, ReplicationFns},
        replication_rules::{GroupReplication, ReplicationRule},
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
        let transform_info = replication_fns.register_serde_fns(
            world,
            serialize_translation,
            deserialize_translation,
            serde_fns::deserialize_in_place,
        );

        // Serialize `player` as usual.
        let player_info = replication_fns.register_default_serde_fns::<Player>(world);

        // We skip `replication` registration since it's a special component.
        // It's automatically inserted on clients after replication and
        // deserialization from scenes.

        ReplicationRule::new(vec![transform_info, player_info])
    }
}

# fn serialize_translation(_: &Transform, _: &mut Cursor<Vec<u8>>) -> bincode::Result<()> { unimplemented!() }
# fn deserialize_translation(_: &mut Cursor<&[u8]>, _: &mut ClientMapper) -> bincode::Result<Transform> { unimplemented!() }
```
**/
pub trait GroupReplication {
    /// Creates the associated replication rules and registers its functions in [`ReplicationFns`].
    fn register(world: &mut World, replication_fns: &mut ReplicationFns) -> ReplicationRule;
}

macro_rules! impl_registrations {
    ($($type:ident),*) => {
        impl<$($type: Component + Serialize + DeserializeOwned),*> GroupReplication for ($($type,)*) {
            fn register(world: &mut World, replication_fns: &mut ReplicationFns) -> ReplicationRule {
                // TODO: initialize with capacity after stabilization: https://github.com/rust-lang/rust/pull/122808
                let mut components = Vec::new();
                $(
                    let fns_info = replication_fns.register_default_serde_fns::<$type>(world);
                    components.push(fns_info);
                )*

                ReplicationRule::new(components)
            }
        }
    }
}

bevy::utils::all_tuples!(impl_registrations, 1, 15, B);

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{core::replication_fns::ReplicationFns, AppRuleExt};

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
