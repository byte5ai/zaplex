use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use tempfile::TempDir;
use warp_cli::control::{ControlAuth, ControlRequest, ControlVerb};

use super::{
    antigravity_title_backup_path, has_managed_antigravity_title, has_managed_deepseek_hooks,
    has_managed_hooks, hook_paths, is_managed_hook_handler, normalize_antigravity_status_input,
    normalize_deepseek_hook_environment, normalize_hook_input, refresh_antigravity_title,
    refresh_codex_hooks, refresh_deepseek_hooks, refresh_deepseek_hooks_with_activation,
    refresh_hooks, remove_antigravity_title, remove_codex_hooks, remove_deepseek_hooks,
    remove_hooks, resolve_worker_agent, supports_agent, HookConfigChange,
    CLAUDE_MANAGED_HOOK_EVENTS, CODEX_MANAGED_HOOK_EVENTS, DEEPSEEK_MANAGED_HOOK_EVENTS,
    GROK_MANAGED_HOOK_EVENTS, MANAGED_BY_MARKER,
};
use crate::terminal::CLIAgent;

fn parse_normalized(agent: CLIAgent, input: Value) -> Value {
    let bytes = serde_json::to_vec(&input).unwrap();
    serde_json::from_slice(&normalize_hook_input(agent, &bytes).unwrap()).unwrap()
}

#[test]
fn hook_bridge_exposes_only_agents_with_managed_native_integrations() {
    for agent in [
        CLIAgent::Claude,
        CLIAgent::Codex,
        CLIAgent::Grok,
        CLIAgent::DeepSeek,
        CLIAgent::Antigravity,
    ] {
        assert!(supports_agent(agent), "{agent:?} should be supported");
    }

    for agent in [
        CLIAgent::Gemini,
        CLIAgent::Amp,
        CLIAgent::Droid,
        CLIAgent::OpenCode,
        CLIAgent::Copilot,
        CLIAgent::Pi,
        CLIAgent::Auggie,
        CLIAgent::CursorCli,
        CLIAgent::Goose,
        CLIAgent::Unknown,
    ] {
        assert!(!supports_agent(agent), "{agent:?} should not be supported");
    }
}

#[cfg(not(windows))]
#[test]
fn managed_hook_command_shell_quotes_spaces_and_apostrophes() {
    let executable = Path::new("/opt/Zaplex's App/zaplex");
    let command =
        super::managed_hook_command(executable, warp_cli::CliAgentHookAgent::Codex).unwrap();
    let tokens = shell_words::split(&command).unwrap();

    assert_eq!(tokens[0], executable.to_str().unwrap());
    assert!(super::is_managed_hook_command(
        &command,
        Some(warp_cli::CliAgentHookAgent::Codex)
    ));
}

#[test]
fn normalizes_supported_native_events_to_control_v1_event_names() {
    for (native, normalized) in [
        ("SessionStart", "session_start"),
        ("PreToolUse", "pre_tool_use"),
        ("PermissionRequest", "permission_request"),
        ("PostToolUse", "tool_complete"),
        ("UserPromptSubmit", "prompt_submit"),
        ("Stop", "stop"),
        ("SessionEnd", "session_end"),
    ] {
        let event = parse_normalized(
            CLIAgent::Codex,
            json!({
                "session_id": "thr_123",
                "cwd": "/workspace",
                "hook_event_name": native,
                "tool_name": "Bash"
            }),
        );

        assert_eq!(event["v"], 1);
        assert_eq!(event["agent"], "codex");
        assert_eq!(event["event"], normalized);
        assert_eq!(event["session_id"], "thr_123");
        assert_eq!(event["cwd"], "/workspace");
    }
}

