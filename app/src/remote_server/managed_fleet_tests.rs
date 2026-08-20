use super::*;
use crate::remote_server::fleet_memory::{MemoryDiagnostic, MemoryMeasurement, MemoryProvenance};

fn measured_host(bytes: u64, collected_at: u64) -> HostMemorySnapshot {
    HostMemorySnapshot {
        available: MemoryMeasurement::measured(bytes, MemoryProvenance::LinuxProcMemAvailable),
        collected_at_epoch_millis: collected_at,
    }
}

#[test]
fn launch_key_is_exact_across_host_account_project_and_provider() {
    let key = ManagedLaunchKey::new("devhost", "opaque-a", "/srv/a", "Claude").unwrap();
    let different_account =
        ManagedLaunchKey::new("devhost", "opaque-b", "/srv/a", "claude").unwrap();
    let different_project =
        ManagedLaunchKey::new("devhost", "opaque-a", "/srv/b", "claude").unwrap();

    assert_eq!(key.host_id(), "devhost");
    assert_eq!(key.account_id(), "opaque-a");
    assert_eq!(key.project_root(), "/srv/a");
    assert_eq!(key.provider(), "claude");
    assert_ne!(key, different_account);
    assert_ne!(key, different_project);
}

#[test]
fn identity_rejects_ambiguous_or_stale_action_targets() {
    let key = ManagedLaunchKey::new("devhost", "opaque-a", "/srv/a", "claude").unwrap();
    let identity = ManagedFleetIdentity::new(key.clone(), "pty-1", 7).unwrap();

    assert!(identity.matches_action(&key, "pty-1", 7));
    assert!(!identity.matches_action(&key, "pty-1", 8));
    assert!(!identity.matches_action(&key, "pty-2", 7));
    assert_eq!(identity.launch_key(), &key);
    assert_eq!(identity.session_id(), "pty-1");
    assert_eq!(identity.generation(), 7);
    assert_eq!(
        ManagedFleetIdentity::new(key, "pty-1", 0),
        Err(FleetValidationError::ZeroGeneration)
    );
}

#[test]
fn opaque_identity_components_reject_controls_and_empty_values() {
    assert_eq!(
        ManagedLaunchKey::new("", "opaque", "/srv/a", "claude"),
        Err(FleetValidationError::EmptyComponent)
    );
    assert_eq!(
        ManagedLaunchKey::new("devhost", "opaque\nleak", "/srv/a", "claude"),
        Err(FleetValidationError::ControlCharacter)
    );
    assert_eq!(
        ManagedLaunchKey::new("devhost", "opaque", "/srv/a", "unknown"),
        Err(FleetValidationError::UnsupportedProvider)
    );
}

#[test]
fn claude_remote_control_builds_documented_argv_without_a_shell_prefix() {
    let spec = ClaudeRemoteControlSpec::new(
        ClaudeSpawnMode::Session,
        64,
        Some(ClaudePermissionMode::Plan),
        Some("Project Alpha"),
    )
    .unwrap();

    assert_eq!(
        spec.argv(),
        vec![
            "claude",
            "remote-control",
            "--spawn",
            "session",
            "--capacity",
            "64",
            "--permission-mode",
            "plan",
            "--name",
            "Project Alpha",
        ]
    );
    assert_eq!(
        ManagedLaunchKind::ClaudeRemoteControl.protocol_name(),
        "claude-remote-control"
    );
    assert_eq!(
        ManagedLaunchKind::InteractiveAgent.protocol_name(),
        "interactive-agent"
    );
}

#[test]
fn display_name_remains_one_argv_value_even_when_shell_sensitive() {
    let spec =
        ClaudeRemoteControlSpec::new(ClaudeSpawnMode::SameDir, 1, None, Some("a'; printenv; 'b"))
            .unwrap();

    let argv = spec.argv();
    assert_eq!(argv.last().map(String::as_str), Some("a'; printenv; 'b"));
    assert_eq!(argv.len(), 8);

    let command = String::from_utf8(spec.startup_command(ShellFamily::Posix)).unwrap();
    assert_eq!(
        shell_words::split(command.trim()).unwrap(),
        spec.argv(),
        "shell parsing must recover the exact original argv boundaries"
    );
}

#[test]
fn remote_control_rejects_invalid_capacity_and_wrong_provider() {
    assert_eq!(
        ClaudeRemoteControlSpec::new(ClaudeSpawnMode::Worktree, 0, None, None),
        Err(FleetValidationError::InvalidCapacity)
    );
    assert_eq!(
        ClaudeRemoteControlSpec::new(ClaudeSpawnMode::Worktree, 257, None, None),
        Err(FleetValidationError::InvalidCapacity)
    );
    assert_eq!(
        ManagedLaunchKind::ClaudeRemoteControl.validate_provider("codex"),
        Err(FleetValidationError::ProviderMismatch)
    );
    assert_eq!(
        ManagedLaunchKind::ClaudeRemoteControl.validate_provider("CLAUDE"),
        Ok(())
    );
}

