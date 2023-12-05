pub(super) mod replication_buffer;

use std::mem;

use bevy::{ecs::component::Tick, prelude::*};
use bevy_renet::renet::{Bytes, ClientId, RenetServer};

use super::{ClientInfo, LastChangeTick};
use crate::replicon_core::{replicon_tick::RepliconTick, ReplicationChannel};
use replication_buffer::ReplicationBuffer;

/// Accumulates replication messages and sends them to clients.
///
/// Messages serialized and deserialized manually because using an intermediate structure
/// leads to allocations and according to our benchmarks it's much slower.
///
/// Reuses allocated memory from older messages.
#[derive(Default)]
pub(crate) struct ReplicationMessages {
    clients_info: Vec<ClientInfo>,
    data: Vec<(InitMessage, UpdateMessage)>,
    clients_count: usize,
}

impl ReplicationMessages {
    /// Initializes messages for each client.
    ///
    /// Reuses already allocated messages.
    /// Creates new messages if number of clients is bigger then the number of allocated messages.
    /// If there are more messages than the number of clients, then the extra messages remain untouched
    /// and iteration methods will not include them.
    pub(super) fn prepare(
        &mut self,
        clients_info: Vec<ClientInfo>,
        replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        self.clients_count = clients_info.len();

        self.data.reserve(self.clients_count);

        for index in 0..clients_info.len() {
            if let Some((init_message, update_message)) = self.data.get_mut(index) {
                init_message.reset(replicon_tick)?;
                update_message.reset()?;
            } else {
                self.data
                    .push((InitMessage::new(replicon_tick)?, UpdateMessage::default()));
            }
        }

        self.clients_info = clients_info;

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
            .zip(&mut self.clients_info)
            .map(|((init_message, update_message), client_info)| {
                (init_message, update_message, client_info)
            })
    }

    /// Sends all messages and returns updated last change tick with clients info that was consumed in [`Self::prepare`].
    pub(super) fn send(
        &mut self,
        server: &mut RenetServer,
        mut last_change_tick: LastChangeTick,
        replicon_tick: RepliconTick,
        tick: Tick,
    ) -> bincode::Result<(LastChangeTick, Vec<ClientInfo>)> {
        if let Some((init_message, _)) = self.data.last() {
            if init_message.arrays_with_data() != 0 {
                last_change_tick.0 = replicon_tick;
            }
        }

        for ((init_message, update_message), client_info) in self
            .data
            .iter_mut()
            .take(self.clients_count)
            .zip(&mut self.clients_info)
        {
            init_message.send(server, client_info.id);
            update_message.send(server, client_info, last_change_tick, tick)?;
        }

        Ok((last_change_tick, mem::take(&mut self.clients_info)))
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

    /// Trims empty arrays from message and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    fn send(&mut self, server: &mut RenetServer, client_id: ClientId) {
        if self.buffer.arrays_with_data() == 0 {
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

/// A reusable message with component updates.
///
/// Contains tick and component updates since the last tick until this tick for each entity.
/// Requires init message with the same tick to be applied to keep the world in valid state.
/// The message will be manually split into packets up to max size that can be applied independently.
/// Splits will happen per-entity to avoid weird behavior of partially changed entity.
/// Sent over [`ReplicationChannel::Unreliable`] channel.
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

    /// Registers entity from buffer's entity data for possible splitting.
    ///
    /// Ignores the entity if data length is 0.
    /// Should be called before [`ReplicationBuffer::end_entity_data`].
    pub(super) fn register_entity(&mut self) {
        if self.buffer.entity_data_len() != 0 {
            let data_size = self.buffer.as_slice().len() - self.buffer.entity_data_pos() as usize;
            self.entities.push((self.buffer.data_entity(), data_size));
        }
    }

    /// Splits message according to `entities` and sends it to the specified client.
    ///
    /// Does nothing if there is no data to send.
    fn send(
        &mut self,
        server: &mut RenetServer,
        client_info: &mut ClientInfo,
        last_change_tick: LastChangeTick,
        tick: Tick,
    ) -> bincode::Result<()> {
        if self.buffer.arrays_with_data() == 0 {
            trace!("no updates to send for client {}", client_info.id);
            return Ok(());
        }

        trace!("sending update message(s) to client {}", client_info.id);
        const TICK_SIZE: usize = mem::size_of::<RepliconTick>();
        let mut header = [0; TICK_SIZE + mem::size_of::<u16>()];
        bincode::serialize_into(&mut header[..], &*last_change_tick)?;

        let mut slice = self.buffer.as_slice();
        let mut entities = Vec::new();
        let mut message_size = 0;
        for &(entity, data_size) in &self.entities {
            const MAX_PACKET_SIZE: usize = 1200; // https://github.com/lucaspoffo/renet/blob/acee8b470e34c70d35700d96c00fb233d9cf6919/renet/src/packet.rs#L7
            if message_size + data_size + header.len() > MAX_PACKET_SIZE {
                let (message, remaining) = slice.split_at(message_size);
                slice = remaining;
                message_size = data_size;

                let update_index = client_info.register_update(tick, entities.clone());
                bincode::serialize_into(&mut header[TICK_SIZE..], &update_index)?;

                server.send_message(
                    client_info.id,
                    ReplicationChannel::Unreliable,
                    Bytes::from_iter(header.into_iter().chain(message.iter().copied())),
                );

                entities.clear();
            } else {
                entities.push(entity);
                println!("{message_size} increase by {data_size}");
                message_size += data_size;
            }
        }

        if !slice.is_empty() {
            println!("sending more data");
            let update_index = client_info.register_update(tick, entities);
            bincode::serialize_into(&mut header[TICK_SIZE..], &update_index)?;

            server.send_message(
                client_info.id,
                ReplicationChannel::Unreliable,
                Bytes::from_iter(header.into_iter().chain(slice.iter().copied())),
            );
        }

        Ok(())
    }
}
