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
}

impl SerdeFns {
    pub(super) fn new<C>(
        commands_id: CommandFnsId,
        serialize: SerializeFn<C>,
        deserialize: DeserializeFn<C>,
    ) -> Self {
        Self {
            commands_id,
            serialize: unsafe { mem::transmute(serialize) },
            deserialize: unsafe { mem::transmute(deserialize) },
        }
    }

    pub unsafe fn serialize<C>(
        &self,
        component: &C,
        cursor: &mut Cursor<Vec<u8>>,
    ) -> bincode::Result<()> {
        let serialize: SerializeFn<C> = mem::transmute(self.serialize);
        (serialize)(component, cursor)
    }

    pub unsafe fn deserialize<C>(
        &self,
        cursor: &mut Cursor<&[u8]>,
        mapper: &mut ClientMapper,
    ) -> bincode::Result<C> {
        let deserialize: DeserializeFn<C> = mem::transmute(self.deserialize);
        (deserialize)(cursor, mapper)
    }

    pub(crate) fn commands_id(&self) -> CommandFnsId {
        self.commands_id
    }
}

/// Signature of component serialization functions.
pub type SerializeFn<C> = fn(&C, &mut Cursor<Vec<u8>>) -> bincode::Result<()>;

/// Signature of component deserialization functions.
pub type DeserializeFn<C> = fn(&mut Cursor<&[u8]>, &mut ClientMapper) -> bincode::Result<C>;

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