#[test]
fn launch_retry_requires_the_same_stable_id_route_and_configuration() {
    let key = ManagedLaunchKey::new("devhost", "opaque-a", "/srv/a", "claude").unwrap();
    let plan = ManagedLaunchPlan::claude_remote_control(
        "launch-1",
        key.clone(),
        ClaudeRemoteControlSpec::default(),
    )
    .unwrap();
    let retry = plan.clone();
    let changed_config = ManagedLaunchPlan::claude_remote_control(
        "launch-1",
        key,
        ClaudeRemoteControlSpec::new(ClaudeSpawnMode::Session, 32, None, None).unwrap(),
    )
    .unwrap();

    assert!(plan.is_retry_of(&retry));
    assert!(!plan.is_retry_of(&changed_config));
    assert_eq!(plan.launch_id(), "launch-1");
    assert_eq!(plan.kind(), ManagedLaunchKind::ClaudeRemoteControl);
    assert_eq!(plan.claude_spec().unwrap().argv()[0], "claude");
}

#[cfg(unix)]
#[test]
fn stored_project_identity_rejects_a_replacement_at_the_same_path() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let displaced = temp.path().join("project-old");
    std::fs::create_dir(&project).unwrap();
    let identity = ManagedProjectIdentity::capture(&project).unwrap();
    let key =
        ManagedLaunchKey::new("devhost", "opaque-a", project.to_str().unwrap(), "claude").unwrap();
    let plan = ManagedLaunchPlan::interactive_agent("launch-1", key)
        .unwrap()
        .with_project_identity(identity);

    assert!(plan.project_identity_is_current());
    std::fs::rename(&project, &displaced).unwrap();
    std::fs::create_dir(&project).unwrap();

    assert!(!plan.project_identity_is_current());
}

#[test]
fn managed_interactive_launch_is_provider_neutral_for_claude_and_codex() {
    for provider in ["claude", "codex"] {
        let key = ManagedLaunchKey::new("devhost", "opaque-a", "/srv/a", provider).unwrap();
        let plan = ManagedLaunchPlan::interactive_agent("launch-1", key).unwrap();
        assert_eq!(plan.kind(), ManagedLaunchKind::InteractiveAgent);
        assert_eq!(plan.claude_spec(), None);
        assert_eq!(
            String::from_utf8(plan.startup_command(ShellFamily::Posix)).unwrap(),
            format!("{provider}\n")
        );
    }
}

#[test]
fn managed_session_metadata_is_an_explicit_keepalive_without_credentials() {
    let key = ManagedLaunchKey::new("devhost", "opaque-a", "/srv/a", "claude").unwrap();
    let plan = ManagedLaunchPlan::claude_remote_control(
        "launch-1",
        key,
        ClaudeRemoteControlSpec::default(),
    )
    .unwrap();
    let process_root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };
    let metadata = ManagedSessionMetadata::new(plan, Some(process_root));

    assert!(metadata.is_keepalive());
    assert_eq!(metadata.plan().launch_id(), "launch-1");
    assert_eq!(metadata.process_root(), Some(process_root));
    let debug = format!("{metadata:?}");
    assert!(!debug.contains("CLAUDE_CONFIG_DIR"));
    assert!(!debug.contains("ANTHROPIC_API_KEY"));
}

#[test]
fn managed_session_is_exempt_from_both_implicit_gc_paths() {
    let key = ManagedLaunchKey::new("devhost", "opaque-a", "/srv/a", "claude").unwrap();
    let plan = ManagedLaunchPlan::claude_remote_control(
        "launch-1",
        key,
        ClaudeRemoteControlSpec::default(),
    )
    .unwrap();
    let managed = ManagedSessionMetadata::new(plan, None);

    assert!(!eligible_for_detached_age_gc(
        Some(&managed),
        true,
        100_000,
        1,
        60_000,
    ));
    assert!(!eligible_for_ring_pressure_gc(Some(&managed), true));
}

#[test]
fn unmanaged_session_keeps_existing_detached_gc_semantics() {
    assert!(eligible_for_detached_age_gc(None, true, 100_000, 1, 60_000,));
    assert!(!eligible_for_detached_age_gc(
        None, false, 100_000, 1, 60_000,
    ));
    assert!(eligible_for_ring_pressure_gc(None, true));
    assert!(!eligible_for_ring_pressure_gc(None, false));
}

#[test]
fn stricter_client_floor_can_raise_but_never_lower_daemon_policy() {
    let daemon_wins = HeadroomPolicy::new(4_000, Some(2_000), 5_000).unwrap();
    let client_raises = HeadroomPolicy::new(4_000, Some(8_000), 5_000).unwrap();

    assert_eq!(daemon_wins.effective_floor_bytes(), 4_000);
    assert_eq!(client_raises.effective_floor_bytes(), 8_000);
    assert_eq!(
        HeadroomPolicy::new(0, None, 5_000),
        Err(FleetValidationError::InvalidHeadroom)
    );
}

