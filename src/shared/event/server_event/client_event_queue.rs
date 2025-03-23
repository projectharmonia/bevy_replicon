use std::{collections::BTreeMap, marker::PhantomData};

use bevy::prelude::*;
use bytes::Bytes;

use crate::shared::replicon_tick::RepliconTick;

/// Stores all received events from server that arrived earlier then replication message with their tick.
///
/// Stores data sorted by ticks and maintains order of arrival.
/// Needed to ensure that when an event is triggered, all the data that it affects or references already exists.
#[derive(Resource)]
pub(super) struct ClientEventQueue<E> {
    map: BTreeMap<RepliconTick, Vec<Bytes>>,
    /// [`Vec`]s from removals.
    ///
    /// All data is drained before the insertion.
    /// Stored to reuse allocated capacity.
    buffer: Vec<Vec<Bytes>>,
    marker: PhantomData<E>,
}

impl<E> ClientEventQueue<E> {
    pub(super) fn insert(&mut self, tick: RepliconTick, message: Bytes) {
        self.map
            .entry(tick)
            .or_insert_with(|| self.buffer.pop().unwrap_or_default())
            .push(message);
    }

    /// Pops the next event that is at least as old as the specified replicon tick.
    pub(super) fn pop_if_le(
        &mut self,
        update_tick: RepliconTick,
    ) -> Option<(RepliconTick, impl IntoIterator<Item = Bytes>)> {
        let entry = self.map.first_entry()?;
        if *entry.key() > update_tick {
            return None;
        }

        let (tick, messages) = entry.remove_entry();
        self.buffer.push(messages);
        let messages = self.buffer.last_mut().unwrap();
        Some((tick, messages.drain(..)))
    }

    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub(super) fn clear(&mut self) {
        while let Some((_, messages)) = self.map.pop_first() {
            self.buffer.push(messages);
        }
    }
}

impl<E> Default for ClientEventQueue<E> {
    fn default() -> Self {
        Self {
            map: Default::default(),
            buffer: Default::default(),
            marker: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_tick() {
        let mut queue = ClientEventQueue::<DummyEvent>::default();
        queue.insert(RepliconTick::new(1), Default::default());

        assert_eq!(queue.len(), 1);
        assert!(queue.pop_if_le(RepliconTick::new(0)).is_none());
    }

    #[test]
    fn bigger_tick() {
        let mut queue = ClientEventQueue::<DummyEvent>::default();
        queue.insert(RepliconTick::new(1), Default::default());

        assert!(queue.pop_if_le(RepliconTick::new(2)).is_some());
        assert!(queue.is_empty());
    }

    #[test]
    fn ticks_ordering() {
        let mut queue = ClientEventQueue::<DummyEvent>::default();
        queue.insert(RepliconTick::new(0), Default::default());
        queue.insert(RepliconTick::new(1), Default::default());
        queue.insert(RepliconTick::new(2), Default::default());

        let (tick, _) = queue.pop_if_le(RepliconTick::new(1)).unwrap();
        assert_eq!(tick, RepliconTick::new(0));

        let (tick, _) = queue.pop_if_le(RepliconTick::new(1)).unwrap();
        assert_eq!(tick, RepliconTick::new(1));

        assert!(queue.pop_if_le(RepliconTick::new(1)).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn messages_ordering() {
        let mut queue = ClientEventQueue::<DummyEvent>::default();
        queue.insert(RepliconTick::new(0), Bytes::from_static(&[0]));
        queue.insert(RepliconTick::new(0), Bytes::from_static(&[1]));

        let (_, messages) = queue.pop_if_le(RepliconTick::new(0)).unwrap();
        let bytes: Vec<_> = messages.into_iter().flatten().collect();
        assert_eq!(bytes, [0, 1]);
        assert!(queue.is_empty());
    }

    struct DummyEvent;
}
