use std::{io::Cursor, marker::PhantomData};

use bevy::{
    ecs::{archetype::ArchetypeId, component::ComponentId, world::EntityMut},
    prelude::*,
    ptr::Ptr,
    scene::DynamicEntity,
    utils::HashMap,
};
use bevy_renet::renet::Bytes;
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::client::{ClientMapper, NetworkEntityMap};

pub trait AppReplicationExt {
    /// Marks component for replication.
    ///
    /// Component will be serialized as is using bincode.
    /// It also registers [`Ignored<T>`] that can be used to exclude the component from replication.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned;

    /// Same as [`Self::replicate`], but maps component entities using [`MapNetworkEntities`] trait.
    ///
    /// Always use it for components that contains entities.
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapNetworkEntities;

    /// Same as [`Self::replicate`], but uses the specified functions for serialization and deserialization.
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
    ) -> &mut Self
    where
        C: Component;
}

impl AppReplicationExt for App {
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.replicate_with::<C>(serialize_component::<C>, deserialize_component::<C>)
    }

    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapNetworkEntities,
    {
        self.replicate_with::<C>(serialize_component::<C>, deserialize_mapped_component::<C>)
    }

    fn replicate_with<C>(&mut self, serialize: SerializeFn, deserialize: DeserializeFn) -> &mut Self
    where
        C: Component,
    {
        let component_id = self.world.init_component::<C>();
        let ignored_id = self.world.init_component::<Ignored<C>>();
        let replicated_component = ReplicationInfo {
            ignored_id,
            serialize,
            deserialize,
            remove: remove_component::<C>,
        };

        let mut replication_rules = self.world.resource_mut::<ReplicationRules>();
        replication_rules.infos.push(replicated_component);

        let replication_id = ReplicationId(replication_rules.infos.len() - 1);
        replication_rules.ids.insert(component_id, replication_id);

        self
    }
}

/// Stores information about which components will be serialized and how.
#[derive(Resource)]
pub struct ReplicationRules {
    /// Maps component IDs to their replication IDs.
    ids: HashMap<ComponentId, ReplicationId>,

    /// Meta information about components that should be replicated.
    infos: Vec<ReplicationInfo>,

    /// ID of [`Replication`] component.
    marker_id: ComponentId,
}

impl ReplicationRules {
    /// Extracts all replicated entities and their components into `scene`.
    ///
    /// # Panics
    ///
    /// Panics if component is not registered using `register_type()`
    /// or missing `#[reflect(Component)]`.
    pub fn extract_entities(
        &self,
        scene: &mut DynamicScene,
        world: &World,
        registry: &AppTypeRegistry,
    ) {
        let registry = registry.read();
        for archetype in world
            .archetypes()
            .iter()
            .filter(|archetype| archetype.id() != ArchetypeId::EMPTY)
            .filter(|archetype| archetype.id() != ArchetypeId::INVALID)
            .filter(|archetype| archetype.contains(self.marker_id))
        {
            let entities_offset = scene.entities.len();
            for archetype_entity in archetype.entities() {
                scene.entities.push(DynamicEntity {
                    entity: archetype_entity.entity(),
                    components: Vec::new(),
                });
            }

            for component_id in archetype.components() {
                let Some((_, replication_info)) = self.get(component_id) else {
                    continue;
                };
                if archetype.contains(replication_info.ignored_id) {
                    continue;
                }

                // SAFETY: `component_info` obtained from the world.
                let component_info = unsafe { world.components().get_info_unchecked(component_id) };
                let type_name = component_info.name();
                let type_id = component_info
                    .type_id()
                    .unwrap_or_else(|| panic!("{type_name} should have registered TypeId"));
                let registration = registry
                    .get(type_id)
                    .unwrap_or_else(|| panic!("{type_name} should be registered"));
                let reflect_component = registration
                    .data::<ReflectComponent>()
                    .unwrap_or_else(|| panic!("{type_name} should have reflect(Component)"));

                for (index, archetype_entity) in archetype.entities().iter().enumerate() {
                    let component = reflect_component
                        .reflect(world.entity(archetype_entity.entity()))
                        .unwrap_or_else(|| panic!("entity should have {type_name}"));

                    scene.entities[entities_offset + index]
                        .components
                        .push(component.clone_value());
                }
            }
        }
    }

    /// ID of [`Replication`] component.
    pub(crate) fn get_marker_id(&self) -> ComponentId {
        self.marker_id
    }

    /// Returns mapping of replicated components to their replication IDs.
    pub(crate) fn get_ids(&self) -> &HashMap<ComponentId, ReplicationId> {
        &self.ids
    }