#[test]
fn daemon_headroom_configuration_is_strict_and_overflow_safe() {
    assert_eq!(
        daemon_headroom_floor_bytes(None),
        Ok(2 * 1024 * 1024 * 1024)
    );
    assert_eq!(
        daemon_headroom_floor_bytes(Some("3072")),
        Ok(3 * 1024 * 1024 * 1024)
    );
    assert_eq!(
        daemon_headroom_floor_bytes(Some("0")),
        Err(FleetValidationError::InvalidHeadroom)
    );
    assert_eq!(
        daemon_headroom_floor_bytes(Some("2GiB")),
        Err(FleetValidationError::InvalidHeadroom)
    );
    assert_eq!(
        daemon_headroom_floor_bytes(Some(&u64::MAX.to_string())),
        Err(FleetValidationError::InvalidHeadroom)
    );
}

#[test]
fn headroom_gate_allows_threshold_and_blocks_below_it() {
    let policy = HeadroomPolicy::new(4_000, None, 5_000).unwrap();

    assert_eq!(
        evaluate_headroom(policy, &measured_host(4_000, 10_000), 10_001),
        HeadroomDecision::Allowed {
            available_bytes: 4_000,
            required_bytes: 4_000,
        }
    );
    assert_eq!(
        evaluate_headroom(policy, &measured_host(3_999, 10_000), 10_001),
        HeadroomDecision::Denied {
            reason: HeadroomDenialReason::BelowFloor,
            available_bytes: Some(3_999),
            required_bytes: 4_000,
        }
    );
}

#[test]
fn unavailable_unsupported_stale_and_future_measurements_fail_closed() {
    let policy = HeadroomPolicy::new(4_000, None, 5_000).unwrap();
    let unavailable = HostMemorySnapshot {
        available: MemoryMeasurement::unavailable(
            MemoryProvenance::LinuxProcMemAvailable,
            MemoryDiagnostic::ReadFailed,
        ),
        collected_at_epoch_millis: 10_000,
    };
    let unsupported = HostMemorySnapshot {
        available: MemoryMeasurement::unsupported(),
        collected_at_epoch_millis: 10_000,
    };

    assert_eq!(
        evaluate_headroom(policy, &unavailable, 10_001),
        HeadroomDecision::Denied {
            reason: HeadroomDenialReason::Unavailable,
            available_bytes: None,
            required_bytes: 4_000,
        }
    );
    assert_eq!(
        evaluate_headroom(policy, &unsupported, 10_001),
        HeadroomDecision::Denied {
            reason: HeadroomDenialReason::Unsupported,
            available_bytes: None,
            required_bytes: 4_000,
        }
    );
    assert_eq!(
        evaluate_headroom(policy, &measured_host(8_000, 1), 10_001),
        HeadroomDecision::Denied {
            reason: HeadroomDenialReason::Stale,
            available_bytes: Some(8_000),
            required_bytes: 4_000,
        }
    );
    assert_eq!(
        evaluate_headroom(policy, &measured_host(8_000, 10_002), 10_001),
        HeadroomDecision::Denied {
            reason: HeadroomDenialReason::FutureDated,
            available_bytes: Some(8_000),
            required_bytes: 4_000,
        }
    );
}

#[test]
fn fixed_error_codes_cannot_echo_credentials() {
    for error in [
        FleetValidationError::EmptyComponent,
        FleetValidationError::ComponentTooLong,
        FleetValidationError::ControlCharacter,
        FleetValidationError::ZeroGeneration,
        FleetValidationError::InvalidCapacity,
        FleetValidationError::InvalidHeadroom,
        FleetValidationError::ProviderMismatch,
        FleetValidationError::UnsupportedProvider,
    ] {
        let code = error.protocol_code();
        assert!(!code.contains("token"));
        assert!(!code.contains("CLAUDE_CONFIG_DIR"));
        assert!(!code.contains('/'));
    }
    for denial in [
        HeadroomDenialReason::BelowFloor,
        HeadroomDenialReason::Unavailable,
        HeadroomDenialReason::Unsupported,
        HeadroomDenialReason::WrongProvenance,
        HeadroomDenialReason::Stale,
        HeadroomDenialReason::FutureDated,
    ] {
        assert!(!denial.protocol_code().contains('/'));
    }
}

#[test]
fn every_supported_permission_mode_maps_to_one_documented_value() {
    let expected = [
        (ClaudePermissionMode::AcceptEdits, "acceptEdits"),
        (ClaudePermissionMode::Auto, "auto"),
        (ClaudePermissionMode::BypassPermissions, "bypassPermissions"),
        (ClaudePermissionMode::Default, "default"),
        (ClaudePermissionMode::DontAsk, "dontAsk"),
        (ClaudePermissionMode::Plan, "plan"),
    ];
    for (mode, value) in expected {
        let spec =
            ClaudeRemoteControlSpec::new(ClaudeSpawnMode::Worktree, 32, Some(mode), None).unwrap();
        assert_eq!(spec.argv().last().map(String::as_str), Some(value));
    }
}
