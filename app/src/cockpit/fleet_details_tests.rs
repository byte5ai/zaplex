use super::*;
use crate::remote_server::fleet_memory::{MemoryDiagnostic, MemoryMeasurement};
use remote_server::proto::{
    AgentAccountInfo, AgentAccountInventory, ManagedSessionExitInfo, ManagedSessionInfo,
    MemoryMeasurement as ProtoMemoryMeasurement,
    MemoryMeasurementStatus as ProtoMemoryMeasurementStatus, SessionInfo, SessionList,
};

fn host(measurement: MemoryMeasurement) -> HostMemorySnapshot {
    HostMemorySnapshot {
        available: measurement,
        collected_at_epoch_millis: 10_000,
    }
}

#[test]
fn sidebar_projection_contains_only_a_managed_marker() {
    assert_eq!(sidebar_marker(true), Some(FleetSidebarMarker::Managed));
    assert_eq!(sidebar_marker(false), None);
}

#[test]
fn compact_details_keep_pss_and_headroom_in_main_pane() {
    let process = ProcessMemorySnapshot {
        pss: MemoryMeasurement::measured(384 * MIB, MemoryProvenance::LinuxProcSmapsRollup),
        collected_at_epoch_millis: 10_000,
    };
    let host = host(MemoryMeasurement::measured(
        6 * GIB + GIB / 2,
        MemoryProvenance::LinuxProcMemAvailable,
    ));
    let details = managed_fleet_details(
        Some(&process),
        &host,
        HeadroomPolicy::new(2 * GIB, None, 5_000).unwrap(),
        10_001,
    );

    assert_eq!(details.process_memory.value, "384 MB");
    assert_eq!(details.process_memory.hint, "PSS · Linux");
    assert_eq!(details.host_headroom.value, "6.5 GB");
    assert_eq!(details.host_headroom.hint, "MemAvailable · min. 2.0 GB");
    assert!(!details.launch_blocked);
}

#[test]
fn missing_measurement_is_an_em_dash_not_zero() {
    let host = host(MemoryMeasurement::unavailable(
        MemoryProvenance::LinuxProcMemAvailable,
        MemoryDiagnostic::ReadFailed,
    ));
    let details = managed_fleet_details(
        None,
        &host,
        HeadroomPolicy::new(2 * GIB, None, 5_000).unwrap(),
        10_001,
    );

    assert_eq!(details.process_memory.value, "—");
    assert_eq!(details.host_headroom.value, "—");
    assert_eq!(details.host_headroom.health, FleetDetailHealth::Degraded);
    assert!(details.launch_blocked);
}

#[test]
fn measured_below_floor_is_visible_and_blocks_launch() {
    let host = host(MemoryMeasurement::measured(
        GIB,
        MemoryProvenance::LinuxProcMemAvailable,
    ));
    let details = managed_fleet_details(
        None,
        &host,
        HeadroomPolicy::new(2 * GIB, None, 5_000).unwrap(),
        10_001,
    );

    assert_eq!(details.host_headroom.value, "1.0 GB");
    assert_eq!(details.host_headroom.hint, "Minimum 2.0 GB");
    assert_eq!(details.host_headroom.health, FleetDetailHealth::Blocked);
    assert!(details.launch_blocked);
}

#[test]
fn unsupported_platform_is_degraded_and_not_rendered_as_zero() {
    let details = managed_fleet_details(
        None,
        &host(MemoryMeasurement::unsupported()),
        HeadroomPolicy::default(),
        10_001,
    );

    assert_eq!(details.host_headroom.value, "—");
    assert_eq!(details.host_headroom.hint, "Nicht verfügbar");
    assert!(details.launch_blocked);
}

#[test]
fn stale_process_pss_is_not_presented_as_current() {
    let process = ProcessMemorySnapshot {
        pss: MemoryMeasurement::measured(384 * MIB, MemoryProvenance::LinuxProcSmapsRollup),
        collected_at_epoch_millis: 1,
    };
    let host = host(MemoryMeasurement::measured(
        4 * GIB,
        MemoryProvenance::LinuxProcMemAvailable,
    ));

    let details = managed_fleet_details(
        Some(&process),
        &host,
        HeadroomPolicy::new(2 * GIB, None, 5_000).unwrap(),
        10_001,
    );

    assert_eq!(details.process_memory.value, "—");
    assert_eq!(details.process_memory.hint, "Messung veraltet");
}

