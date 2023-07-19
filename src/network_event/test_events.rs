//! Network event implementations for unit tests

use std::{
    any,
    fmt::{self, Formatter},
};

use bevy::{
    ecs::entity::EntityMap,
    prelude::*,
    reflect::{
        serde::{ReflectSerializer, UntypedReflectDeserializer},
        TypeRegistryInternal,
    },
};
use serde::{
    de::{self, DeserializeSeed, SeqAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};
use strum::{EnumVariantNames, IntoStaticStr, VariantNames};

use super::{BuildEventDeserializer, BuildEventSerializer, MapError, MapEventEntities};

#[derive(Reflect, Debug)]
pub(super) struct DummyComponent;

#[derive(Debug, Deserialize, Event, Serialize)]
pub(super) struct DummyEvent(pub(super) Entity);

impl MapEventEntities for DummyEvent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapError> {
        self.0 = entity_map.get(self.0).ok_or(MapError(self.0))?;
        Ok(())
    }
}

#[derive(Debug, Event)]
pub(super) struct ReflectEvent {
    pub(super) entity: Entity,
    pub(super) component: Box<dyn Reflect>,
}

impl MapEventEntities for ReflectEvent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapError> {
        self.entity = entity_map.get(self.entity).ok_or(MapError(self.entity))?;
        Ok(())
    }
}

#[derive(IntoStaticStr, EnumVariantNames)]
#[strum(serialize_all = "snake_case")]
enum ReflectEventField {
    Entity,
    Component,
}

pub(super) struct ReflectEventSerializer<'a> {
    registry: &'a TypeRegistryInternal,
    event: &'a ReflectEvent,
}

impl BuildEventSerializer<ReflectEvent> for ReflectEventSerializer<'_> {
    type EventSerializer<'a> = ReflectEventSerializer<'a>;

    fn new<'a>(
        event: &'a ReflectEvent,
        registry: &'a TypeRegistryInternal,
    ) -> Self::EventSerializer<'a> {
        Self::EventSerializer { event, registry }
    }
}

impl Serialize for ReflectEventSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct(
            any::type_name::<ReflectEvent>(),
            ReflectEventField::VARIANTS.len(),
        )?;
        state.serialize_field(ReflectEventField::Entity.into(), &self.event.entity)?;
        state.serialize_field(
            ReflectEventField::Entity.into(),
            &ReflectSerializer::new(&*self.event.component, self.registry),
        )?;
        state.end()
    }
}

pub(super) struct ReflectEventDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl BuildEventDeserializer for ReflectEventDeserializer<'_> {
    type EventDeserializer<'a> = ReflectEventDeserializer<'a>;

    fn new(registry: &TypeRegistryInternal) -> Self::EventDeserializer<'_> {
        Self::EventDeserializer { registry }
    }
}

impl<'de> DeserializeSeed<'de> for ReflectEventDeserializer<'_> {
    type Value = ReflectEvent;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(
            any::type_name::<Self::Value>(),
            ReflectEventField::VARIANTS,
            self,
        )
    }
}

impl<'de> Visitor<'de> for ReflectEventDeserializer<'_> {
    type Value = ReflectEvent;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let entity = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(ReflectEventField::Entity as usize, &self))?;
        let component = seq
            .next_element_seed(UntypedReflectDeserializer::new(self.registry))?
            .ok_or_else(|| {
                de::Error::invalid_length(ReflectEventField::Component as usize, &self)
            })?;
        Ok(ReflectEvent { entity, component })
    }
}
