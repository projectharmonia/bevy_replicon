use std::{mem, time::Duration};

use bevy::{ecs::component::Tick, prelude::*};
use bevy_renet::renet::{Bytes, ClientId, RenetServer};

use super::{replication_buffer::ReplicationBuffer, ClientInfo, LastChangeTick};
use crate::{
    replicon_core::{replicon_tick::RepliconTick, ReplicationChannel},
    server::clients_info::UpdateInfo,
};

/// Accumulates replication messages and sends them to clients.
///
/// Messages are serialized and deserialized manually because using an intermediate structure
/// leads to allocations and according to our benchmarks it's much slower.
///
/// Reuses allocated memory from older messages.
#[derive(Default)]
pub(crate) struct ReplicationMessages {
    info: Vec<ClientInfo>,
    data: Vec<(InitMessage, UpdateMessage)>,
    clients_count: usize,
}

impl ReplicationMessages {
    /// Initializes messages for each client.
    ///
    /// Reuses already allocated messages.
    /// Creates new messages if the number of clients is bigger then the number of allocated messages.
    /// If there are more messages than the number of clients, then the extra messages remain untouched
    /// and iteration methods will not include them.
    pub(super) fn prepare(
        &mut self,
        info: Vec<ClientInfo>,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        self.clients_count = info.len();

        self.data.reserve(self.clients_count);

        for index in 0..info.len() {
            if let Some((init_message, update_message)) = self.data.get_mut(index) {
                init_message.reset(replicon_tick)?;
                update_message.reset()?;
            } else {
                self.data
                    .push((InitMessage::new(replicon_tick)?, UpdateMessage::default()));
            }
        }

        self.info = info;

        Ok(())
    }

    /// Returns iterator over messages for each client.
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut (InitMessage, UpdateMessage)> {
        self.data.iter_mut().take(self.clients_count)
    }

    /// Same as [`Self::iter_mut`], but also iterates over clients info.
    pub(super) fn iter_mut_with_info(
        &mut self,
    ) -> impl Iterator<Item = (&mut InitMessage, &mut UpdateMessage, &mut ClientInfo)> {
        self.data
            .iter_mut()
            .take(self.clients_count)
            .zip(&mut self.info)
            .map(|((init_message, update_message), client_info)| {
                (init_message, update_message, client_info)
            })
    }

    /// Sends cached messages to clients specified in the last [`Self::prepare`] call.
    ///
    /// Returns the server's last change tick, which will equal the latest replicon tick if any init
    /// messages were sent to clients. If only update messages were sent (or no messages at all) then
    /// it will equal the input `last_change_tick`.
    pub(super) fn send(
        &mut self,
        server: &mut RenetServer,
        entity_buffer: &mut Vec<Vec<Entity>>,
        mut last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<(LastChangeTick, Vec<ClientInfo>)> {
        if let Some((init_message, _)) = self.data.first() {
            if init_message.is_sendable() {
                last_change_tick.0 = replicon_tick;
            }
        }

        for ((init_message, update_message), client_info) in self
            .data
            .iter_mut()
            .take(self.clients_count)
            .zip(&mut self.info)
        {
            init_message.send(server, client_info.id);
            update_message.send(
                server,
                entity_buffer,
                client_info,
                last_change_tick,
                replicon_tick,
                tick,
                timestamp,
            )?;
        }

        Ok((last_change_tick, mem::take(&mut self.info)))
    }
}

/// A reusable message with replicated data.
///
/// Contains tick and mappings, insertions, removals and despawns that
/// happened on this tick.
/// Sent over [`ReplicationChannel::Reliable`] channel.
///
/// See also [Limits](../index.html#limits)
#[derive(Deref, DerefMut)]
pub(super) struct InitMessage {
    /// Message data.
    #[deref]
    buffer: ReplicationBuffer,
}

impl InitMessage {
    /// Creates a new message for the specified tick.
    fn new(replicon_tick: RepliconTick) -> bincode::Result<Self> {
        let mut buffer = ReplicationBuffer::default();
        buffer.write(&replicon_tick)?;

        Ok(Self { buffer })
    }

    /// Clears the message and assigns tick to it.
    ///
    /// Keeps allocated capacity of the buffer.
    fn reset(&mut self, replicon_tick: RepliconTick) -> bincode::Result<()> {
        self.buffer.reset();
        self.buffer.write(&replicon_tick)
    }

    /// Returns `true` is message contains any non-empty arrays.
    fn is_sendable(&self) -> bool {
        self.buffer.arrays_with_data() != 0
    }

    /// Trims empty arrays from the message and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    fn send(&mut self, server: &mut RenetServer, client_id: ClientId) {
        if !self.is_sendable() {
            trace!("no init data to send for client {client_id}");
            return;
        }