fn managed_session(generation: u64) -> SessionInfo {
    SessionInfo {
        session_id: "pty-42".to_string(),
        generation,
        managed: Some(ManagedSessionInfo {
            schema_version: 1,
            provider: "claude".to_string(),
            account_id: "opaque-account".to_string(),
            project_root: "/srv/zaplex".to_string(),
            launch_kind: "interactive-agent".to_string(),
            launch_id: "launch-9".to_string(),
            generation,
        }),
        process_memory: Some(ProtoMemoryMeasurement {
            status: ProtoMemoryMeasurementStatus::Measured.into(),
            bytes: Some(384 * MIB),
            provenance: "linux-proc-smaps-rollup".to_string(),
            diagnostic_code: String::new(),
        }),
        ..Default::default()
    }
}

fn managed_exit(generation: u64) -> ManagedSessionExitInfo {
    ManagedSessionExitInfo {
        managed: managed_session(generation).managed,
        session_id: "pty-ended".to_string(),
        exit_code: Some(1),
        exited_at_epoch_millis: 9_000,
        diagnostic_code: "process-ended".to_string(),
    }
}

fn stopped_managed_exit(generation: u64) -> ManagedSessionExitInfo {
    ManagedSessionExitInfo {
        diagnostic_code: "stopped".to_string(),
        ..managed_exit(generation)
    }
}

#[test]
fn managed_inventory_retains_only_exact_current_wire_identities() {
    let mut inventory = ManagedFleetInventory::default();
    let mut stale = managed_session(3);
    stale.managed.as_mut().unwrap().generation = 2;
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            sessions: vec![managed_session(3), stale],
            host_available_memory: Some(ProtoMemoryMeasurement {
                status: ProtoMemoryMeasurementStatus::Measured.into(),
                bytes: Some(6 * GIB),
                provenance: "linux-proc-memavailable".to_string(),
                diagnostic_code: String::new(),
            }),
            daemon_min_available_bytes: 2 * GIB,
            collected_at_epoch_millis: 10_000,
            ..Default::default()
        },
    );

    assert_eq!(inventory.sessions().len(), 1);
    let target = inventory.sessions()[0].clone();
    assert_eq!(inventory.exact(&target), Some(&target));
    let mut replaced = target.clone();
    replaced.generation += 1;
    assert_eq!(inventory.exact(&replaced), None);
}

#[test]
fn unexpected_managed_exit_remains_restartable_but_never_attachable() {
    let mut inventory = ManagedFleetInventory::default();
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            recent_managed_exits: vec![managed_exit(3)],
            host_available_memory: Some(ProtoMemoryMeasurement {
                status: ProtoMemoryMeasurementStatus::Measured.into(),
                bytes: Some(6 * GIB),
                provenance: "linux-proc-memavailable".to_string(),
                diagnostic_code: String::new(),
            }),
            daemon_min_available_bytes: 2 * GIB,
            collected_at_epoch_millis: 10_000,
            ..Default::default()
        },
    );

    let target = inventory.sessions().first().expect("projected exit");
    assert!(!target.is_running());
    assert_eq!(
        target.exit.as_ref().and_then(|exit| exit.exit_code),
        Some(1)
    );
    assert_eq!(inventory.exact(target), Some(target));
    assert_eq!(inventory.exact_live(target), None);
    assert!(!managed_fleet_details_from_proto_at(target, 10_001).launch_blocked);
}

#[test]
fn explicitly_stopped_managed_session_is_distinct_and_restartable() {
    let mut inventory = ManagedFleetInventory::default();
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            recent_managed_exits: vec![stopped_managed_exit(3)],
            ..Default::default()
        },
    );

    let target = inventory.sessions().first().expect("projected stop");
    assert!(!target.is_running());
    assert_eq!(target.exit.as_ref().unwrap().state_label(), "Stopped");
    assert_eq!(inventory.exact_live(target), None);
}