#[test]
fn normalizes_deepseek_environment_without_forwarding_tool_arguments() {
    for (native, normalized, tool_name) in [
        ("session_start", "session_start", None),
        ("message_submit", "prompt_submit", None),
        ("tool_call_before", "pre_tool_use", Some("exec_shell")),
        ("tool_call_after", "tool_complete", Some("exec_shell")),
        ("turn_end", "stop", None),
        ("session_end", "session_end", None),
    ] {
        let bytes = normalize_deepseek_hook_environment(
            native,
            Some("sess_12345678"),
            Some("/workspace"),
            tool_name,
        )
        .unwrap();
        let event: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(event["agent"], "deepseek");
        assert_eq!(event["event"], normalized);
        assert_eq!(event["session_id"], "sess_12345678");
        assert_eq!(event["cwd"], "/workspace");
        assert_eq!(event.get("tool_name").and_then(Value::as_str), tool_name);
        assert!(!event.to_string().contains("tool_args"));
    }
}

#[test]
fn antigravity_status_uses_the_authoritative_confirmation_and_agent_state_fields() {
    let pending = serde_json::to_vec(&json!({
        "conversation_id": "agy-123",
        "workspace": {"current_dir": "/workspace/project"},
        "agent_state": "tool_use",
        "tool_confirmation_pending": true,
        "email": "secret@example.com",
        "transcript_path": "/secret/transcript.jsonl"
    }))
    .unwrap();
    let events = normalize_antigravity_status_input(&pending).unwrap();
    assert_eq!(events.len(), 1);
    let event: Value = serde_json::from_slice(&events[0]).unwrap();
    assert_eq!(event["agent"], "agy");
    assert_eq!(event["event"], "permission_request");
    assert_eq!(event["session_id"], "agy-123");
    assert_eq!(event["cwd"], "/workspace/project");
    assert_eq!(event["summary"], "Antigravity is waiting for approval");
    assert!(!event.to_string().contains("secret@example.com"));
    assert!(!event.to_string().contains("transcript.jsonl"));

    let active = serde_json::to_vec(&json!({
        "conversation_id": "agy-123",
        "workspace": {"current_dir": "/workspace/project"},
        "agent_state": "tool_use",
        "tool_confirmation_pending": false
    }))
    .unwrap();
    let events = normalize_antigravity_status_input(&active).unwrap();
    assert_eq!(events.len(), 2);
    let events = events
        .iter()
        .map(|event| serde_json::from_slice::<Value>(event).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events[0]["event"], "permission_replied");
    assert_eq!(events[1]["event"], "pre_tool_use");
}