        self.buffer.trim_empty_arrays();

        trace!("sending init message to client {client_id}");
        server.send_message(
            client_id,
            ReplicationChannel::Reliable,
            Bytes::copy_from_slice(self.buffer.as_slice()),
        );
    }
}

/// A reusable message with replicated component updates.
///
/// Contains last change tick, current tick and component updates since the last acknowledged tick for each entity.
/// Cannot be applied on the client until the init message matching this update message's last change tick
/// has been applied to the client world.
/// The message will be manually split into packets up to max size, and each packet will be applied
/// independently on the client.
/// Message splits only happen per-entity to avoid weird behavior from partial entity updates.
/// Sent over the [`ReplicationChannel::Unreliable`] channel.
///
/// See also [Limits](../index.html#limits)
#[derive(Deref, DerefMut, Default)]
pub(super) struct UpdateMessage {
    /// Entities and their data sizes.
    entities: Vec<(Entity, usize)>,

    /// Message data.
    #[deref]
    buffer: ReplicationBuffer,
}

impl UpdateMessage {
    /// Clears the message.
    ///
    /// Keeps allocated capacity of the buffer.
    fn reset(&mut self) -> bincode::Result<()> {
        self.entities.clear();
        self.buffer.reset();

        Ok(())
    }

    /// Registers entity from buffer's entity data and its size for possible splitting.
    pub(super) fn register_entity(&mut self) {
        let data_size = self.buffer.as_slice().len() - self.buffer.entity_data_pos() as usize;
        self.entities.push((self.buffer.data_entity(), data_size));
    }

    /// Returns `true` is message contains any written data.
    fn is_sendable(&self) -> bool {
        !self.buffer.as_slice().is_empty()
    }

    /// Splits message according to entities inside it and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    #[allow(clippy::too_many_arguments)]
    fn send(
        &mut self,
        server: &mut RenetServer,
        entity_buffer: &mut Vec<Vec<Entity>>,
        client_info: &mut ClientInfo,
        last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<()> {
        if !self.is_sendable() {
            trace!("no updates to send for client {}", client_info.id);
            return Ok(());
        }

        trace!("sending update message(s) to client {}", client_info.id);
        const TICKS_SIZE: usize = 2 * mem::size_of::<RepliconTick>();
        let mut header = [0; TICKS_SIZE + mem::size_of::<u16>()];
        bincode::serialize_into(&mut header[..], &(*last_change_tick, replicon_tick))?;

        let mut slice = self.buffer.as_slice();
        let mut entities = entity_buffer.pop().unwrap_or_default();
        let mut message_size = 0;
        for &(entity, data_size) in &self.entities {
            // Try to pack back first, then try to pack forward.
            if message_size == 0
                || can_pack(header.len(), message_size, data_size)
                || can_pack(header.len(), data_size, message_size)
            {
                entities.push(entity);
                message_size += data_size;
            } else {
                let (message, remaining) = slice.split_at(message_size);
                slice = remaining;
                message_size = data_size;

                let update_info = UpdateInfo {
                    tick,
                    timestamp,
                    entities,
                };
                let update_index = client_info.register_update(update_info);
                bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

                server.send_message(
                    client_info.id,
                    ReplicationChannel::Unreliable,
                    Bytes::from_iter(header.into_iter().chain(message.iter().copied())),
                );

                entities = entity_buffer.pop().unwrap_or_default();
            }
        }

        if !slice.is_empty() {
            let update_info = UpdateInfo {
                tick,
                timestamp,
                entities,
            };
            let update_index = client_info.register_update(update_info);
            bincode::serialize_into(&mut header[TICKS_SIZE..], &update_index)?;

            server.send_message(
                client_info.id,
                ReplicationChannel::Unreliable,
                Bytes::from_iter(header.into_iter().chain(slice.iter().copied())),
            );
        }

        Ok(())
    }
}

fn can_pack(header_len: usize, base: usize, add: usize) -> bool {
    const MAX_PACKET_SIZE: usize = 1200; // https://github.com/lucaspoffo/renet/blob/acee8b470e34c70d35700d96c00fb233d9cf6919/renet/src/packet.rs#L7

    let dangling = (base + header_len) % MAX_PACKET_SIZE;
    (dangling > 0) && ((dangling + add) <= MAX_PACKET_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing() {
        assert!(can_pack(10, 0, 5));
        assert!(can_pack(10, 0, 1190));
        assert!(!can_pack(10, 0, 1191));
        assert!(!can_pack(10, 0, 3000));

        assert!(can_pack(10, 1189, 1));
        assert!(!can_pack(10, 1190, 0));
        assert!(!can_pack(10, 1190, 1));
        assert!(!can_pack(10, 1190, 3000));
    }
}