#[test]
fn fresh_remote_account_inventory_labels_an_ended_managed_session() {
    let mut inventory = ManagedFleetInventory::default();
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            recent_managed_exits: vec![managed_exit(3)],
            ..Default::default()
        },
    );
    inventory.enrich_remote_account_labels(
        "host-1",
        &AgentAccountInventory {
            schema_version: 1,
            health: "loaded".to_string(),
            accounts: vec![AgentAccountInfo {
                provider: "claude".to_string(),
                account_id: "opaque-account".to_string(),
                display_label: "Claude · owner@example.test".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    assert_eq!(
        inventory.sessions()[0].account_label.as_deref(),
        Some("Claude · owner@example.test")
    );
    assert_ne!(
        inventory.sessions()[0].account_label.as_deref(),
        Some("opaque-account")
    );
}

#[test]
fn ambiguous_remote_account_labels_fail_closed() {
    let mut inventory = ManagedFleetInventory::default();
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            sessions: vec![managed_session(3)],
            ..Default::default()
        },
    );
    let account = AgentAccountInfo {
        provider: "claude".to_string(),
        account_id: "opaque-account".to_string(),
        display_label: "One".to_string(),
        ..Default::default()
    };
    inventory.enrich_remote_account_labels(
        "host-1",
        &AgentAccountInventory {
            schema_version: 1,
            health: "loaded".to_string(),
            accounts: vec![
                account.clone(),
                AgentAccountInfo {
                    display_label: "Two".to_string(),
                    ..account
                },
            ],
            ..Default::default()
        },
    );

    assert_eq!(inventory.sessions()[0].account_label, None);
}

#[test]
fn live_replacement_suppresses_matching_exit_and_invalid_exit_is_dropped() {
    let mut invalid = managed_exit(3);
    invalid.diagnostic_code = "raw secret diagnostic".to_string();
    let mut inventory = ManagedFleetInventory::default();
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            sessions: vec![managed_session(3)],
            recent_managed_exits: vec![managed_exit(3), invalid],
            ..Default::default()
        },
    );

    assert_eq!(inventory.sessions().len(), 1);
    assert!(inventory.sessions()[0].is_running());
}

#[test]
fn proto_details_preserve_provenance_and_never_turn_unknown_into_zero() {
    let mut inventory = ManagedFleetInventory::default();
    let mut session = managed_session(3);
    session.process_memory = None;
    inventory.extend_session_list(
        "host-1",
        "devhost",
        Some("node-1"),
        SessionList {
            sessions: vec![session],
            host_available_memory: None,
            daemon_min_available_bytes: 2 * GIB,
            ..Default::default()
        },
    );
    let details = managed_fleet_details_from_proto(&inventory.sessions()[0]);
    assert_eq!(details.process_memory.value, "—");
    assert_eq!(details.host_headroom.value, "—");
    assert!(details.launch_blocked);
}

#[test]
fn stale_proto_details_degrade_and_block_restart() {
    let mut inventory = ManagedFleetInventory::default();
    inventory.extend_session_list(
        "host-stale",
        "devhost",
        Some("node-stale"),
        SessionList {
            sessions: vec![managed_session(3)],
            host_available_memory: Some(ProtoMemoryMeasurement {
                status: ProtoMemoryMeasurementStatus::Measured.into(),
                bytes: Some(6 * GIB),
                provenance: "linux-proc-memavailable".to_string(),
                diagnostic_code: String::new(),
            }),
            daemon_min_available_bytes: 2 * GIB,
            collected_at_epoch_millis: 10_000,
            ..Default::default()
        },
    );
    let details = managed_fleet_details_from_proto_at(&inventory.sessions()[0], 20_000);
    assert_eq!(details.process_memory.value, "—");
    assert_eq!(details.host_headroom.hint, "Messung veraltet");
    assert!(details.launch_blocked);
}
