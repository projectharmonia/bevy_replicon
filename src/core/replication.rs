pub mod change_message_flags;
pub mod command_markers;
pub mod deferred_entity;
pub mod replicated_clients;
pub mod replication_registry;
pub mod replication_rules;
pub mod track_mutate_messages;

use bevy::prelude::*;

#[deprecated(note = "use `Replicated` instead")]
pub type Replication = Replicated;

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replicated;
