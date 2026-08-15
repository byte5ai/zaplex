use super::*;
use clap::Parser;

#[derive(Debug, Parser)]
struct TestControlCli {
    #[command(subcommand)]
    command: ControlCommand,
}

fn auth() -> ControlAuth {
    ControlAuth {
        token: "secret".to_string(),
        caller_surface_id: "surface-a".to_string(),
        caller_tab_id: "tab-a".to_string(),
    }
}

#[test]
fn every_public_control_verb_is_strictly_typed() {
    let requests = [
        ControlRequest::from_cli(
            auth(),
            ControlCommand::SplitPane(SplitPaneArgs {
                orientation: ControlOrientation::Right,
                dir: Some(PathBuf::from("/workspace")),
            }),
        )
        .unwrap(),
        ControlRequest::from_cli(
            auth(),
            ControlCommand::OpenWorktreeInPane(OpenWorktreeInPaneArgs {
                repo: PathBuf::from("/repo"),
                branch: "feature/control".to_string(),
            }),
        )
        .unwrap(),
        ControlRequest::from_cli(
            auth(),
            ControlCommand::FocusSession(FocusSessionArgs {
                surface_id: Some("surface-b".to_string()),
                host: None,
                session_id: None,
            }),
        )
        .unwrap(),
        ControlRequest::from_cli(
            auth(),
            ControlCommand::SendText(SendTextArgs {
                surface_id: "surface-b".to_string(),
                text: "cargo test".to_string(),
                submit: true,
            }),
        )
        .unwrap(),
    ];

    assert!(matches!(requests[0].verb, ControlVerb::SplitPane { .. }));
    assert!(matches!(
        requests[1].verb,
        ControlVerb::OpenWorktreeInPane { .. }
    ));
    assert!(matches!(requests[2].verb, ControlVerb::FocusSession { .. }));
    assert!(matches!(requests[3].verb, ControlVerb::SendText { .. }));
}

#[test]
fn all_control_verbs_parse_from_the_public_cli_shape() {
    let commands = [
        TestControlCli::try_parse_from([
            "control",
            "split-pane",
            "--orientation",
            "left",
            "--dir",
            "/workspace",
        ])
        .unwrap()
        .command,
        TestControlCli::try_parse_from([
            "control",
            "open-worktree-in-pane",
            "--repo",
            "/repo",
            "--branch",
            "feature/control",
        ])
        .unwrap()
        .command,
        TestControlCli::try_parse_from([
            "control",
            "focus-session",
            "--host",
            "devhost",
            "--session-id",
            "session-a",
        ])
        .unwrap()
        .command,
        TestControlCli::try_parse_from([
            "control",
            "send-text",
            "--surface-id",
            "surface-a",
            "--text",
            "continue",
            "--submit",
        ])
        .unwrap()
        .command,
    ];

    assert!(matches!(commands[0], ControlCommand::SplitPane(_)));
    assert!(matches!(commands[1], ControlCommand::OpenWorktreeInPane(_)));
    assert!(matches!(commands[2], ControlCommand::FocusSession(_)));
    assert!(matches!(commands[3], ControlCommand::SendText(_)));
}

#[test]
fn zaplex_control_is_wired_into_the_top_level_cli() {
    let args = crate::Args::try_parse_from(["zaplex", "control", "split-pane"]).unwrap();

    assert!(matches!(
        args.command(),
        Some(crate::Command::CommandLine(command))
            if matches!(command.as_ref(), crate::CliCommand::Control(ControlCommand::SplitPane(_)))
    ));
}

#[test]
fn malformed_focus_and_oversized_payloads_fail_closed() {
    assert!(ControlRequest::from_cli(
        auth(),
        ControlCommand::FocusSession(FocusSessionArgs {
            surface_id: Some("surface-b".to_string()),
            host: Some("host-a".to_string()),
            session_id: Some("session-a".to_string()),
        }),
    )
    .is_err());

    assert!(ControlRequest::from_cli(
        auth(),
        ControlCommand::FocusSession(FocusSessionArgs {
            surface_id: None,
            host: None,
            session_id: None,
        }),
    )
    .is_err());

    assert!(ControlRequest::from_cli(
        auth(),
        ControlCommand::SendText(SendTextArgs {
            surface_id: String::new(),
            text: "safe".to_string(),
            submit: false,
        }),
    )
    .is_err());

    assert!(ControlRequest::from_cli(
        auth(),
        ControlCommand::SendText(SendTextArgs {
            surface_id: "surface-b".to_string(),
            text: "x".repeat(MAX_TEXT_BYTES + 1),
            submit: false,
        }),
    )
    .is_err());
}

#[test]
fn hook_forwarding_uses_the_same_authenticated_surface_binding() {
    let request =
        ControlRequest::hook_event(auth(), r#"{"v":1,"event":"stop"}"#.to_string()).unwrap();

    assert_eq!(request.auth.caller_surface_id, "surface-a");
    assert_eq!(request.auth.caller_tab_id, "tab-a");
    assert!(matches!(request.verb, ControlVerb::HookEvent { .. }));
}

#[test]
fn deserialized_requests_are_revalidated_at_the_server_boundary() {
    let invalid = ControlRequest {
        version: CONTROL_PROTOCOL_VERSION,
        auth: auth(),
        verb: ControlVerb::SendText {
            surface_id: String::new(),
            text: "ignored".to_string(),
            submit: false,
        },
    };

    assert!(invalid.validate().is_err());
}
