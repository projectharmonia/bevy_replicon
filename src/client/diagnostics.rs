use bevy::diagnostic::DiagnosticPath;
use bevy::{
    diagnostic::{Diagnostic, Diagnostics, RegisterDiagnostic},
    prelude::*,
    time::common_conditions::on_timer,
};
use std::time::Duration;

/// Replication stats during message processing.
///
/// Flushed to Diagnostics system periodically.
#[derive(Default, Resource, Debug)]
pub struct ClientStats {
    /// Incremented per entity that changes.
    pub entities_changed: u32,
    /// Incremented for every component that changes.
    pub components_changed: u32,
    /// Incremented per client mapping added.
    pub mappings: u32,
    /// Incremented per entity despawn.
    pub despawns: u32,
    /// Replication messages received.
    pub messages: u32,
    /// Replication bytes received in message payloads (without internal messaging plugin data).
    pub bytes: u64,
}

/// Plugin to write Diagnostics every second.
///
/// Not added by default.
pub struct ClientDiagnosticsPlugin;

impl Plugin for ClientDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            Self::add_measurements.run_if(on_timer(Duration::from_secs(1))),
        )
        .init_resource::<ClientStats>()
        .register_diagnostic(
            Diagnostic::new(Self::ENTITY_CHANGES)
                .with_suffix("entities changed per second")
                .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
        )
        .register_diagnostic(
            Diagnostic::new(Self::COMPONENT_CHANGES)
                .with_suffix("components changed per second")
                .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
        )
        .register_diagnostic(
            Diagnostic::new(Self::MAPPINGS)
                .with_suffix("mappings added per second")
                .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
        )
        .register_diagnostic(
            Diagnostic::new(Self::DESPAWNS)
                .with_suffix("despawns per second")
                .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
        )
        .register_diagnostic(
            Diagnostic::new(Self::MESSAGES)
                .with_suffix("messages per second")
                .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
        )
        .register_diagnostic(
            Diagnostic::new(Self::BYTES)
                .with_suffix("bytes per second")
                .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
        );
    }
}

impl ClientDiagnosticsPlugin {
    /// How many entities modified per second by replication.
    pub const ENTITY_CHANGES: DiagnosticPath =
        DiagnosticPath::const_new("replication.client.entity_changes");
    /// How many components modified per second by replication.
    pub const COMPONENT_CHANGES: DiagnosticPath =
        DiagnosticPath::const_new("replication.client.component_changes");
    /// How many client-mappings added per second by replication.
    pub const MAPPINGS: DiagnosticPath = DiagnosticPath::const_new("replication.client.mappings");
    /// How many despawns per second from replication.
    pub const DESPAWNS: DiagnosticPath = DiagnosticPath::const_new("replication.client.despawns");
    /// How many replication messages processed per second.
    pub const MESSAGES: DiagnosticPath = DiagnosticPath::const_new("replication.client.messages");
    /// How many bytes of replication messages payloads per second.
    pub const BYTES: DiagnosticPath = DiagnosticPath::const_new("replication.client.bytes");

    /// Max diagnostic history length.
    pub const DIAGNOSTIC_HISTORY_LEN: usize = 60;

    fn add_measurements(mut stats: ResMut<ClientStats>, mut diagnostics: Diagnostics) {
        diagnostics.add_measurement(&Self::ENTITY_CHANGES, || stats.entities_changed as f64);
        diagnostics.add_measurement(&Self::COMPONENT_CHANGES, || stats.components_changed as f64);
        diagnostics.add_measurement(&Self::MAPPINGS, || stats.mappings as f64);
        diagnostics.add_measurement(&Self::DESPAWNS, || stats.despawns as f64);
        diagnostics.add_measurement(&Self::BYTES, || stats.bytes as f64);
        diagnostics.add_measurement(&Self::MESSAGES, || stats.messages as f64);
        *stats = ClientStats::default();
    }
}
