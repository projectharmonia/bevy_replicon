use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{ecs::entity::MapEntities, prelude::*};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::ctx::{SerializeCtx, WriteCtx};

/// Type-erased version of [`RuleFns`].
///
/// Stored inside [`ReplicationFns`](super::ReplicationFns) after registration.
pub(crate) struct UntypedRuleFns {
    type_id: TypeId,
    type_name: &'static str,

    serialize: unsafe fn(),
    deserialize: unsafe fn(),
    deserialize_in_place: unsafe fn(),
    consume: unsafe fn(),
}

impl UntypedRuleFns {
    /// Restores the original [`RuleFns`] from which this type was created.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which this instance was created.
    pub(super) unsafe fn typed<C: Component>(&self) -> RuleFns<C> {
        debug_assert_eq!(
            self.type_id,
            TypeId::of::<C>(),
            "trying to call rule functions with {}, but they were created with {}",
            any::type_name::<C>(),
            self.type_name,
        );

        RuleFns {
            serialize: unsafe { mem::transmute(self.serialize) },
            deserialize: unsafe { mem::transmute(self.deserialize) },
            deserialize_in_place: unsafe { mem::transmute(self.deserialize_in_place) },
            consume: unsafe { mem::transmute(self.consume) },
        }
    }
}

impl<C: Component> From<RuleFns<C>> for UntypedRuleFns {
    fn from(value: RuleFns<C>) -> Self {
        // SAFETY: these functions won't be called until the type is restored.
        Self {
            type_id: TypeId::of::<C>(),
            type_name: any::type_name::<C>(),
            serialize: unsafe { mem::transmute(value.serialize) },
            deserialize: unsafe { mem::transmute(value.deserialize) },
            deserialize_in_place: unsafe { mem::transmute(value.deserialize_in_place) },
            consume: unsafe { mem::transmute(value.consume) },
        }
    }
}

/// Serialization and deserialization functions for a component.
///
/// See also [`AppRuleExt`](crate::core::replication_rules::AppRuleExt)
/// and [`ReplicationRule`](crate::core::replication_rules::ReplicationRule).
pub struct RuleFns<C> {
    serialize: SerializeFn<C>,
    deserialize: DeserializeFn<C>,
    deserialize_in_place: DeserializeInPlaceFn<C>,
    consume: ConsumeFn<C>,
}

impl<C: Component> RuleFns<C> {
    /// Creates a new instance.
    ///
    /// See also [`Self::with_in_place`] and [`Self::with_consume`].
    pub fn new(serialize: SerializeFn<C>, deserialize: DeserializeFn<C>) -> Self {
        Self {
            serialize,
            deserialize,
            deserialize_in_place: in_place_as_deserialize::<C>,
            consume: consume_as_deserialize,
        }
    }

    /// Replaces default [`in_place_as_deserialize`] with a custom function.
    ///
    /// This function will be called when a component is already present on an entity.
    /// For insertion [`Self::deserialize`] will be called instead.
    pub fn with_in_place(mut self, deserialize_in_place: DeserializeInPlaceFn<C>) -> Self {
        self.deserialize_in_place = deserialize_in_place;
        self
    }

    /// Replaces the default [`consume_as_deserialize`] with a custom function.
    ///
    /// This function will be called if an entity has at least one marker
    /// that requires history for components that don't belong to that markers.
    ///
    /// If no markers require history, old entity updates will be skipped entirely
    /// by just advancing the cursor (without calling any consume functions).
    ///
    /// See [`MarkerConfig::need_history`](crate::core::command_markers::MarkerConfig::need_history)
    /// for details.
    pub fn with_consume(mut self, consume: ConsumeFn<C>) -> Self {
        self.consume = consume;
        self
    }

    /// Serializes a component into a cursor.
    pub(super) fn serialize(
        &self,
        ctx: &SerializeCtx,
        component: &C,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        (self.serialize)(ctx, component, cursor)
    }

    /// Deserializes a component from a cursor.
    ///
    /// Use this function when inserting a new component.
    pub fn deserialize(
        &self,
        ctx: &mut WriteCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<C> {
        (self.deserialize)(ctx, cursor)
    }

    /// Same as [`Self::deserialize`], but instead of returning a component, it updates the passed reference.
    ///
    /// Use this function for updating an existing component.
    pub fn deserialize_in_place(
        &self,
        ctx: &mut WriteCtx,
        component: &mut C,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<()> {
        (self.deserialize_in_place)(self.deserialize, ctx, component, cursor)
    }

    /// Consumes a component from a cursor.
    pub(super) fn consume(
        &self,
        ctx: &mut WriteCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<()> {
        (self.consume)(self.deserialize, ctx, cursor)
    }
}

impl<C: Component + Serialize + DeserializeOwned + MapEntities> RuleFns<C> {
    /// Like [`Self::default`], but uses a special deserialization function to map server
    /// entities inside the component into client entities.
    ///
    /// Always use it for components that contain entities.
    ///
    /// See also [`default_serialize`], [`default_deserialize_mapped`] and [`in_place_as_deserialize`].
    pub fn default_mapped() -> Self {
        Self::new(default_serialize::<C>, default_deserialize_mapped::<C>)
    }
}

impl<C: Component + Serialize + DeserializeOwned> Default for RuleFns<C> {
    /// Creates a new instance with default functions for a component.
    ///
    /// If your component contains any [`Entity`] inside, use [`Self::default_mapped`].
    ///
    /// See also [`default_serialize`], [`default_deserialize`] and [`in_place_as_deserialize`].
    fn default() -> Self {
        Self::new(default_serialize::<C>, default_deserialize::<C>)
    }
}

/// Signature of component serialization functions.
pub type SerializeFn<C> = fn(&SerializeCtx, &C, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component deserialization functions.
pub type DeserializeFn<C> = fn(&mut WriteCtx, &mut Cursor<&[u8]>) -> bincode::Result<C>;

/// Signature of component in-place deserialization functions.
pub type DeserializeInPlaceFn<C> =
    fn(DeserializeFn<C>, &mut WriteCtx, &mut C, &mut Cursor<&[u8]>) -> bincode::Result<()>;

/// Signature of component consume functions.
pub type ConsumeFn<C> =
    fn(DeserializeFn<C>, &mut WriteCtx, &mut Cursor<&[u8]>) -> bincode::Result<()>;

/// Default component serialization function.
pub fn default_serialize<C: Component + Serialize>(
    _ctx: &SerializeCtx,
    component: &C,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default component deserialization function.
pub fn default_deserialize<C: Component + DeserializeOwned>(
    _ctx: &mut WriteCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<C> {
    DefaultOptions::new().deserialize_from(cursor)
}

/// Like [`default_deserialize`], but also maps entities before insertion.
pub fn default_deserialize_mapped<C: Component + DeserializeOwned + MapEntities>(
    ctx: &mut WriteCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<C> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;
    component.map_entities(ctx);
    Ok(component)
}

/// Default component in-place deserialization function.
///
/// This implementation just assigns the value from the passed deserialization function.
pub fn in_place_as_deserialize<C: Component>(
    deserialize: DeserializeFn<C>,
    ctx: &mut WriteCtx,
    component: &mut C,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    *component = (deserialize)(ctx, cursor)?;
    Ok(())
}

/// Default component consume function.
///
/// This implementation just calls deserialization function and ignores its result.
pub fn consume_as_deserialize<C: Component>(
    deserialize: DeserializeFn<C>,
    ctx: &mut WriteCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    ctx.ignore_mapping = true;
    (deserialize)(ctx, cursor)?;
    ctx.ignore_mapping = false;
    Ok(())
}
