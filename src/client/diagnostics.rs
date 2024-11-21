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
                Diagnostic::new(Self::ENTITIES_CHANGED)
                    .with_suffix("entities changed per second")
                    .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(Self::COMPONENTS_CHANGED)
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
                Diagnostic::new(Self::REPLICATION_MESSAGES)
                    .with_suffix("replication messages per second")
                    .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(Self::REPLICATION_BYTES)
                    .with_suffix("replication bytes per second")
                    .with_max_history_length(Self::DIAGNOSTIC_HISTORY_LEN),
            );
    }
}

impl ClientDiagnosticsPlugin {
    /// How many entities modified per second by replication.
    pub const ENTITIES_CHANGED: DiagnosticPath =
        DiagnosticPath::const_new("client/replication/entities_changed");
    /// How many components modified per second by replication.
    pub const COMPONENTS_CHANGED: DiagnosticPath =
        DiagnosticPath::const_new("client/replication/components_changed");
    /// How many client-mappings added per second by replication.
    pub const MAPPINGS: DiagnosticPath = DiagnosticPath::const_new("client/replication/mappings");
    /// How many despawns per second from replication.
    pub const DESPAWNS: DiagnosticPath = DiagnosticPath::const_new("client/replication/despawns");
    /// How many replication messages received per second.
    pub const REPLICATION_MESSAGES: DiagnosticPath =
        DiagnosticPath::const_new("client/replication/messages");
    /// How many replication bytes received per second.
    pub const REPLICATION_BYTES: DiagnosticPath =
        DiagnosticPath::const_new("client/replication/bytes");

    /// Max diagnostic history length.
    pub const DIAGNOSTIC_HISTORY_LEN: usize = 60;

    fn add_measurements(mut stats: ResMut<ClientReplicationStats>, mut diagnostics: Diagnostics) {
        diagnostics.add_measurement(&Self::ENTITIES_CHANGED, || stats.entities_changed as f64);
        diagnostics.add_measurement(&Self::COMPONENTS_CHANGED, || {
            stats.components_changed as f64
        });
        diagnostics.add_measurement(&Self::MAPPINGS, || stats.mappings as f64);
        diagnostics.add_measurement(&Self::DESPAWNS, || stats.despawns as f64);
        diagnostics.add_measurement(&Self::REPLICATION_MESSAGES, || stats.messages as f64);
        diagnostics.add_measurement(&Self::REPLICATION_BYTES, || stats.bytes as f64);
        *stats = ClientReplicationStats::default();
    }
}
