use std::{
    any,
    fmt::{self, Formatter},
};

use bevy::{
    ecs::{component::Tick, world::EntityMut},
    prelude::*,
    ptr::Ptr,
    reflect::erased_serde,
    utils::HashMap,
};
use derive_more::Constructor;
use serde::{
    de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
    ser::{SerializeMap, SerializeSeq, SerializeStruct, SerializeTuple},
    Deserialize, Deserializer, Serialize, Serializer,
};
use strum::{EnumDiscriminants, EnumVariantNames, IntoStaticStr, VariantNames};

use super::{ReplicationId, ReplicationInfo, ReplicationRules};
use crate::{client::LastTick, prelude::NetworkEntityMap};

/// Changed world data and current tick from server.
///
/// Sent from server to clients.
pub(crate) struct WorldDiff<'a> {
    pub(crate) tick: Tick,
    pub(crate) entities: HashMap<Entity, Vec<ComponentDiff<'a>>>,
    pub(crate) despawns: Vec<Entity>,
}

impl WorldDiff<'_> {
    /// Creates a new [`WorldDiff`] with a tick and empty entities.
    pub(crate) fn new(tick: Tick) -> Self {
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

/// Type of component change.
#[derive(EnumDiscriminants)]
#[strum_discriminants(
    name(ComponentDiffField),
    derive(Deserialize, EnumVariantNames, IntoStaticStr),
    strum(serialize_all = "snake_case")
)]
pub(crate) enum ComponentDiff<'a> {
    /// Indicates that a component was added or changed, contains its ID and pointer.
    Changed((ReplicationId, Ptr<'a>)),
    /// Indicates that a component was removed, contains its ID.
    Removed(ReplicationId),
}

#[derive(Constructor)]
pub(crate) struct WorldDiffSerializer<'a> {
    world_diff: &'a WorldDiff<'a>,
    replication_rules: &'a ReplicationRules,
}

impl Serialize for WorldDiffSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct(
            any::type_name::<WorldDiff>(),
            WorldDiffField::VARIANTS.len(),
        )?;
        state.serialize_field(WorldDiffField::Tick.into(), &self.world_diff.tick.get())?;
        state.serialize_field(
            WorldDiffField::Entities.into(),
            &EntitiesSerializer::new(&self.world_diff.entities, self.replication_rules),
        )?;
        state.serialize_field(WorldDiffField::Despawned.into(), &self.world_diff.despawns)?;
        state.end()
    }
}

#[derive(Constructor)]
struct EntitiesSerializer<'a> {
    entities: &'a HashMap<Entity, Vec<ComponentDiff<'a>>>,
    replication_rules: &'a ReplicationRules,
}

impl Serialize for EntitiesSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entities.len()))?;
        for (entity, components) in self.entities {
            map.serialize_entry(
                entity,
                &ComponentsSerializer::new(components, self.replication_rules),
            )?;
        }
        map.end()
    }
}

#[derive(Constructor)]
struct ComponentsSerializer<'a> {
    components: &'a [ComponentDiff<'a>],
    replication_rules: &'a ReplicationRules,
}

impl Serialize for ComponentsSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.components.len()))?;
        for component_diff in self.components {
            seq.serialize_element(&ComponentDiffSerializer::new(
                component_diff,
                self.replication_rules,
            ))?;
        }
        seq.end()
    }
}

#[derive(Constructor)]
struct ComponentDiffSerializer<'a> {
    component_diff: &'a ComponentDiff<'a>,
    replication_rules: &'a ReplicationRules,
}

impl Serialize for ComponentDiffSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self.component_diff {
            ComponentDiff::Changed((replication_id, ptr)) => serializer.serialize_newtype_variant(
                any::type_name::<ComponentDiff>(),
                ComponentDiffField::Changed as u32,
                ComponentDiffField::Changed.into(),
                &ComponentChangeSerializer::new(replication_id, ptr, self.replication_rules),
            ),
            ComponentDiff::Removed(replication_id) => serializer.serialize_newtype_variant(
                any::type_name::<ComponentDiff>(),
                ComponentDiffField::Removed as u32,
                ComponentDiffField::Removed.into(),
                &replication_id,
            ),
        }
    }
}

#[derive(Constructor)]
struct ComponentChangeSerializer<'a> {
    replication_id: ReplicationId,
    ptr: Ptr<'a>,
    replication_rules: &'a ReplicationRules,
}

impl Serialize for ComponentChangeSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.replication_id)?;

        let replication_info = self.replication_rules.get_info(self.replication_id);
        let component = (replication_info.serialize)(self.ptr);
        tuple.serialize_element(&component)?;

        tuple.end()
    }
}

#[derive(Constructor)]
pub(crate) struct WorldDiffDeserializer<'a> {
    world: &'a mut World,
    replication_rules: &'a ReplicationRules,
    entity_map: &'a mut NetworkEntityMap,
}

impl<'de> DeserializeSeed<'de> for WorldDiffDeserializer<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(
            any::type_name::<Self::Value>(),
            WorldDiffField::VARIANTS,
            self,
        )
    }
}

impl<'de> Visitor<'de> for WorldDiffDeserializer<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let tick: u32 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(WorldDiffField::Tick as usize, &self))?;
        *self.world.resource_mut::<LastTick>() = Tick::new(tick).into();

        seq.next_element_seed(EntitiesDeserializer::new(
            self.world,
            self.replication_rules,
            self.entity_map,
        ))?
        .ok_or_else(|| de::Error::invalid_length(WorldDiffField::Entities as usize, &self))?;

        let despawns: Vec<Entity> = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(WorldDiffField::Despawned as usize, &self))?;
        for server_entity in despawns {
            // The entity might have already been deleted with the last diff,
            // but the server might not yet have received confirmation from the
            // client and could include the deletion in the latest diff.
            if let Some(client_entity) = self.entity_map.remove_by_server(server_entity) {
                self.world.entity_mut(client_entity).despawn_recursive();
            }
        }

        Ok(())
    }
}

