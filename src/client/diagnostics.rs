use bevy::diagnostic::DiagnosticId;
use bevy::{
    diagnostic::{Diagnostic, Diagnostics, RegisterDiagnostic},
    prelude::*,
    time::common_conditions::on_timer,
};
use std::time::Duration;

/// Collects replication stats during packet processing
/// flushed to Diagnostics system periodically.
#[derive(Default, Resource, Debug)]
pub struct ReplicationStats {
    // incremented per entity that changes
    pub entities_changed: u32,
    // incremented for every component that changes
    pub components_changed: u32,
    // incremented per client mapping added
    pub mappings: u32,
    // incrementer per entity despawn
    pub despawns: u32,
    // replication packets recvd
    pub packets: u32,
    // replication bytes received as packet payload (not transport layer headers, etc)
    pub bytes: u32,
}

/// Diagnostic IDs for per-second replication diagnostics
pub mod replication_diagnostics {
    use super::*;
    /// How many entities modified per second by replication
    pub const REPLICATED_ENTITY_CHANGES_DIAGNOSTIC: DiagnosticId =
        DiagnosticId::from_u128(87359945710349305342211647237348);
    /// How many components modified per second by replication
    pub const REPLICATED_COMPONENT_CHANGES_DIAGNOSTIC: DiagnosticId =
        DiagnosticId::from_u128(36575095059706152186806005753628);
    /// How many client-mappings added per second by replication
    pub const REPLICATED_MAPPINGS_DIAGNOSTIC: DiagnosticId =
        DiagnosticId::from_u128(61564098061172206743744706749187);
    /// How many despawns per second from replication
    pub const REPLICATED_DESPAWNS_DIAGNOSTIC: DiagnosticId =
        DiagnosticId::from_u128(11043323327351349675112378115868);
    /// How many replication packets processed per second
    pub const REPLICATED_PACKETS_DIAGNOSTIC: DiagnosticId =
        DiagnosticId::from_u128(40094818756895929689855772983865);
    /// How many bytes of replication packets payloads per second
    pub const REPLICATED_BYTES_DIAGNOSTIC: DiagnosticId =
        DiagnosticId::from_u128(87998088176776397493423835383418);

    /// Diagnostic max_history_length
    pub const REPLICATED_DIAGNOSTIC_HISTORY_LEN: usize = 60;
}

use replication_diagnostics::*;

/// Clientside plugin to write Diagnostics every second
/// Not added by default.
///
/// See [`replication_diagnostics`]
#[derive(Default)]
pub(super) struct ReplicationStatsPlugin;

impl Plugin for ReplicationStatsPlugin {
    fn build(&self, app: &mut App) {
        let diagnostics_timer = Duration::from_millis(1000);
        app.add_systems(
            Update,
            write_diagnostics.run_if(on_timer(diagnostics_timer)),
        )
        .init_resource::<ReplicationStats>()
        .register_diagnostic(Diagnostic::new(
            REPLICATED_ENTITY_CHANGES_DIAGNOSTIC,
            "entities changed per second",
            REPLICATED_DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            REPLICATED_COMPONENT_CHANGES_DIAGNOSTIC,
            "components changed per second",
            REPLICATED_DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            REPLICATED_MAPPINGS_DIAGNOSTIC,
            "mappings added per second",
            REPLICATED_DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            REPLICATED_DESPAWNS_DIAGNOSTIC,
            "despawns per second",
            REPLICATED_DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            REPLICATED_PACKETS_DIAGNOSTIC,
            "packets per second",
            REPLICATED_DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            REPLICATED_BYTES_DIAGNOSTIC,
            "bytes per second",
            REPLICATED_DIAGNOSTIC_HISTORY_LEN,
        ));
    }
}

/// flushes aggregated stats to diagnostics system
fn write_diagnostics(mut stats: ResMut<ReplicationStats>, mut diagnostics: Diagnostics) {
    diagnostics.add_measurement(REPLICATED_ENTITY_CHANGES_DIAGNOSTIC, || {
        if stats.packets == 0 {
            0_f64
        } else {
            stats.entities_changed as f64 / stats.packets as f64
        }
    });
    diagnostics.add_measurement(REPLICATED_COMPONENT_CHANGES_DIAGNOSTIC, || {
        if stats.packets == 0 {
            0_f64
        } else {
            stats.components_changed as f64 / stats.packets as f64
        }
    });
    diagnostics.add_measurement(REPLICATED_MAPPINGS_DIAGNOSTIC, || {
        if stats.packets == 0 {
            0_f64
        } else {
            stats.mappings as f64 / stats.packets as f64
        }
    });
    diagnostics.add_measurement(REPLICATED_DESPAWNS_DIAGNOSTIC, || {
        if stats.packets == 0 {
            0_f64
        } else {
            stats.despawns as f64 / stats.packets as f64
        }
    });
    diagnostics.add_measurement(REPLICATED_BYTES_DIAGNOSTIC, || {
        if stats.packets == 0 {
            0_f64
        } else {
            stats.bytes as f64 / stats.packets as f64
        }
    });
    diagnostics.add_measurement(REPLICATED_PACKETS_DIAGNOSTIC, || stats.packets as f64);
    *stats = ReplicationStats::default();
}