#[test]
fn antigravity_status_maps_idle_to_completion_and_waits_for_conversation_identity() {
    let idle = serde_json::to_vec(&json!({
        "session_id": "agy-legacy-alias",
        "cwd": "/workspace",
        "agent_state": "idle"
    }))
    .unwrap();
    let events = normalize_antigravity_status_input(&idle).unwrap();
    let events = events
        .iter()
        .map(|event| serde_json::from_slice::<Value>(event).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events[0]["event"], "permission_replied");
    assert_eq!(events[1]["event"], "stop");

    assert!(
        normalize_antigravity_status_input(br#"{"agent_state":"idle"}"#)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn deepseek_hook_normalization_fails_closed_without_identity_or_for_unknown_events() {
    assert!(
        normalize_deepseek_hook_environment("session_start", None, Some("/workspace"), None)
            .is_err()
    );
    assert!(normalize_deepseek_hook_environment(
        "approval_request",
        Some("sess_12345678"),
        Some("/workspace"),
        None
    )
    .is_err());
}

#[test]
fn normalization_drops_prompts_tool_inputs_tool_responses_and_assistant_text() {
    let normalized = normalize_hook_input(
        CLIAgent::Codex,
        serde_json::to_vec(&json!({
            "session_id": "thr_123",
            "cwd": "/workspace",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "prompt": "prompt-secret",
            "tool_input": {"command": "tool-input-secret"},
            "tool_response": {"output": "tool-response-secret"},
            "last_assistant_message": "assistant-secret",
            "transcript_path": "/secret/transcript.jsonl"
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();
    let normalized = String::from_utf8(normalized).unwrap();

    assert!(normalized.contains("\"tool_name\":\"Bash\""));
    for secret in [
        "prompt-secret",
        "tool-input-secret",
        "tool-response-secret",
        "assistant-secret",
        "transcript.jsonl",
    ] {
        assert!(!normalized.contains(secret));
    }
}

#[test]
fn normalized_hook_event_is_forwardable_over_the_authenticated_control_transport() {
    let body = normalize_hook_input(
        CLIAgent::Codex,
        br#"{"session_id":"thr_123","hook_event_name":"Stop"}"#,
    )
    .unwrap();
    let request = ControlRequest::hook_event(
        ControlAuth {
            token: "secret".to_string(),
            caller_surface_id: "surface-a".to_string(),
            caller_tab_id: "tab-a".to_string(),
        },
        String::from_utf8(body).unwrap(),
    )
    .unwrap();

    assert!(matches!(request.verb, ControlVerb::HookEvent { .. }));
    assert_eq!(request.auth.caller_surface_id, "surface-a");
    assert_eq!(request.auth.caller_tab_id, "tab-a");
}

#[test]
fn permission_request_uses_a_fixed_non_sensitive_summary() {
    let event = parse_normalized(
        CLIAgent::Codex,
        json!({
            "session_id": "thr_123",
            "cwd": "/workspace",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {"description": "contains-sensitive-details"}
        }),
    );

    assert_eq!(event["summary"], "Agent is waiting for approval");
    assert_eq!(event["tool_name"], "Bash");
    assert!(!event.to_string().contains("contains-sensitive-details"));
}

#[test]
fn normalizes_grok_camel_case_without_inventing_approval_state() {
    let event = parse_normalized(
        CLIAgent::Grok,
        json!({
            "sessionId": "grok-123",
            "cwd": "/workspace",
            "hookEventName": "PreToolUse",
            "toolName": "Bash"
        }),
    );
    assert_eq!(event["agent"], "grok");
    assert_eq!(event["event"], "pre_tool_use");
    assert_eq!(event["tool_name"], "Bash");

    let notification = parse_normalized(
        CLIAgent::Grok,
        json!({
            "sessionId": "grok-123",
            "hookEventName": "Notification"
        }),
    );
    assert_eq!(notification["event"], "idle_prompt");
    assert!(notification.get("summary").is_none());
}

#[test]
fn normalizes_documented_grok_snake_case_hook_payloads() {
    let event = parse_normalized(
        CLIAgent::Grok,
        json!({
            "sessionId": "grok-123",
            "cwd": "/workspace",
            "hookEventName": "pre_tool_use",
            "toolName": "run_terminal_cmd",
            "toolInput": {"command": "sensitive command"}
        }),
    );

    assert_eq!(event["agent"], "grok");
    assert_eq!(event["event"], "pre_tool_use");
    assert_eq!(event["session_id"], "grok-123");
    assert_eq!(event["tool_name"], "run_terminal_cmd");
    assert!(!event.to_string().contains("sensitive command"));

    let denied = parse_normalized(
        CLIAgent::Grok,
        json!({
            "sessionId": "grok-123",
            "hookEventName": "permission_denied",
            "toolName": "run_terminal_cmd",
            "toolInput": {"command": "sensitive command"}
        }),
    );
    assert_eq!(denied["event"], "tool_complete");
    assert_eq!(denied["tool_name"], "run_terminal_cmd");
    assert!(!denied.to_string().contains("sensitive command"));
}

#[test]
fn documented_grok_failure_events_clear_running_state() {
    let tool_failure = parse_normalized(
        CLIAgent::Grok,
        json!({
            "sessionId": "grok-123",
            "hookEventName": "PostToolUseFailure",
            "toolName": "run_terminal_cmd"
        }),
    );
    assert_eq!(tool_failure["event"], "tool_complete");
    assert_eq!(tool_failure["tool_name"], "run_terminal_cmd");

    let turn_failure = parse_normalized(
        CLIAgent::Grok,
        json!({
            "sessionId": "grok-123",
            "hookEventName": "StopFailure"
        }),
    );
    assert_eq!(turn_failure["event"], "stop");
}

#[test]
fn grok_ignores_inherited_claude_hooks_and_accepts_its_dedicated_hook() {
    assert_eq!(
        resolve_worker_agent(warp_cli::CliAgentHookAgent::Claude, true),
        None
    );
    assert_eq!(
        resolve_worker_agent(warp_cli::CliAgentHookAgent::Grok, true),
        Some(CLIAgent::Grok)
    );
    assert_eq!(
        resolve_worker_agent(warp_cli::CliAgentHookAgent::Claude, false),
        Some(CLIAgent::Claude)
    );
}

#[test]
fn claude_permission_notification_maps_to_blocked_without_forwarding_message() {
    let event = parse_normalized(
        CLIAgent::Claude,
        json!({
            "session_id": "claude-123",
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "message": "sensitive permission description"
        }),
    );

    assert_eq!(event["event"], "permission_request");
    assert_eq!(event["summary"], "Agent is waiting for approval");
    assert!(!event
        .to_string()
        .contains("sensitive permission description"));
}

#[test]
fn rejects_unknown_agents_events_and_oversized_fields() {
    let valid_input = br#"{
        "session_id":"thr_123",
        "cwd":"/workspace",
        "hook_event_name":"Stop"
    }"#;
    assert!(normalize_hook_input(CLIAgent::Unknown, valid_input).is_err());

    let unknown_event = json!({
        "session_id": "thr_123",
        "hook_event_name": "FutureEvent"
    });
    assert!(normalize_hook_input(
        CLIAgent::Codex,
        &serde_json::to_vec(&unknown_event).unwrap()
    )
    .is_err());

    let oversized_session_id = json!({
        "session_id": "x".repeat(513),
        "hook_event_name": "Stop"
    });
    assert!(normalize_hook_input(
        CLIAgent::Codex,
        &serde_json::to_vec(&oversized_session_id).unwrap()
    )
    .is_err());
}

#[test]
fn refresh_is_idempotent_and_preserves_unrelated_json() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("hooks.json");
    let foreign = json!({
        "description": "user hooks",
        "custom": {"enabled": true},
        "hooks": {
            "PreToolUse": [{
                "matcher": "^Bash$",
                "hooks": [{
                    "type": "command",
                    "command": "/usr/local/bin/user-policy"
                }]
            }]
        }
    });
    fs::write(&path, serde_json::to_vec_pretty(&foreign).unwrap()).unwrap();

    assert_eq!(
        refresh_codex_hooks(&path, Path::new("/opt/Zaplex App/zaplex")).unwrap(),
        HookConfigChange::Changed
    );
    let first_bytes = fs::read(&path).unwrap();
    let refreshed: Value = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(refreshed["description"], "user hooks");
    assert_eq!(refreshed["custom"], json!({"enabled": true}));
    assert_eq!(
        refreshed["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "/usr/local/bin/user-policy"
    );
    assert_eq!(
        refreshed["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        format!(
            "'/opt/Zaplex App/zaplex' cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
        )
    );

    assert_eq!(
        refresh_codex_hooks(&path, Path::new("/opt/Zaplex App/zaplex")).unwrap(),
        HookConfigChange::Unchanged
    );
    assert_eq!(fs::read(&path).unwrap(), first_bytes);
}

#[test]
fn deepseek_refresh_is_idempotent_and_preserves_unrelated_toml() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    fs::write(
        &path,
        r#"# keep this comment
model = "deepseek-chat"

[hooks]
enabled = false

[[hooks.hooks]]
name = "user-audit"
event = "turn_end"
command = "/usr/local/bin/user-audit"
"#,
    )
    .unwrap();

    assert_eq!(
        refresh_deepseek_hooks(&path, Path::new("/opt/Zaplex App/zaplex")).unwrap(),
        HookConfigChange::Changed
    );
    let first_bytes = fs::read(&path).unwrap();
    let source = String::from_utf8(first_bytes.clone()).unwrap();
    assert!(source.contains("# keep this comment"));
    let document: toml::Value = toml::from_str(&source).unwrap();
    assert_eq!(document["model"].as_str(), Some("deepseek-chat"));
    assert_eq!(document["hooks"]["enabled"].as_bool(), Some(true));
    let entries = document["hooks"]["hooks"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry["name"].as_str() == Some("user-audit")));
    let managed = entries
        .iter()
        .filter(|entry| entry["name"].as_str() == Some(MANAGED_BY_MARKER))
        .collect::<Vec<_>>();
    assert_eq!(managed.len(), DEEPSEEK_MANAGED_HOOK_EVENTS.len());
    for &event in DEEPSEEK_MANAGED_HOOK_EVENTS {
        let entry = managed
            .iter()
            .find(|entry| entry["event"].as_str() == Some(event))
            .unwrap();
        assert_eq!(entry["background"].as_bool(), Some(false));
        assert_eq!(entry["continue_on_error"].as_bool(), Some(true));
        assert!(entry["command"]
            .as_str()
            .unwrap()
            .contains(&format!("--event {event}")));
    }
    assert!(has_managed_deepseek_hooks(&path).unwrap());

    assert_eq!(
        refresh_deepseek_hooks(&path, Path::new("/opt/Zaplex App/zaplex")).unwrap(),
        HookConfigChange::Unchanged
    );
    assert_eq!(fs::read(&path).unwrap(), first_bytes);
}

#[test]
fn antigravity_title_bridge_is_idempotent_and_restores_the_exact_user_title() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("settings.json");
    let original_title = json!({
        "type": "command",
        "command": "/usr/local/bin/user-title --compact"
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "colorScheme": "dark",
            "title": original_title
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        refresh_antigravity_title(&path, Path::new("/opt/Zaplex App/zaplex")).unwrap(),
        HookConfigChange::Changed
    );
    let installed_bytes = fs::read(&path).unwrap();
    let installed: Value = serde_json::from_slice(&installed_bytes).unwrap();
    assert_eq!(installed["colorScheme"], "dark");
    assert!(installed["title"]["command"]
        .as_str()
        .unwrap()
        .contains("cli-agent-hook --agent antigravity"));
    assert!(has_managed_antigravity_title(&path).unwrap());

    let backup: Value =
        serde_json::from_slice(&fs::read(antigravity_title_backup_path(&path)).unwrap()).unwrap();
    assert_eq!(backup["v"], 1);
    assert_eq!(backup["title"], original_title);

    assert_eq!(
        refresh_antigravity_title(&path, Path::new("/opt/Zaplex App/zaplex")).unwrap(),
        HookConfigChange::Unchanged
    );
    assert_eq!(fs::read(&path).unwrap(), installed_bytes);

    assert_eq!(
        remove_antigravity_title(&path).unwrap(),
        HookConfigChange::Changed
    );
    let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(restored["colorScheme"], "dark");
    assert_eq!(restored["title"], original_title);
    assert!(!antigravity_title_backup_path(&path).exists());
    assert!(!has_managed_antigravity_title(&path).unwrap());

    assert_eq!(
        remove_antigravity_title(&path).unwrap(),
        HookConfigChange::Unchanged
    );
}

#[test]
fn antigravity_title_bridge_returns_to_the_builtin_default_when_no_user_title_existed() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("settings.json");
    fs::write(&path, br#"{"colorScheme":"dark"}"#).unwrap();

    refresh_antigravity_title(&path, Path::new("/opt/zaplex")).unwrap();
    remove_antigravity_title(&path).unwrap();

    let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(restored, json!({"colorScheme": "dark"}));
}

#[test]
fn deepseek_launch_refresh_preserves_a_user_disable() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    refresh_deepseek_hooks(&path, Path::new("/opt/zaplex-old")).unwrap();
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("enabled = true", "enabled = false");
    fs::write(&path, source).unwrap();

    assert_eq!(
        refresh_deepseek_hooks_with_activation(&path, Path::new("/opt/zaplex-current"), false)
            .unwrap(),
        HookConfigChange::Changed
    );
    let source = fs::read_to_string(&path).unwrap();
    let document: toml::Value = toml::from_str(&source).unwrap();
    assert_eq!(document["hooks"]["enabled"].as_bool(), Some(false));
    assert!(source.contains("/opt/zaplex-current"));
    assert!(!source.contains("/opt/zaplex-old"));
    assert!(!has_managed_deepseek_hooks(&path).unwrap());
}

#[test]
fn deepseek_remove_is_idempotent_and_preserves_foreign_hooks() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    refresh_deepseek_hooks(&path, Path::new("/opt/zaplex")).unwrap();
    let mut source = fs::read_to_string(&path).unwrap();
    source.push_str(
        r#"
[[hooks.hooks]]
name = "user-audit"
event = "turn_end"
command = "/usr/local/bin/user-audit"
"#,
    );
    fs::write(&path, source).unwrap();

    assert_eq!(
        remove_deepseek_hooks(&path).unwrap(),
        HookConfigChange::Changed
    );
    let first_bytes = fs::read(&path).unwrap();
    let source = String::from_utf8(first_bytes.clone()).unwrap();
    assert!(source.contains("user-audit"));
    assert!(!source.contains(MANAGED_BY_MARKER));
    assert!(!has_managed_deepseek_hooks(&path).unwrap());

    assert_eq!(
        remove_deepseek_hooks(&path).unwrap(),
        HookConfigChange::Unchanged
    );
    assert_eq!(fs::read(&path).unwrap(), first_bytes);
}

#[test]
fn managed_event_sets_match_each_agents_documented_surface() {
    assert!(CLAUDE_MANAGED_HOOK_EVENTS.contains(&"Notification"));
    assert!(CLAUDE_MANAGED_HOOK_EVENTS.contains(&"PermissionRequest"));
    assert!(CODEX_MANAGED_HOOK_EVENTS.contains(&"PermissionRequest"));
    assert!(!GROK_MANAGED_HOOK_EVENTS.contains(&"PermissionRequest"));
    assert!(GROK_MANAGED_HOOK_EVENTS.contains(&"Notification"));
    assert!(GROK_MANAGED_HOOK_EVENTS.contains(&"PermissionDenied"));
    assert!(DEEPSEEK_MANAGED_HOOK_EVENTS.contains(&"turn_end"));
    assert!(!DEEPSEEK_MANAGED_HOOK_EVENTS.contains(&"permission_request"));
}

#[test]
fn agent_refresh_and_remove_preserve_other_managed_agents() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("shared.json");
    refresh_hooks(
        &path,
        Path::new("/opt/zaplex"),
        warp_cli::CliAgentHookAgent::Claude,
    )
    .unwrap();
    refresh_hooks(
        &path,
        Path::new("/opt/zaplex"),
        warp_cli::CliAgentHookAgent::Codex,
    )
    .unwrap();

    assert!(has_managed_hooks(&path, warp_cli::CliAgentHookAgent::Claude).unwrap());
    assert!(has_managed_hooks(&path, warp_cli::CliAgentHookAgent::Codex).unwrap());

    remove_hooks(&path, warp_cli::CliAgentHookAgent::Claude).unwrap();
    assert!(!has_managed_hooks(&path, warp_cli::CliAgentHookAgent::Claude).unwrap());
    assert!(has_managed_hooks(&path, warp_cli::CliAgentHookAgent::Codex).unwrap());
}

