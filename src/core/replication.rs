pub mod command_markers;
pub mod deferred_entity;
pub mod replicated_clients;
pub mod replication_registry;
pub mod replication_rules;
pub mod track_mutate_messages;
pub mod update_message_flags;

use bevy::prelude::*;

/// Marks entity for replication.
#[derive(Component, Clone, Copy, Default, Reflect, Debug)]
#[reflect(Component)]
pub struct Replicated;
