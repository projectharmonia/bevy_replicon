use std::cmp::Reverse;

use bevy::{
    ecs::{archetype::Archetype, component::ComponentId, entity::MapEntities},
    prelude::*,
    utils::HashSet,
};
use serde::{de::DeserializeOwned, Serialize};

use super::replication_fns::{
    self, DeserializeFn, RemoveFn, RemoveFnId, ReplicationFns, SerdeFns, SerdeFnsId, SerializeFn,
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

    If one group is a subset of another group or component, then the superset will take precedence.
    For example, a rule with [`Transform`] and user's `Player` marker will take precedence over single [`Transform`] rule.

    If you remove a single component from a group, only a single removal will be sent to clients.
    Other group components will continue to be present on both server and clients.
    Replication for them will be stopped, unless they match other rule.

    If an entity archetype matches several overlapping rules, overlapping components will
    be replicated server times (for each rule they match).

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
        let mut fns = self.world.resource_mut::<ReplicationFns>();
        let serde_id = fns.register_serde_fns(SerdeFns {
            serialize,
            deserialize,
        });
        let remove_id = fns.register_remove_fn(remove);

        let mut rules = self.world.resource_mut::<ReplicationRules>();
        rules.insert(ReplicationRule::new(
            vec![(component_id, serde_id)],
            remove_id,
        ));

        self
    }

    fn replicate_group<C: GroupReplication>(&mut self) -> &mut Self {
        let rule = self
            .world
            .resource_scope(|world, mut fns: Mut<ReplicationFns>| C::register(world, &mut fns));

        self.world.resource_mut::<ReplicationRules>().insert(rule);
        self
    }
}

/// All registered rules for components replication.
#[derive(Default, Deref, Resource)]
pub struct ReplicationRules(Vec<ReplicationRule>);

impl ReplicationRules {
    /// Inserts a new rule, maintaining sorting by the number of components in descending order.
    pub fn insert(&mut self, rule: ReplicationRule) {
        match self.binary_search_by_key(&Reverse(rule.len), |rule| Reverse(rule.len)) {
            Ok(index) => self.0.insert(index, rule),
            Err(index) => self.0.insert(index, rule),
        };
    }

    /// Calculates subset rules.
    ///
    /// Should be called only after all [`Self::insert`] and only once.
    pub(crate) fn calculate_subsets(&mut self) {
        for index in 1..self.len() {
            let (left, right) = self.0.split_at_mut(index);
            let left_rule = left
                .last_mut()
                .expect("slice isn't empty because index starts from 1");

            for (right_index, right_rule) in right.iter_mut().enumerate() {
                if left_rule.contains(right_rule) {
                    left_rule.subsets.push(index + right_index);
                } else if right_rule.contains(left_rule) {
                    right_rule.subsets.push(index);
                }
            }
        }
    }
}

/// Describes a replicated group of components that.
pub struct ReplicationRule {
    /// Number of all components in a rule.
    ///
    /// May differ from the length of `components`, which includes only serializable components.
    len: usize,

    /// Rule indexes that are a subset of this rule.
    pub(crate) subsets: Vec<usize>,

    /// Rule components and their serialization and deserialization.
    pub(crate) components: Vec<(ComponentId, SerdeFnsId)>,

    /// ID of the function that removes rule components from [`EntityWorldMut`].
    pub(crate) remove_id: RemoveFnId,
}

impl ReplicationRule {
    /// Creates a new rule with components count equal to the number of serialized components.
    ///
    /// See also [`Self::with_skipped_components`].
    pub fn new(components: Vec<(ComponentId, SerdeFnsId)>, remove_id: RemoveFnId) -> Self {
        Self {
            len: components.len(),
            subsets: Default::default(),
            components,
            remove_id,
        }
    }