#[test]
fn hook_paths_respect_agent_homes_and_include_claude_accounts() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(".claude-work/projects")).unwrap();
    fs::create_dir_all(temp.path().join(".claude-backup/projects")).unwrap();

    let claude_paths = hook_paths(
        warp_cli::CliAgentHookAgent::Claude,
        temp.path(),
        Some(std::ffi::OsStr::new("/custom/claude")),
        None,
        None,
        None,
        None,
        None,
    );
    assert!(claude_paths.contains(&temp.path().join(".claude/settings.json")));
    assert!(claude_paths.contains(&temp.path().join(".claude-work/settings.json")));
    assert!(!claude_paths.contains(&temp.path().join(".claude-backup/settings.json")));
    assert!(claude_paths.contains(&Path::new("/custom/claude/settings.json").to_path_buf()));

    assert_eq!(
        hook_paths(
            warp_cli::CliAgentHookAgent::Codex,
            temp.path(),
            None,
            None,
            Some(std::ffi::OsStr::new("/custom/codex")),
            None,
            None,
            None,
        ),
        vec![Path::new("/custom/codex/hooks.json").to_path_buf()]
    );
    assert_eq!(
        hook_paths(
            warp_cli::CliAgentHookAgent::Grok,
            temp.path(),
            None,
            None,
            None,
            Some(std::ffi::OsStr::new("/custom/grok")),
            None,
            None,
        ),
        vec![Path::new("/custom/grok/hooks/zaplex.json").to_path_buf()]
    );
    assert_eq!(
        hook_paths(
            warp_cli::CliAgentHookAgent::Antigravity,
            temp.path(),
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        vec![temp.path().join(".gemini/antigravity-cli/settings.json")]
    );
    assert_eq!(
        hook_paths(
            warp_cli::CliAgentHookAgent::DeepSeek,
            temp.path(),
            None,
            None,
            None,
            None,
            Some(std::ffi::OsStr::new("/custom/codewhale.toml")),
            None,
        ),
        vec![Path::new("/custom/codewhale.toml").to_path_buf()]
    );

    assert_eq!(
        hook_paths(
            warp_cli::CliAgentHookAgent::DeepSeek,
            temp.path(),
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        vec![temp.path().join(".codewhale/config.toml")]
    );
    fs::create_dir_all(temp.path().join(".deepseek")).unwrap();
    fs::write(temp.path().join(".deepseek/config.toml"), "").unwrap();
    assert_eq!(
        hook_paths(
            warp_cli::CliAgentHookAgent::DeepSeek,
            temp.path(),
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        vec![temp.path().join(".deepseek/config.toml")]
    );
}

#[test]
fn refresh_replaces_only_stale_zaplex_commands() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("hooks.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "Stop": [{
                    "hooks": [
                        {
                            "type": "command",
                            "command": format!(
                                "/old/zaplex cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
                            )
                        },
                        {
                            "type": "command",
                            "command": format!(
                                "/usr/bin/other cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
                            )
                        }
                    ]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    refresh_codex_hooks(&path, Path::new("/new/zaplex")).unwrap();
    let refreshed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let stop_groups = refreshed["hooks"]["Stop"].as_array().unwrap();
    let commands = stop_groups
        .iter()
        .flat_map(|group| group["hooks"].as_array().unwrap())
        .filter_map(|handler| handler["command"].as_str())
        .collect::<Vec<_>>();

    assert!(!commands
        .iter()
        .any(|command| command.contains("/old/zaplex")));
    assert!(commands
        .iter()
        .any(|command| command.contains("/usr/bin/other")));
    assert!(commands
        .iter()
        .any(|command| command.contains("/new/zaplex")));
}

#[test]
fn remove_is_idempotent_and_preserves_foreign_hooks() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("hooks.json");
    refresh_codex_hooks(&path, Path::new("/opt/zaplex")).unwrap();
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["foreign"] = json!("preserve");
    document["hooks"]["SessionStart"][0]["matcher"] = json!("user-added-matcher");
    document["hooks"]["Stop"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "hooks": [{
                "type": "command",
                "command": "/usr/local/bin/user-stop-hook"
            }]
        }));
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    assert_eq!(
        remove_codex_hooks(&path).unwrap(),
        HookConfigChange::Changed
    );
    let removed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(removed["foreign"], "preserve");
    assert_eq!(
        removed["hooks"]["SessionStart"][0]["matcher"],
        "user-added-matcher"
    );
    assert_eq!(removed["hooks"]["SessionStart"][0]["hooks"], json!([]));
    assert_eq!(
        removed["hooks"]["Stop"][0]["hooks"][0]["command"],
        "/usr/local/bin/user-stop-hook"
    );
    for groups in removed["hooks"].as_object().unwrap().values() {
        for group in groups.as_array().unwrap() {
            for handler in group["hooks"].as_array().unwrap() {
                assert!(!is_managed_hook_handler(handler));
            }
        }
    }

    let first_bytes = fs::read(&path).unwrap();
    assert_eq!(
        remove_codex_hooks(&path).unwrap(),
        HookConfigChange::Unchanged
    );
    assert_eq!(fs::read(&path).unwrap(), first_bytes);
}

