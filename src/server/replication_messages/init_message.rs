use std::{
    io::{Cursor, Write},
    mem,
};

use bevy::{prelude::*, ptr::Ptr};
use bincode::{DefaultOptions, Options};
use bytes::Bytes;

use super::update_message::UpdateMessage;
use crate::{
    core::{
        channels::ReplicationChannel,
        replication::{
            replicated_clients::ReplicatedClient,
            replication_registry::{
                component_fns::ComponentFns, ctx::SerializeCtx, rule_fns::UntypedRuleFns, FnsId,
            },
        },
        replicon_server::RepliconServer,
        replicon_tick::RepliconTick,
    },
    server::client_entity_map::ClientMapping,
};

/// A reusable message with replicated data.
///
/// Contains tick and mappings, insertions, removals and despawns that
/// happened on this tick.
/// Sent over [`ReplicationChannel::Init`] channel.
///
/// See also [Limits](../index.html#limits)
pub(crate) struct InitMessage {
    /// Serialized data.
    cursor: Cursor<Vec<u8>>,

    /// Length of the array that updated automatically after writing data.
    array_len: u16,

    /// Position of the array from last call of [`Self::start_array`].
    array_pos: u64,

    /// The number of empty arrays at the end.
    trailing_empty_arrays: usize,

    /// Entity from last call of [`Self::start_entity_data`].
    data_entity: Entity,

    /// Size in bytes of the component data stored for the currently-being-written entity.
    entity_data_size: u16,

    /// Position of entity from last call of [`Self::start_entity_data`].
    entity_data_pos: u64,

    /// Position of entity data length from last call of [`Self::write_data_entity`].
    entity_data_size_pos: u64,
}

impl InitMessage {
    /// Clears the message.
    ///
    /// Keeps allocated capacity for reuse.
    pub(super) fn reset(&mut self) {
        self.cursor.set_position(0);
        self.trailing_empty_arrays = 0;
    }

    /// Returns size in bytes of the current entity data.
    ///
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(crate) fn entity_data_size(&self) -> u16 {
        self.entity_data_size
    }

    /// Starts writing array by remembering its position to write length after.
    ///
    /// Arrays can contain entity data or despawns inside.
    /// See also [`Self::end_array`], [`Self::write_client_mapping`], [`Self::write_entity`] and [`Self::start_entity_data`].
    pub(crate) fn start_array(&mut self) {
        debug_assert_eq!(self.array_len, 0);

        self.array_pos = self.cursor.position();
        self.cursor
            .set_position(self.array_pos + mem::size_of_val(&self.array_len) as u64);
    }

    /// Ends writing array by writing its length into the last remembered position.
    ///
    /// See also [`Self::start_array`].
    pub(crate) fn end_array(&mut self) -> bincode::Result<()> {
        if self.array_len != 0 {
            let previous_pos = self.cursor.position();
            self.cursor.set_position(self.array_pos);

            bincode::serialize_into(&mut self.cursor, &self.array_len)?;

            self.cursor.set_position(previous_pos);
            self.array_len = 0;
            self.trailing_empty_arrays = 0;
        } else {
            self.trailing_empty_arrays += 1;
            self.cursor.set_position(self.array_pos);
            bincode::serialize_into(&mut self.cursor, &self.array_len)?;
        }

        Ok(())
    }

