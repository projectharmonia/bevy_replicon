use std::{
    any,
    fmt::{self, Formatter},
};

use bevy::{
    prelude::*,
    reflect::{
        serde::{ReflectSerializer, UntypedReflectDeserializer},
        TypeRegistryInternal,
    },
    utils::HashMap,
};
use derive_more::Constructor;
use serde::{
    de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
    ser::{SerializeMap, SerializeSeq, SerializeStruct},
    Deserialize, Deserializer, Serialize, Serializer,
};
use strum::{EnumDiscriminants, EnumVariantNames, IntoStaticStr, VariantNames};

use crate::tick::Tick;

/// Changed world data and current tick from server.
///
/// Sent from server to clients.
pub(super) struct WorldDiff {
    pub(super) tick: Tick,
    pub(super) entities: HashMap<Entity, Vec<ComponentDiff>>,
    pub(super) despawns: Vec<Entity>,
}

impl WorldDiff {
    /// Creates a new [`WorldDiff`] with a tick and empty entities.
    pub(super) fn new(tick: Tick) -> Self {
        Self {
            tick,
            entities: Default::default(),
            despawns: Default::default(),
        }
    }
}

/// Fields of [`WorldDiff`] for manual deserialization.
#[derive(IntoStaticStr, EnumVariantNames)]
#[strum(serialize_all = "snake_case")]
enum WorldDiffField {
    Tick,
    Entities,
    Despawned,
}

/// Type of component or resource change.
#[derive(EnumDiscriminants)]
#[strum_discriminants(
    name(ComponentDiffField),
    derive(Deserialize, EnumVariantNames, IntoStaticStr),
    strum(serialize_all = "snake_case")
)]
pub(super) enum ComponentDiff {
    /// Indicates that a component was added or changed, contains serialized [`Reflect`].
    Changed(Box<dyn Reflect>),
    /// Indicates that a component was removed, contains component name.
    Removed(String),
}

impl ComponentDiff {
    /// Returns changed component type name.
    pub(super) fn type_name(&self) -> &str {
        match self {
            ComponentDiff::Changed(component) => component.type_name(),
            ComponentDiff::Removed(type_name) => type_name,
        }
    }
}

#[derive(Constructor)]
pub(super) struct WorldDiffSerializer<'a> {
    world_diff: &'a WorldDiff,
    registry: &'a TypeRegistryInternal,
}

impl Serialize for WorldDiffSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct(
            any::type_name::<WorldDiff>(),
            WorldDiffField::VARIANTS.len(),
        )?;
        state.serialize_field(WorldDiffField::Tick.into(), &self.world_diff.tick)?;
        state.serialize_field(
            WorldDiffField::Entities.into(),
            &EntitiesSerializer::new(&self.world_diff.entities, self.registry),
        )?;
        state.serialize_field(WorldDiffField::Despawned.into(), &self.world_diff.despawns)?;
        state.end()
    }
}

#[derive(Constructor)]
struct EntitiesSerializer<'a> {
    entities: &'a HashMap<Entity, Vec<ComponentDiff>>,
    registry: &'a TypeRegistryInternal,
}

impl Serialize for EntitiesSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entities.len()))?;
        for (entity, components) in self.entities {
            map.serialize_entry(
                entity,
                &ComponentsSerializer::new(components, self.registry),
            )?;
        }
        map.end()
    }
}

#[derive(Constructor)]
struct ComponentsSerializer<'a> {
    components: &'a [ComponentDiff],
    registry: &'a TypeRegistryInternal,
}

impl Serialize for ComponentsSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.components.len()))?;
        for component_diff in self.components {
            seq.serialize_element(&ComponentDiffSerializer::new(component_diff, self.registry))?;
        }
        seq.end()
    }
}

#[derive(Constructor)]
struct ComponentDiffSerializer<'a> {
    component_diff: &'a ComponentDiff,
    registry: &'a TypeRegistryInternal,
}

impl Serialize for ComponentDiffSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.component_diff {
            ComponentDiff::Changed(component) => serializer.serialize_newtype_variant(
                any::type_name::<ComponentDiff>(),
                ComponentDiffField::Changed as u32,
                ComponentDiffField::Changed.into(),
                &ReflectSerializer::new(&**component, self.registry),
            ),
            ComponentDiff::Removed(type_name) => serializer.serialize_newtype_variant(
                any::type_name::<ComponentDiff>(),
                ComponentDiffField::Removed as u32,
                ComponentDiffField::Removed.into(),
                type_name,
            ),
        }
    }
}

#[derive(Constructor)]
pub(super) struct WorldDiffDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl<'de> DeserializeSeed<'de> for WorldDiffDeserializer<'_> {
    type Value = WorldDiff;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(
            any::type_name::<Self::Value>(),
            WorldDiffField::VARIANTS,
            self,
        )
    }
}

impl<'de> Visitor<'de> for WorldDiffDeserializer<'_> {
    type Value = WorldDiff;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let tick = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(WorldDiffField::Tick as usize, &self))?;
        let entities = seq
            .next_element_seed(EntitiesDeserializer::new(self.registry))?
            .ok_or_else(|| de::Error::invalid_length(WorldDiffField::Entities as usize, &self))?;
        let despawns = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(WorldDiffField::Despawned as usize, &self))?;
        Ok(WorldDiff {
            tick,
            entities,
            despawns,
        })
    }
}

