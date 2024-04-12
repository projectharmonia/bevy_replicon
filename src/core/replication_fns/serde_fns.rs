use std::{io::Cursor, mem};

use bevy::{ecs::entity::MapEntities, prelude::*};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use super::CommandFnsId;
use crate::client::client_mapper::ClientMapper;

pub struct SerdeFns {
    commands_id: CommandFnsId,
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
    deserialize_in_place: unsafe fn(),
}

impl SerdeFns {
    pub(super) fn new<C: Component>(
        commands_id: CommandFnsId,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
        deserialize_in_place: DeserializeInPlaceFn<C>,
    ) -> Self {
        Self {
            commands_id,
            serialize: unsafe { mem::transmute(serialize) },
            deserialize: unsafe { mem::transmute(deserialize) },
            deserialize_in_place: unsafe { mem::transmute(deserialize_in_place) },
        }
    }

    pub unsafe fn serialize<C: Component>(
        &self,
        component: &C,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        let serialize: SerializeFn<C> = mem::transmute(self.serialize);
        (serialize)(component, cursor)
    }

    pub unsafe fn deserialize<C: Component>(
        &self,
        cursor: &mut Cursor<&[u8]>,
        mapper: &mut ClientMapper,
    ) -> bincode::Result<C> {
        let deserialize: DeserializeFn<C> = mem::transmute(self.deserialize);
        (deserialize)(cursor, mapper)
    }

    pub unsafe fn deserialize_in_place<C: Component>(
        &self,
        component: &mut C,
        cursor: &mut Cursor<&[u8]>,
        mapper: &mut ClientMapper,
    ) -> bincode::Result<()> {
        let deserialize_in_place: DeserializeInPlaceFn<C> =
            mem::transmute(self.deserialize_in_place);
        let deserialize: DeserializeFn<C> = mem::transmute(self.deserialize);
        (deserialize_in_place)(deserialize, component, cursor, mapper)
    }

    pub(crate) fn commands_id(&self) -> CommandFnsId {
        self.commands_id
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
pub fn serialize<C: Component + Serialize>(
    component: &C,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default component deserialization function.
pub fn deserialize<C: Component + DeserializeOwned>(
    cursor: &mut Cursor<&[u8]>,
    _mapper: &mut ClientMapper,
) -> bincode::Result<C> {
    DefaultOptions::new().deserialize_from(cursor)
}

/// Like [`deserialize`], but also maps entities before insertion.
pub fn deserialize_mapped<C: Component + DeserializeOwned + MapEntities>(
    cursor: &mut Cursor<&[u8]>,
    mapper: &mut ClientMapper,
) -> bincode::Result<C> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;
    component.map_entities(mapper);
    Ok(component)
}

pub fn deserialize_in_place<C: Component + DeserializeOwned>(
    deserialize: DeserializeFn<C>,
    component: &mut C,
    cursor: &mut Cursor<&[u8]>,
    mapper: &mut ClientMapper,
) -> bincode::Result<()> {
    *component = (deserialize)(cursor, mapper)?;
    Ok(())
}
