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
    /// Marks single component for replication.
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

    The used [`remove`] is the default component removal,
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
    Marks group of components for replication.

    Group will only be replicated if all its components are present on the entity.
    Never remove grouped components from an entity partially, you can only **remove the whole group at once**.

    We provide blanket impls for tuples to replicate them as is, but user could manually implement the trait
    to customize how components will be serialized, deserialized and removed. For details see [`GroupReplication`].

    If a group contains a single component, it will work exactly as [`Self::replicate`].

    Never register rules where one is a subset of another.
    For example, if you registered a single `Player`, never register `(Player, Human)`.

    # Panics

    Panics if `debug_assertions` are enabled and any rule is a subset of another.

    # Examples

    Replicate [`Transform`] and `Player` only if both of them are present on an entity:

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
        let serde_id = replication_fns.register_serde_fns(SerdeFns {
            serialize,
            deserialize,
        });
        let remove_id = replication_fns.register_remove_fn(remove);

        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules.push(ReplicationRule {
            components: vec![(component_id, serde_id)],
            remove_id,
        });

        self
    }

    fn replicate_group<C: GroupReplication>(&mut self) -> &mut Self {
        let replication_rule =
            self.world
                .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
                    C::register(world, &mut replication_fns)
                });

        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules.push(replication_rule);

        self
    }
}

/// All registered rules for components replication.
#[derive(Default, Deref, Resource)]
pub struct ReplicationRules(Vec<ReplicationRule>);

impl ReplicationRules {
    fn push(&mut self, replication_rule: ReplicationRule) {
        self.0.push(replication_rule);
    }
}

/// Describes how component or a group of components will be serialized, deserialized and removed.
pub struct ReplicationRule {
    /// Rule components and their serialization and deserialization.
    pub components: Vec<(ComponentId, SerdeFnsId)>,

    /// ID of the function that removes rule components from [`EntityWorldMut`].
    pub remove_id: RemoveFnId,
}

impl ReplicationRule {
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

    pub(super) fn is_subset(&self, other_rule: &ReplicationRule) -> bool {
        for (component_id, _) in &self.components {
            if other_rule
                .components
                .iter()
                .all(|(other_id, _)| component_id != other_id)
            {
                return true;
            }
        }

        false
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
    fn register(world: &mut World, replication_fns: &mut ReplicationFns) -> ReplicationRule {
        // Customize serlialization to serialize only `translation`.
        let transform_id = world.init_component::<Transform>();
        let transform_serde_id = replication_fns.register_serde_fns(SerdeFns {
            // For function definitions see the example from `AppReplicationExt::replicate_with`.
            serialize: serialize_translation,
            deserialize: deserialize_translation,
        });

        // Serialize `player` as usual.
        let visibility_id = world.init_component::<Player>();
        let visibility_serde_id = replication_fns.register_serde_fns(SerdeFns {
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
        let remove_id = replication_fns.register_remove_fn(replication_fns::remove::<(Transform, Player)>);

        ReplicationRule {
            components,
            remove_id,
        }
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
                    let serde_id = replication_fns.register_serde_fns(SerdeFns {
                        serialize: replication_fns::serialize::<$type>,
                        deserialize: replication_fns::deserialize::<$type>,
                    });
                    components.push((component_id, serde_id));
                )*
                let remove_id = replication_fns.register_remove_fn(replication_fns::remove::<($($type),*,)>);

                ReplicationRule {
                    components,
                    remove_id,
                }
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
