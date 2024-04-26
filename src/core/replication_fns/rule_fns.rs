use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{ecs::entity::MapEntities, prelude::*};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::ctx::{SerializeCtx, WriteDeserializeCtx};

/// Type-erased version of [`RuleFns`].
///
/// Stored inside [`ReplicationFns`](super::ReplicationFns) after registration.
pub(crate) struct UntypedRuleFns {
    type_id: TypeId,
    type_name: &'static str,

    serialize: unsafe fn(),
    deserialize: unsafe fn(),
    deserialize_in_place: unsafe fn(),
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
}

impl<C: Component> RuleFns<C> {
    /// Creates a new instance.
    ///
    /// You can also provide a custom behavior for deserialization in place, see [`Self::with_in_place`].
    pub fn new(serialize: SerializeFn<C>, deserialize: DeserializeFn<C>) -> Self {
        Self {
            serialize,
            deserialize,
            deserialize_in_place: in_place_as_deserialize::<C>,
        }
    }

    /// Replaces default [`in_place_as_deserialize`] with a custom function.
    pub fn with_in_place(mut self, deserialize_in_place: DeserializeInPlaceFn<C>) -> Self {
        self.deserialize_in_place = deserialize_in_place;
        self
    }

    /// Serializes a component into a cursor.
    pub fn serialize(
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
        ctx: &mut WriteDeserializeCtx,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<C> {
        (self.deserialize)(ctx, cursor)
    }

    /// Same as [`Self::deserialize`], but instead of returning a component, it updates the passed reference.
    ///
    /// Use this function for updating an existing component.
    pub fn deserialize_in_place(
        &self,
        ctx: &mut WriteDeserializeCtx,
        component: &mut C,
        cursor: &mut Cursor<&[u8]>,
    ) -> bincode::Result<()> {
        (self.deserialize_in_place)(self.deserialize, ctx, component, cursor)
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
pub type DeserializeFn<C> = fn(&mut WriteDeserializeCtx, &mut Cursor<&[u8]>) -> bincode::Result<C>;

/// Signature of in-place component deserialization functions.
pub type DeserializeInPlaceFn<C> = fn(
    DeserializeFn<C>,
    &mut WriteDeserializeCtx,
    &mut C,
    &mut Cursor<&[u8]>,
) -> bincode::Result<()>;

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
    _ctx: &mut WriteDeserializeCtx,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<C> {
    DefaultOptions::new().deserialize_from(cursor)
}

/// Like [`default_deserialize`], but also maps entities before insertion.
pub fn default_deserialize_mapped<C: Component + DeserializeOwned + MapEntities>(
    ctx: &mut WriteDeserializeCtx,
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
    ctx: &mut WriteDeserializeCtx,
    component: &mut C,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    *component = (deserialize)(ctx, cursor)?;
    Ok(())
}
