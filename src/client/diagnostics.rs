use bevy::diagnostic::DiagnosticPath;
use bevy::{
    diagnostic::{Diagnostic, Diagnostics, RegisterDiagnostic},
    prelude::*,
    time::common_conditions::on_timer,
};
use std::time::Duration;

use super::ClientReplicationStats;

/// Plugin to write [`Diagnostics`] based on [`ClientReplicationStats`] every second.
///
/// Adds [`ClientReplicationStats`] resource and automatically resets it to get diagnostics per second.
pub struct ClientDiagnosticsPlugin;

impl Plugin for ClientDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientReplicationStats>()
            .add_systems(
                Update,
                Self::add_measurements.run_if(on_timer(Duration::from_secs(1))),
            )
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
        DiagnosticPath::const_new("client/replication/entity_changes");
    /// How many components modified per second by replication.
    pub const COMPONENT_CHANGES: DiagnosticPath =
        DiagnosticPath::const_new("client/replication/component_changes");
    /// How many client-mappings added per second by replication.
    pub const MAPPINGS: DiagnosticPath = DiagnosticPath::const_new("client/replication/mappings");
    /// How many despawns per second from replication.
    pub const DESPAWNS: DiagnosticPath = DiagnosticPath::const_new("client/replication/despawns");
    /// How many replication messages processed per second.
    pub const MESSAGES: DiagnosticPath = DiagnosticPath::const_new("client/replication/messages");
    /// How many bytes of replication messages payloads per second.
    pub const BYTES: DiagnosticPath = DiagnosticPath::const_new("client/replication/bytes");

    /// Max diagnostic history length.
    pub const DIAGNOSTIC_HISTORY_LEN: usize = 60;

    fn add_measurements(mut stats: ResMut<ClientReplicationStats>, mut diagnostics: Diagnostics) {
        diagnostics.add_measurement(&Self::ENTITY_CHANGES, || stats.entities_changed as f64);
        diagnostics.add_measurement(&Self::COMPONENT_CHANGES, || stats.components_changed as f64);
        diagnostics.add_measurement(&Self::MAPPINGS, || stats.mappings as f64);
        diagnostics.add_measurement(&Self::DESPAWNS, || stats.despawns as f64);
        diagnostics.add_measurement(&Self::BYTES, || stats.bytes as f64);
        diagnostics.add_measurement(&Self::MESSAGES, || stats.messages as f64);
        *stats = ClientReplicationStats::default();
    }
}
