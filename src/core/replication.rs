pub mod command_markers;
pub mod deferred_entity;
pub mod replicated_clients;
pub mod replication_registry;
pub mod replication_rules;

use bevy::prelude::*;

#[deprecated(note = "use `Replicated` instead")]
pub type Replication = Replicated;

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replicated;

use bitflags::bitflags;

bitflags! {
    /// Types of data that can be optionally included inside init message if the bit is set.
    ///
    /// Serialized at the beginning of the message.
    #[derive(Default, Clone, Copy)]
    pub(crate) struct InitMessageHeader: u8 {
        const MAPPINGS = 0b00000001;
        const DESPAWNS = 0b00000010;
        const REMOVALS = 0b00000100;
        const CHANGES = 0b00001000;
    }
}