    /// Returns a new rule with skipped components taken into account.
    ///
    /// Useful for cases when some components aren't serialized.
    /// For example, a rule with [`Transform`] and user's `Player` marker,
    /// where only [`Transform`] is serialized, will have one serializable component,
    /// but two components overall. This way it will override a rule with only [`Transform`].
    ///
    /// In other words, use it if you skip serialization of some components.
    ///
    /// For usage example see [`GroupReplication`].
    pub fn with_skipped_components(mut self, count: usize) -> Self {
        self.len += count;
        self
    }

    pub(crate) fn matches_archetype(&self, archetype: &Archetype) -> bool {
        self.components
            .iter()
            .all(|&(component_id, _)| archetype.contains(component_id))
    }

    pub(crate) fn matches(&self, components: &HashSet<ComponentId>) -> bool {
        self.components
            .iter()
            .all(|(component_id, _)| components.contains(component_id))
    }

    /// Returns `true` if `other_rule` is a subset of this rule.
    pub(super) fn contains(&self, other_rule: &ReplicationRule) -> bool {
        if self.len < other_rule.len {
            return false;
        }

        for (component_id, _) in &other_rule.components {
            if self
                .components
                .iter()
                .all(|(other_id, _)| component_id != other_id)
            {
                return false;
            }
        }

        true
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
        replication_fns::{self, ReplicationFns, SerdeFns},
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
    fn register(world: &mut World, fns: &mut ReplicationFns) -> ReplicationRule {
        // Customize serlialization to serialize only `translation`.
        let transform_id = world.init_component::<Transform>();
        let transform_serde_id = fns.register_serde_fns(SerdeFns {
            // For function definitions see the example from `AppReplicationExt::replicate_with`.
            serialize: serialize_translation,
            deserialize: deserialize_translation,
        });

        // Serialize `player` as usual.
        let visibility_id = world.init_component::<Player>();
        let visibility_serde_id = fns.register_serde_fns(SerdeFns {
            serialize: replication_fns::serialize::<Player>,
            deserialize: replication_fns::deserialize::<Player>,
        });

        // We skip `replication` registration since it's automatically inserted on
        // client after replicaiton and deserialization from scenes.
        // Any other components that inserted after replication,
        // like components that initialize "blueprints", can be skipped as well.

        let components = vec![
            (transform_id, transform_serde_id),
            (visibility_id, visibility_serde_id),
        ];
        let remove_id = fns.register_remove_fn(replication_fns::remove::<(Transform, Player)>);

        ReplicationRule::new(components, remove_id).with_skipped_components(1)
    }
}

# fn serialize_translation(_: Ptr, _: &mut Cursor<Vec<u8>>) -> bincode::Result<()> { unimplemented!() }
# fn deserialize_translation(_: &mut EntityWorldMut, _: &mut ServerEntityMap, _: &mut Cursor<&[u8]>, _: RepliconTick) -> bincode::Result<()> { unimplemented!() }
```
**/
pub trait GroupReplication {
    /// Creates the associated replication rules and register its functions in [`ReplicationFns`].
    fn register(world: &mut World, fns: &mut ReplicationFns) -> ReplicationRule;
}

macro_rules! impl_registrations {
    ($($type:ident),*) => {
        impl<$($type: Component + Serialize + DeserializeOwned),*> GroupReplication for ($($type,)*) {
            fn register(world: &mut World, fns: &mut ReplicationFns) -> ReplicationRule {
                // TODO: initialize with capacity after stabilization: https://github.com/rust-lang/rust/pull/122808
                let mut components = Vec::new();
                $(
                    let component_id = world.init_component::<$type>();
                    let serde_id = fns.register_serde_fns(SerdeFns {
                        serialize: replication_fns::serialize::<$type>,
                        deserialize: replication_fns::deserialize::<$type>,
                    });
                    components.push((component_id, serde_id));
                )*
                let remove_id = fns.register_remove_fn(replication_fns::remove::<($($type),*,)>);

                ReplicationRule::new(
                    components,
                    remove_id,
                )
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
