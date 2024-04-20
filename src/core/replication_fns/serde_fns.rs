use std::{
    any::{self, TypeId},
    io::Cursor,
    mem,
};

use bevy::{ecs::entity::MapEntities, prelude::*};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use crate::client::client_mapper::ClientMapper;

/// Erased serialization and deserialization function pointers for a component.
pub struct SerdeFns {
    type_id: TypeId,
    type_name: &'static str,

    serialize: unsafe fn(),
    deserialize: unsafe fn(),
    deserialize_in_place: unsafe fn(),
}

impl SerdeFns {
    /// Creates a new instance for `C` by erasing the passed function pointers.
    ///
    /// All other functions should be called with the same `C`.
    pub(super) fn new<C: Component>(
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> Self {
        Self {
            type_id: TypeId::of::<C>(),
            type_name: any::type_name::<C>(),
            serialize: unsafe { mem::transmute(serialize) },
            deserialize: unsafe { mem::transmute(deserialize) },
            deserialize_in_place: unsafe { mem::transmute(deserialize_in_place) },
        }
    }

    /// Serializes a component into a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which it was created.
    pub unsafe fn serialize<C: Component>(
        &self,
        component: &C,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        self.debug_type_check::<C>();

        let serialize: SerializeFn<C> = mem::transmute(self.serialize);
        (serialize)(component, cursor)
    }

    /// Deserializes a component from a cursor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which it was created.
    pub unsafe fn deserialize<C: Component>(
        &self,
        cursor: &mut Cursor<&[u8]>,
        mapper: &mut ClientMapper,
    ) -> bincode::Result<C> {
        self.debug_type_check::<C>();

        let deserialize: DeserializeFn<C> = mem::transmute(self.deserialize);
        (deserialize)(cursor, mapper)
    }

    /// Same as [`Self::deserialize`], but instead of returning a component, it updates the passed reference.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which it was created.
    pub unsafe fn deserialize_in_place<C: Component>(
        &self,
        component: &mut C,
        cursor: &mut Cursor<&[u8]>,
        mapper: &mut ClientMapper,
    ) -> bincode::Result<()> {
        self.debug_type_check::<C>();

        let deserialize_in_place: DeserializeInPlaceFn<C> =
            mem::transmute(self.deserialize_in_place);
        let deserialize: DeserializeFn<C> = mem::transmute(self.deserialize);
        (deserialize_in_place)(deserialize, component, cursor, mapper)
    }

    /// Panics if a component differs from [`Self::new`].
    fn debug_type_check<C: Component>(&self) {
        debug_assert_eq!(
            self.type_id,
            TypeId::of::<C>(),
            "trying to call serde functions with {}, but they were created with {}",
            any::type_name::<C>(),
            self.type_name,
        );
    }
}

/// Signature of component serialization functions.
pub type SerializeFn<C> = fn(&C, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component deserialization functions.
pub type DeserializeFn<C> = fn(&mut Cursor<&[u8]>, &mut ClientMapper) -> bincode::Result<C>;

/// Signature of component deserialization functions.
pub type DeserializeInPlaceFn<C> =
    fn(DeserializeFn<C>, &mut C, &mut Cursor<&[u8]>, &mut ClientMapper) -> bincode::Result<()>;

/// Default component serialization function.
pub fn default_serialize<C: Component + Serialize>(
    component: &C,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default component deserialization function.
pub fn default_deserialize<C: Component + DeserializeOwned>(
    cursor: &mut Cursor<&[u8]>,
    _mapper: &mut ClientMapper,
) -> bincode::Result<C> {
    DefaultOptions::new().deserialize_from(cursor)
}

/// Like [`default_deserialize`], but also maps entities before insertion.
pub fn default_deserialize_mapped<C: Component + DeserializeOwned + MapEntities>(
    cursor: &mut Cursor<&[u8]>,
    mapper: &mut ClientMapper,
) -> bincode::Result<C> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;
    component.map_entities(mapper);
    Ok(component)
}

/// Default component in-place deserialization function.
///
/// This implementation just assigns the value from the passed deserialization function.
pub fn in_place_as_deserialize<C: Component + DeserializeOwned>(
    deserialize: DeserializeFn<C>,
    component: &mut C,
    cursor: &mut Cursor<&[u8]>,
    mapper: &mut ClientMapper,
) -> bincode::Result<()> {
    *component = (deserialize)(cursor, mapper)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[test]
    #[should_panic]
    fn packing() {
        let serde_fns = SerdeFns::new(
            default_serialize::<ComponentA>,
            default_deserialize::<ComponentA>,
            in_place_as_deserialize::<ComponentA>,
        );

        // SAFETY: Called with a different type, but should panic in debug mode.
        unsafe {
            serde_fns
                .serialize(&ComponentB, &mut Cursor::default())
                .ok();
        }
    }

    #[derive(Component, Serialize, Deserialize)]
    struct ComponentA;

    #[derive(Component)]
    struct ComponentB;
}