    /// Returns replication ID and meta information about component if its replicated.
    pub(crate) fn get(
        &self,
        component_id: ComponentId,
    ) -> Option<(ReplicationId, &ReplicationInfo)> {
        let replication_id = self.ids.get(&component_id).copied()?;
        // SAFETY: ID corresponds to a valid index because it obtained from `ids`.
        let replication_info = unsafe { self.infos.get_unchecked(replication_id.0) };

        Some((replication_id, replication_info))
    }

    /// Returns meta information about replicated component.
    ///
    /// # Safety
    ///
    /// `replication_id` should come from the same replication rules.
    pub(crate) unsafe fn get_info_unchecked(
        &self,
        replication_id: ReplicationId,
    ) -> &ReplicationInfo {
        self.infos.get_unchecked(replication_id.0)
    }
}

impl FromWorld for ReplicationRules {
    fn from_world(world: &mut World) -> Self {
        Self {
            infos: Default::default(),
            ids: Default::default(),
            marker_id: world.init_component::<Replication>(),
        }
    }
}

/// Signature of component serialization functions.
pub type SerializeFn = fn(Ptr, &mut Cursor<Vec<u8>>) -> Result<(), bincode::Error>;

/// Signature of component deserialization functions.
pub type DeserializeFn =
    fn(&mut EntityMut, &mut NetworkEntityMap, &mut Cursor<Bytes>) -> Result<(), bincode::Error>;

/// Stores meta information about replicated component.
pub(crate) struct ReplicationInfo {
    /// ID of [`Ignored<T>`] component.
    pub(crate) ignored_id: ComponentId,

    /// Function that serializes component into bytes.
    pub(crate) serialize: SerializeFn,

    /// Function that deserializes component from bytes and inserts it to [`EntityMut`].
    pub(crate) deserialize: DeserializeFn,

    /// Function that removes specific component from [`EntityMut`].
    pub(crate) remove: fn(&mut EntityMut),
}

/// Marks entity for replication.
#[derive(Component, Clone, Copy)]
pub struct Replication;

/// Replication will be ignored for `T` if this component is present on the same entity.
#[derive(Component)]
pub struct Ignored<T>(PhantomData<T>);

impl<T> Default for Ignored<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Same as [`ComponentId`], but consistent between server and clients.
///
/// Internally represents index of [`ReplicationInfo`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct ReplicationId(usize);

/// Maps entities inside component.
///
/// The same as [`bevy::ecs::entity::MapEntities`], but never creates new entities on mapping error.
pub trait MapNetworkEntities {
    /// Maps stored entities using specified map.
    fn map_entities<T: Mapper>(&mut self, mapper: &mut T);
}

pub trait Mapper {
    fn map(&mut self, entity: Entity) -> Entity;
}

/// Default serialization function.
fn serialize_component<C: Component + Serialize>(
    component: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> Result<(), bincode::Error> {
    // SAFETY: Function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default deserialization function.
fn deserialize_component<C: Component + DeserializeOwned>(
    entity: &mut EntityMut,
    _entity_map: &mut NetworkEntityMap,
    cursor: &mut Cursor<Bytes>,
) -> Result<(), bincode::Error> {
    let component: C = DefaultOptions::new().deserialize_from(cursor)?;
    entity.insert(component);

    Ok(())
}

/// Like [`deserialize_component`], but also maps entities before insertion.
fn deserialize_mapped_component<C: Component + DeserializeOwned + MapNetworkEntities>(
    entity: &mut EntityMut,
    entity_map: &mut NetworkEntityMap,
    cursor: &mut Cursor<Bytes>,
) -> Result<(), bincode::Error> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;

    entity.world_scope(|world| {
        component.map_entities(&mut ClientMapper::new(world, entity_map));
    });

    entity.insert(component);

    Ok(())
}

/// Removes specified component from entity.
fn remove_component<C: Component>(entity: &mut EntityMut) {
    entity.remove::<C>();
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[test]
    fn scene_creation() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .register_type::<DummyComponent>()
            .replicate::<DummyComponent>();

        app.world.spawn(DummyComponent);
        let dummy_entity = app.world.spawn((Replication, DummyComponent)).id();
        let empty_entity = app
            .world
            .spawn((
                Replication,
                DummyComponent,
                Ignored::<DummyComponent>::default(),
            ))
            .id();

        let registry = app.world.resource::<AppTypeRegistry>();
        let mut scene = DynamicScene::default();
        app.world
            .resource::<ReplicationRules>()
            .extract_entities(&mut scene, &app.world, registry);

        assert!(scene.resources.is_empty());

        let [dummy, empty] = &scene.entities[..] else {
            panic!("scene should only contain entities marked for replication");
        };

        assert_eq!(dummy.entity, dummy_entity);
        assert_eq!(dummy.components.len(), 1);

        assert_eq!(empty.entity, empty_entity);
        assert!(empty.components.is_empty());
    }

    #[derive(Component, Default, Deserialize, Reflect, Serialize)]
    #[reflect(Component)]
    struct DummyComponent;
}