    /// Serializes entity to entity mapping as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`].
    pub(crate) fn write_client_mapping(&mut self, mapping: &ClientMapping) -> bincode::Result<()> {
        super::serialize_entity(&mut self.cursor, mapping.server_entity)?;
        super::serialize_entity(&mut self.cursor, mapping.client_entity)?;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes entity as an array element.
    ///
    /// Reuses previously shared bytes if they exist, or updates them.
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`].
    pub(crate) fn write_entity<'a>(
        &'a mut self,
        shared_bytes: &mut Option<&'a [u8]>,
        entity: Entity,
    ) -> bincode::Result<()> {
        super::write_with(shared_bytes, &mut self.cursor, |cursor| {
            super::serialize_entity(cursor, entity)
        })?;

        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Starts writing entity and its data as an array element.
    ///
    /// Should be called only inside an array and increases its length by 1.
    /// Data can contain components with their IDs or IDs only.
    /// Entity will be written lazily after first data write.
    /// See also [`Self::end_entity_data`] and [`Self::write_component`].
    pub(crate) fn start_entity_data(&mut self, entity: Entity) {
        debug_assert_eq!(self.entity_data_size, 0);

        self.data_entity = entity;
        self.entity_data_pos = self.cursor.position();
    }

    /// Writes entity for the current data and remembers the position after it to write length later.
    ///
    /// Should be called only after first data write.
    fn write_data_entity(&mut self) -> bincode::Result<()> {
        super::serialize_entity(&mut self.cursor, self.data_entity)?;
        self.entity_data_size_pos = self.cursor.position();
        self.cursor.set_position(
            self.entity_data_size_pos + mem::size_of_val(&self.entity_data_size) as u64,
        );

        Ok(())
    }

    /// Ends writing entity data by writing its length into the last remembered position.
    ///
    /// If the entity data is empty, nothing will be written unless `save_empty` is set to true.
    /// Should be called only inside an array and increases its length by 1.
    /// See also [`Self::start_array`], [`Self::write_component`] and
    /// [`Self::write_component_id`].
    pub(crate) fn end_entity_data(&mut self, save_empty: bool) -> bincode::Result<()> {
        if self.entity_data_size == 0 && !save_empty {
            self.cursor.set_position(self.entity_data_pos);
            return Ok(());
        }

        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        self.cursor.set_position(self.entity_data_size_pos);

        bincode::serialize_into(&mut self.cursor, &self.entity_data_size)?;

        self.cursor.set_position(previous_pos);
        self.entity_data_size = 0;
        self.array_len = self
            .array_len
            .checked_add(1)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes component and its replication functions ID as an element of entity data.
    ///
    /// Reuses previously shared bytes if they exist, or updates them.
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(crate) fn write_component<'a>(
        &'a mut self,
        shared_bytes: &mut Option<&'a [u8]>,
        rule_fns: &UntypedRuleFns,
        component_fns: &ComponentFns,
        ctx: &SerializeCtx,
        fns_id: FnsId,
        ptr: Ptr,
    ) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let size = super::write_with(shared_bytes, &mut self.cursor, |cursor| {
            DefaultOptions::new().serialize_into(&mut *cursor, &fns_id)?;
            // SAFETY: `component_fns`, `ptr` and `rule_fns` were created for the same component type.
            unsafe { component_fns.serialize(ctx, rule_fns, ptr, cursor) }
        })?;

        self.entity_data_size = self
            .entity_data_size
            .checked_add(size)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Serializes replication functions ID as an element of entity data.
    ///
    /// Should be called only inside an entity data and increases its size.
    /// See also [`Self::start_entity_data`].
    pub(crate) fn write_fns_id(&mut self, fns_id: FnsId) -> bincode::Result<()> {
        if self.entity_data_size == 0 {
            self.write_data_entity()?;
        }

        let previous_pos = self.cursor.position();
        DefaultOptions::new().serialize_into(&mut self.cursor, &fns_id)?;

        let id_size = self.cursor.position() - previous_pos;
        self.entity_data_size = self
            .entity_data_size
            .checked_add(id_size as u16)
            .ok_or(bincode::ErrorKind::SizeLimit)?;

        Ok(())
    }

    /// Removes entity data elements from update message and copies it.
    ///
    /// Ends entity data for the update message.
    /// See also [`Self::start_entity_data`] and [`Self::end_entity_data`].
    pub(crate) fn take_entity_data(
        &mut self,
        update_message: &mut UpdateMessage,
    ) -> bincode::Result<()> {
        if update_message.entity_data_size != 0 {
            if self.entity_data_size == 0 {
                self.write_data_entity()?;
            }

            let slice = update_message.as_slice();
            let offset = update_message.entity_data_size_pos as usize
                + mem::size_of_val(&update_message.entity_data_size);
            self.cursor.write_all(&slice[offset..]).unwrap();

            self.entity_data_size = self
                .entity_data_size
                .checked_add(update_message.entity_data_size)
                .ok_or(bincode::ErrorKind::SizeLimit)?;
            update_message.entity_data_size = 0;
        }

        update_message
            .cursor
            .set_position(update_message.entity_data_pos);

        Ok(())
    }

    /// Returns the serialized data, excluding trailing empty arrays, as a byte array.
    fn as_slice(&self) -> &[u8] {
        let slice = self.cursor.get_ref();
        let position = self.cursor.position() as usize;
        let extra_len = self.trailing_empty_arrays * mem::size_of_val(&self.array_len);
        &slice[..position - extra_len]
    }

    /// Sends the message, excluding trailing empty arrays, to the specified client.
    ///
    /// Updates change tick for the client if there are data to send.
    /// Does nothing if there is no data to send.
    pub(crate) fn send(
        &self,
        server: &mut RepliconServer,
        client: &mut ReplicatedClient,
        server_tick: RepliconTick,
    ) -> bincode::Result<()> {
        debug_assert_eq!(self.array_len, 0);
        debug_assert_eq!(self.entity_data_size, 0);

        let slice = self.as_slice();
        if slice.is_empty() {
            trace!("no init data to send for {:?}", client.id());
            return Ok(());
        }

        client.set_init_tick(server_tick);

        let mut header = [0; mem::size_of::<RepliconTick>()];
        bincode::serialize_into(&mut header[..], &server_tick)?;

        trace!("sending init message to {:?}", client.id());
        server.send(
            client.id(),
            ReplicationChannel::Init,
            Bytes::from([&header, slice].concat()),
        );

        Ok(())
    }
}

impl Default for InitMessage {
    fn default() -> Self {
        Self {
            cursor: Default::default(),
            array_len: Default::default(),
            array_pos: Default::default(),
            trailing_empty_arrays: Default::default(),
            entity_data_size: Default::default(),
            entity_data_pos: Default::default(),
            entity_data_size_pos: Default::default(),
            data_entity: Entity::PLACEHOLDER,
        }
    }
}
