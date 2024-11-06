pub(super) mod init_message;
pub(super) mod update_message;

use std::{
    io::{Cursor, Write},
    mem,
    time::Duration,
};

use bevy::{ecs::component::Tick, prelude::*};
use varint_rs::VarintWriter;

use crate::core::{
    replicated_clients::{ClientBuffers, ReplicatedClient, ReplicatedClients},
    replicon_server::RepliconServer,
    replicon_tick::RepliconTick,
};
use init_message::InitMessage;
use update_message::UpdateMessage;

/// Accumulates replication messages and sends them to clients.
///
/// Messages are serialized and deserialized manually because using an intermediate structure
/// leads to allocations and according to our benchmarks it's much slower.
///
/// Reuses allocated memory from older messages.
#[derive(Default)]
pub(crate) struct ReplicationMessages {
    replicated_clients: ReplicatedClients,
    data: Vec<(InitMessage, UpdateMessage)>,
}

impl ReplicationMessages {
    /// Initializes messages for each client.
    ///
    /// Reuses already allocated messages.
    /// Creates new messages if the number of clients is bigger then the number of allocated messages.
    /// If there are more messages than the number of clients, then the extra messages remain untouched
    /// and iteration methods will not include them.
    pub(super) fn prepare(&mut self, replicated_clients: ReplicatedClients) {
        self.data
            .reserve(replicated_clients.len().saturating_sub(self.data.len()));

        for index in 0..replicated_clients.len() {
            if let Some((init_message, update_message)) = self.data.get_mut(index) {
                init_message.reset();
                update_message.reset();
            } else {
                self.data.push(Default::default());
            }
        }

        self.replicated_clients = replicated_clients;
    }

    /// Returns iterator over messages for each client.
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut (InitMessage, UpdateMessage)> {
        self.data.iter_mut().take(self.replicated_clients.len())
    }

    /// Same as [`Self::iter_mut`], but also includes [`ReplicatedClient`].
    pub(super) fn iter_mut_with_clients(
        &mut self,
    ) -> impl Iterator<Item = (&mut InitMessage, &mut UpdateMessage, &mut ReplicatedClient)> {
        self.data
            .iter_mut()
            .zip(self.replicated_clients.iter_mut())
            .map(|((init_message, update_message), client)| (init_message, update_message, client))
    }

    /// Sends cached messages to clients specified in the last [`Self::prepare`] call.
    ///
    /// The change tick of each client with an init message is updated to equal the latest replicon tick.
    /// messages were sent to clients. If only update messages were sent (or no messages at all) then
    /// it will equal the input `last_change_tick`.
    pub(super) fn send(
        &mut self,
        server: &mut RepliconServer,
        client_buffers: &mut ClientBuffers,
        server_tick: RepliconTick,
        tick: Tick,
        timestamp: Duration,
    ) -> bincode::Result<ReplicatedClients> {
        for ((init_message, update_message), client) in
            self.data.iter_mut().zip(self.replicated_clients.iter_mut())
        {
            init_message.send(server, client, server_tick)?;
            update_message.send(server, client_buffers, client, server_tick, tick, timestamp)?;
            client.visibility_mut().update();
        }

        let replicated_clients = mem::take(&mut self.replicated_clients);

        Ok(replicated_clients)
    }
}

/// Writes new data into a cursor and returns the serialized size.
///
/// Reuses previously shared bytes if they exist, or updates them.
/// Serialized size should be less then [`u16`].
fn write_with<'a>(
    shared_bytes: &mut Option<&'a [u8]>,
    cursor: &'a mut Cursor<Vec<u8>>,
    write_fn: impl FnOnce(&mut Cursor<Vec<u8>>) -> bincode::Result<()>,
) -> bincode::Result<u16> {
    let bytes = if let Some(bytes) = shared_bytes {
        cursor.write_all(bytes)?;
        bytes
    } else {
        let previous_pos = cursor.position() as usize;
        (write_fn(cursor))?;
        let current_pos = cursor.position() as usize;

        let buffer = cursor.get_ref();
        let bytes = &buffer[previous_pos..current_pos];
        *shared_bytes = Some(bytes);

        bytes
    };

    let size = bytes
        .len()
        .try_into()
        .map_err(|_| bincode::ErrorKind::SizeLimit)?;

    Ok(size)
}

/// Serializes `entity` by writing its index and generation as separate varints.
///
/// The index is first prepended with a bit flag to indicate if the generation
/// is serialized or not. It is not serialized if <= 1; note that generations are [`NonZeroU32`](std::num::NonZeroU32)
/// and a value of zero is used in [`Option<Entity>`] to signify [`None`], so generation 1 is the first
/// generation.
fn serialize_entity(cursor: &mut Cursor<Vec<u8>>, entity: Entity) -> bincode::Result<()> {
    let mut flagged_index = (entity.index() as u64) << 1;
    let flag = entity.generation() > 1;
    flagged_index |= flag as u64;

    cursor.write_u64_varint(flagged_index)?;
    if flag {
        cursor.write_u32_varint(entity.generation() - 1)?;
    }

    Ok(())
}