#[derive(Constructor)]
struct EntitiesDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl<'de> DeserializeSeed<'de> for EntitiesDeserializer<'_> {
    type Value = HashMap<Entity, Vec<ComponentDiff>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for EntitiesDeserializer<'_> {
    type Value = HashMap<Entity, Vec<ComponentDiff>>;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entities = HashMap::with_capacity(map.size_hint().unwrap_or_default());
        while let Some(key) = map.next_key()? {
            let value = map.next_value_seed(ComponentsDeserializer::new(self.registry))?;
            entities.insert(key, value);
        }

        Ok(entities)
    }
}

#[derive(Constructor)]
struct ComponentsDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl<'de> DeserializeSeed<'de> for ComponentsDeserializer<'_> {
    type Value = Vec<ComponentDiff>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for ComponentsDeserializer<'_> {
    type Value = Vec<ComponentDiff>;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut components = Vec::with_capacity(seq.size_hint().unwrap_or_default());
        while let Some(component_diff) =
            seq.next_element_seed(ComponentDiffDeserializer::new(self.registry))?
        {
            components.push(component_diff);
        }

        Ok(components)
    }
}

#[derive(Constructor)]
struct ComponentDiffDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl<'de> DeserializeSeed<'de> for ComponentDiffDeserializer<'_> {
    type Value = ComponentDiff;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_enum(
            any::type_name::<ComponentDiff>(),
            ComponentDiffField::VARIANTS,
            self,
        )
    }
}

impl<'de> Visitor<'de> for ComponentDiffDeserializer<'_> {
    type Value = ComponentDiff;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (field, variant) = data.variant::<ComponentDiffField>()?;
        let component_diff = match field {
            ComponentDiffField::Changed => ComponentDiff::Changed(
                variant.newtype_variant_seed(UntypedReflectDeserializer::new(self.registry))?,
            ),
            ComponentDiffField::Removed => ComponentDiff::Removed(variant.newtype_variant()?),
        };

        Ok(component_diff)
    }
}

#[cfg(test)]
mod tests {
    use serde_test::Token;

    use super::*;

    const COMPONENT_NAME: &str = "My component";

    #[derive(Component, Reflect, Default)]
    #[reflect(Component)]
    struct DummyComponent;

    #[test]
    fn component_diff_removed_ser() {
        let registry = TypeRegistryInternal::new();
        let component_diff = ComponentDiff::Removed(COMPONENT_NAME.to_string());
        let serializer = ComponentDiffSerializer::new(&component_diff, &registry);

        serde_test::assert_ser_tokens(
            &serializer,
            &[
                Token::NewtypeVariant {
                    name: any::type_name::<ComponentDiff>(),
                    variant: ComponentDiffField::Removed.into(),
                },
                Token::Str(COMPONENT_NAME),
            ],
        );
    }

    #[test]
    fn component_diff_changed_ser() {
        let mut registry = TypeRegistryInternal::new();
        registry.register::<DummyComponent>();
        let component_diff = ComponentDiff::Changed(DummyComponent.clone_value());
        let serializer = ComponentDiffSerializer::new(&component_diff, &registry);

        serde_test::assert_ser_tokens(
            &serializer,
            &[
                Token::NewtypeVariant {
                    name: any::type_name::<ComponentDiff>(),
                    variant: ComponentDiffField::Changed.into(),
                },
                Token::Map { len: Some(1) },
                Token::Str(any::type_name::<DummyComponent>()),
                Token::Struct {
                    name: "DummyComponent",
                    len: 0,
                },
                Token::StructEnd,
                Token::MapEnd,
            ],
        );
    }

    #[test]
    fn world_diff_ser() {
        let registry = TypeRegistryInternal::default();
        let world_diff = WorldDiff {
            tick: Tick::new(0),
            entities: HashMap::from([(
                Entity::PLACEHOLDER,
                Vec::from([ComponentDiff::Removed(COMPONENT_NAME.to_string())]),
            )]),
            despawns: Vec::from([Entity::PLACEHOLDER]),
        };
        let serializer = WorldDiffSerializer::new(&world_diff, &registry);

        serde_test::assert_ser_tokens(
            &serializer,
            &[
                Token::Struct {
                    name: any::type_name::<WorldDiff>(),
                    len: WorldDiffField::VARIANTS.len(),
                },
                Token::Str(WorldDiffField::Tick.into()),
                Token::Struct {
                    name: "Tick",
                    len: 1,
                },
                Token::Str("tick"),
                Token::U32(world_diff.tick.get()),
                Token::StructEnd,
                Token::Str(WorldDiffField::Entities.into()),
                Token::Map { len: Some(1) },
                Token::Struct {
                    name: "Entity",
                    len: 2,
                },
                Token::Str("generation"),
                Token::U32(Entity::PLACEHOLDER.generation()),
                Token::Str("index"),
                Token::U32(Entity::PLACEHOLDER.index()),
                Token::StructEnd,
                Token::Seq { len: Some(1) },
                Token::NewtypeVariant {
                    name: any::type_name::<ComponentDiff>(),
                    variant: ComponentDiffField::Removed.into(),
                },
                Token::Str(COMPONENT_NAME),
                Token::SeqEnd,
                Token::MapEnd,
                Token::Str(WorldDiffField::Despawned.into()),
                Token::Seq { len: Some(1) },
                Token::Struct {
                    name: "Entity",
                    len: 2,
                },
                Token::Str("generation"),
                Token::U32(Entity::PLACEHOLDER.generation()),
                Token::Str("index"),
                Token::U32(Entity::PLACEHOLDER.index()),
                Token::StructEnd,
                Token::SeqEnd,
                Token::StructEnd,
            ],
        );
    }
}