#[test]
fn corrupt_json_and_corrupt_hook_shapes_fail_closed() {
    let temp = TempDir::new().unwrap();
    for (name, bytes) in [
        ("invalid.json", b"{not-json".as_slice()),
        ("invalid-shape.json", br#"{"hooks":[]}"#.as_slice()),
        (
            "invalid-handler.json",
            br#"{"hooks":{"Stop":[{"hooks":["not-an-object"]}]}}"#.as_slice(),
        ),
    ] {
        let path = temp.path().join(name);
        fs::write(&path, bytes).unwrap();
        let before = fs::read(&path).unwrap();

        assert!(refresh_codex_hooks(&path, Path::new("/opt/zaplex")).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(remove_codex_hooks(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn corrupt_deepseek_toml_fails_closed() {
    let temp = TempDir::new().unwrap();
    for (name, source) in [
        ("invalid.toml", "[hooks"),
        ("invalid-shape.toml", "hooks = []"),
        ("invalid-entries.toml", "[hooks]\nhooks = []\n"),
    ] {
        let path = temp.path().join(name);
        fs::write(&path, source).unwrap();
        let before = fs::read(&path).unwrap();

        assert!(refresh_deepseek_hooks(&path, Path::new("/opt/zaplex")).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(remove_deepseek_hooks(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn atomic_refresh_preserves_existing_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let path = temp.path().join("hooks.json");
    fs::write(&path, b"{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    refresh_codex_hooks(&path, Path::new("/opt/zaplex")).unwrap();

    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn ownership_detection_requires_the_zaplex_executable_and_exact_marker() {
    let managed = json!({
        "type": "command",
        "command": format!(
            "/opt/zaplex cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
        )
    });
    let wrong_executable = json!({
        "type": "command",
        "command": format!(
            "/opt/other cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
        )
    });
    let marker_only = json!({
        "type": "command",
        "command": format!("echo {MANAGED_BY_MARKER}")
    });
    let wrong_type = json!({
        "type": "prompt",
        "command": format!(
            "/opt/zaplex cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
        )
    });
    let wrong_subcommand = json!({
        "type": "command",
        "command": format!(
            "/opt/zaplex other cli-agent-hook --agent codex --managed-by {MANAGED_BY_MARKER}"
        )
    });

    assert!(is_managed_hook_handler(&managed));
    assert!(!is_managed_hook_handler(&wrong_executable));
    assert!(!is_managed_hook_handler(&marker_only));
    assert!(!is_managed_hook_handler(&wrong_type));
    assert!(!is_managed_hook_handler(&wrong_subcommand));
}