#[derive(Constructor)]
struct EntitiesDeserializer<'a> {
    world: &'a mut World,
    replication_rules: &'a ReplicationRules,
    entity_map: &'a mut NetworkEntityMap,
}

impl<'de> DeserializeSeed<'de> for EntitiesDeserializer<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for EntitiesDeserializer<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while let Some(entity) = map.next_key()? {
            let mut entity = self.entity_map.get_by_server_or_spawn(self.world, entity);
            map.next_value_seed(ComponentsDeserializer::new(
                &mut entity,
                self.replication_rules,
                self.entity_map,
            ))?;
        }

        Ok(())
    }
}

#[derive(Constructor)]
struct ComponentsDeserializer<'a> {
    entity: &'a mut EntityMut<'a>,
    replication_rules: &'a ReplicationRules,
    entity_map: &'a mut NetworkEntityMap,
}

impl<'de> DeserializeSeed<'de> for ComponentsDeserializer<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for ComponentsDeserializer<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<Self::Value, A::Error> {
        loop {
            let deserializer = ComponentDiffDeserializer::new(
                self.entity,
                self.replication_rules,
                self.entity_map,
            );
            if let Some((entity, entity_map)) = seq.next_element_seed(deserializer)? {
                // Mutable references consumed when passed to a struct, so we reborrow them again by returning them from deserializer.
                self.entity = entity;
                self.entity_map = entity_map;
            } else {
                break;
            }
        }

        Ok(())
    }
}

#[derive(Constructor)]
struct ComponentDiffDeserializer<'a> {
    entity: &'a mut EntityMut<'a>,
    replication_rules: &'a ReplicationRules,
    entity_map: &'a mut NetworkEntityMap,
}

impl<'a, 'de> DeserializeSeed<'de> for ComponentDiffDeserializer<'a> {
    type Value = (&'a mut EntityMut<'a>, &'a mut NetworkEntityMap);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_enum(
            any::type_name::<ComponentDiff>(),
            ComponentDiffField::VARIANTS,
            self,
        )
    }
}

impl<'a, 'de> Visitor<'de> for ComponentDiffDeserializer<'a> {
    type Value = (&'a mut EntityMut<'a>, &'a mut NetworkEntityMap);

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (field, variant) = data.variant::<ComponentDiffField>()?;
        let (entity, entity_map) = match field {
            ComponentDiffField::Changed => {
                variant.newtype_variant_seed(ComponentChangeDeserializer::new(
                    self.entity,
                    self.replication_rules,
                    self.entity_map,
                ))?
            }
            ComponentDiffField::Removed => {
                let entity = variant.newtype_variant_seed(ComponentRemoveDeserializer::new(
                    self.entity,
                    self.replication_rules,
                ))?;
                (entity, self.entity_map)
            }
        };

        Ok((entity, entity_map))
    }
}

#[derive(Constructor)]
struct ComponentChangeDeserializer<'a> {
    entity: &'a mut EntityMut<'a>,
    replication_rules: &'a ReplicationRules,
    entity_map: &'a mut NetworkEntityMap,
}

impl<'a, 'de> DeserializeSeed<'de> for ComponentChangeDeserializer<'a> {
    type Value = (&'a mut EntityMut<'a>, &'a mut NetworkEntityMap);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_tuple(2, self)
    }
}

impl<'a, 'de> Visitor<'de> for ComponentChangeDeserializer<'a> {
    type Value = (&'a mut EntityMut<'a>, &'a mut NetworkEntityMap);

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let replication_id = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        let replication_info = self.replication_rules.get_info(replication_id);
        let (entity, entity_map) = seq
            .next_element_seed(PtrDeserializer::new(
                self.entity,
                replication_info,
                self.entity_map,
            ))?
            .unwrap();
        // TODO: can't pass self due to borrowing issues
        // .ok_or_else(|| de::Error::invalid_length(1, &self))?;

        Ok((entity, entity_map))
    }
}

#[derive(Constructor)]
struct PtrDeserializer<'a> {
    entity: &'a mut EntityMut<'a>,
    replciation_info: &'a ReplicationInfo,
    entity_map: &'a mut NetworkEntityMap,
}

impl<'a, 'de> DeserializeSeed<'de> for PtrDeserializer<'a> {
    type Value = (&'a mut EntityMut<'a>, &'a mut NetworkEntityMap);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        let deserializer = &mut <dyn erased_serde::Deserializer>::erase(deserializer);
        (self.replciation_info.deserialize)(self.entity, self.entity_map, deserializer)
            .map_err(de::Error::custom)?;

        Ok((self.entity, self.entity_map))
    }
}

#[derive(Constructor)]
struct ComponentRemoveDeserializer<'a> {
    entity: &'a mut EntityMut<'a>,
    replication_rules: &'a ReplicationRules,
}

impl<'a, 'de> DeserializeSeed<'de> for ComponentRemoveDeserializer<'a> {
    type Value = &'a mut EntityMut<'a>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        let replication_id = ReplicationId::deserialize(deserializer)?;
        let replication_info = self.replication_rules.get_info(replication_id);
        (replication_info.remove)(self.entity);

        Ok(self.entity)
    }
}
