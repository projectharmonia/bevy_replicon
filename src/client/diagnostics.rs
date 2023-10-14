use bevy::diagnostic::DiagnosticId;
use bevy::{
    diagnostic::{Diagnostic, Diagnostics, RegisterDiagnostic},
    prelude::*,
    time::common_conditions::on_timer,
};
use std::time::Duration;

/// Replication stats during packet processing.
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
    /// Replication packets received.
    pub packets: u32,
    /// Replication bytes received in packet payloads (without internal Renet data).
    pub bytes: u64,
}

/// Plugin to write Diagnostics every second.
///
/// Not added by default.
pub struct ClientDiagnosticsPlugin;

impl Plugin for ClientDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        let diagnostics_timer = Duration::from_millis(1000);
        app.add_systems(
            Update,
            Self::diagnostic_system.run_if(on_timer(diagnostics_timer)),
        )
        .init_resource::<ClientStats>()
        .register_diagnostic(Diagnostic::new(
            Self::ENTITY_CHANGES,
            "entities changed per second",
            Self::DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            Self::COMPONENT_CHANGES,
            "components changed per second",
            Self::DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            Self::MAPPINGS,
            "mappings added per second",
            Self::DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            Self::DESPAWNS,
            "despawns per second",
            Self::DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            Self::PACKETS,
            "packets per second",
            Self::DIAGNOSTIC_HISTORY_LEN,
        ))
        .register_diagnostic(Diagnostic::new(
            Self::BYTES,
            "bytes per second",
            Self::DIAGNOSTIC_HISTORY_LEN,
        ));
    }
}

impl ClientDiagnosticsPlugin {
    /// How many entities modified per second by replication.
    pub const ENTITY_CHANGES: DiagnosticId =
        DiagnosticId::from_u128(87359945710349305342211647237348);
    /// How many components modified per second by replication.
    pub const COMPONENT_CHANGES: DiagnosticId =
        DiagnosticId::from_u128(36575095059706152186806005753628);
    /// How many client-mappings added per second by replication.
    pub const MAPPINGS: DiagnosticId = DiagnosticId::from_u128(61564098061172206743744706749187);
    /// How many despawns per second from replication.
    pub const DESPAWNS: DiagnosticId = DiagnosticId::from_u128(11043323327351349675112378115868);
    /// How many replication packets processed per second.
    pub const PACKETS: DiagnosticId = DiagnosticId::from_u128(40094818756895929689855772983865);
    /// How many bytes of replication packets payloads per second.
    pub const BYTES: DiagnosticId = DiagnosticId::from_u128(87998088176776397493423835383418);

    /// Max diagnostic history length.
    pub const DIAGNOSTIC_HISTORY_LEN: usize = 60;

    fn diagnostic_system(mut stats: ResMut<ClientStats>, mut diagnostics: Diagnostics) {
        diagnostics.add_measurement(Self::ENTITY_CHANGES, || {
            if stats.packets == 0 {
                0_f64
            } else {
                stats.entities_changed as f64 / stats.packets as f64
            }
        });
        diagnostics.add_measurement(Self::COMPONENT_CHANGES, || {
            if stats.packets == 0 {
                0_f64
            } else {
                stats.components_changed as f64 / stats.packets as f64
            }
        });
        diagnostics.add_measurement(Self::MAPPINGS, || {
            if stats.packets == 0 {
                0_f64
            } else {
                stats.mappings as f64 / stats.packets as f64
            }
        });
        diagnostics.add_measurement(Self::DESPAWNS, || {
            if stats.packets == 0 {
                0_f64
            } else {
                stats.despawns as f64 / stats.packets as f64
            }
        });
        diagnostics.add_measurement(Self::BYTES, || {
            if stats.packets == 0 {
                0_f64
            } else {
                stats.bytes as f64 / stats.packets as f64
            }
        });
        diagnostics.add_measurement(Self::PACKETS, || stats.packets as f64);
        *stats = ClientStats::default();
    }
}
