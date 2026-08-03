//! Structured hook bridge for native CLI-agent hooks.
//!
//! Hook payloads are minimized here, then forwarded through the authenticated
//! local control socket bound to the PTY that launched the agent.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use command::{blocking::Command, Stdio};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tempfile::NamedTempFile;
use toml_edit::{
    value as toml_value, ArrayOfTables as TomlArrayOfTables, DocumentMut as TomlDocument,
    Item as TomlItem, Table as TomlTable,
};

use crate::terminal::CLIAgent;

const MAX_HOOK_INPUT_BYTES: u64 = 64 * 1024;
const MAX_SESSION_ID_BYTES: usize = 512;
const MAX_CWD_BYTES: usize = 4096;
const MAX_TOOL_NAME_BYTES: usize = 512;
const MAX_ANTIGRAVITY_TITLE_BYTES: usize = 16 * 1024;
const ANTIGRAVITY_TITLE_BACKUP_FILE: &str = ".zaplex-title-backup.json";
const CLAUDE_MANAGED_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Notification",
    "UserPromptSubmit",
    "Stop",
    "SessionEnd",
];
const CODEX_MANAGED_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "UserPromptSubmit",
    "Stop",
    "SessionEnd",
];
const GROK_MANAGED_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionDenied",
    "UserPromptSubmit",
    "Stop",
    "StopFailure",
    "Notification",
    "SessionEnd",
];
const DEEPSEEK_MANAGED_HOOK_EVENTS: &[&str] = &[
    "session_start",
    "message_submit",
    "tool_call_before",
    "tool_call_after",
    "turn_end",
    "session_end",
];

pub(crate) const MANAGED_BY_MARKER: &str = "zaplex-cli-agent-hook-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookConfigChange {
    Changed,
    Unchanged,
}

#[derive(Deserialize)]
struct NativeHookInput {
    #[serde(alias = "sessionId")]
    session_id: String,
    cwd: Option<String>,
    #[serde(alias = "hookEventName")]
    hook_event_name: String,
    #[serde(alias = "toolName")]
    tool_name: Option<String>,
    #[serde(alias = "notificationType")]
    notification_type: Option<String>,
}

#[derive(Deserialize)]
struct AntigravityStatusInput {
    conversation_id: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    workspace: Option<AntigravityWorkspace>,
    agent_state: Option<String>,
    #[serde(default)]
    tool_confirmation_pending: bool,
}

#[derive(Deserialize)]
struct AntigravityWorkspace {
    current_dir: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct AntigravityTitleBackup {
    v: u32,
    title: Option<Value>,
}

#[derive(Serialize)]
struct HookBridgeEvent<'a> {
    v: u32,
    agent: &'a str,
    event: &'a str,
    session_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<&'a str>,
}

pub(crate) fn run_hook_worker(
    agent: warp_cli::CliAgentHookAgent,
    managed_by: &str,
    native_event: Option<&str>,
) -> Result<()> {
    if managed_by != MANAGED_BY_MARKER {
        bail!("unrecognized hook worker owner");
    }

    let Some(agent) = resolve_worker_agent(agent, std::env::var_os("GROK_HOOK_EVENT").is_some())
    else {
        return Ok(());
    };
    let mut input = Vec::new();
    io::stdin()
        .take(MAX_HOOK_INPUT_BYTES + 1)
        .read_to_end(&mut input)
        .context("failed to read hook input")?;
    if input.len() as u64 > MAX_HOOK_INPUT_BYTES {
        bail!("hook input exceeds {MAX_HOOK_INPUT_BYTES} bytes");
    }

    if agent == CLIAgent::Antigravity {
        let events = normalize_antigravity_status_input(&input)?;
        if let Err(error) = events
            .into_iter()
            .try_for_each(crate::control_surface::forward_hook_event_if_available)
        {
            log::warn!("failed to forward Antigravity status to Zaplex: {error:#}");
        }
        return render_antigravity_title(&input);
    }

    let body = if agent == CLIAgent::DeepSeek {
        normalize_deepseek_hook_environment(
            native_event.context("DeepSeek hook event is missing")?,
            std::env::var("DEEPSEEK_SESSION_ID").ok().as_deref(),
            std::env::var("DEEPSEEK_WORKSPACE").ok().as_deref(),
            std::env::var("DEEPSEEK_TOOL_NAME").ok().as_deref(),
        )?
    } else {
        normalize_hook_input(agent, &input)?
    };
    if let Err(error) = crate::control_surface::forward_hook_event_if_available(body) {
        log::warn!("failed to forward CLI-agent hook to Zaplex: {error:#}");
    }
    Ok(())
}

fn resolve_worker_agent(
    requested: warp_cli::CliAgentHookAgent,
    grok_environment: bool,
) -> Option<CLIAgent> {
    // Grok reads Claude hook files for compatibility. Ignore inherited Zaplex
    // entries so the dedicated per-agent Grok toggle remains independent and
    // the same lifecycle event is not forwarded twice.
    if grok_environment && requested != warp_cli::CliAgentHookAgent::Grok {
        return None;
    }
    Some(match requested {
        warp_cli::CliAgentHookAgent::Claude => CLIAgent::Claude,
        warp_cli::CliAgentHookAgent::Codex => CLIAgent::Codex,
        warp_cli::CliAgentHookAgent::Grok => CLIAgent::Grok,
        warp_cli::CliAgentHookAgent::Antigravity => CLIAgent::Antigravity,
        warp_cli::CliAgentHookAgent::DeepSeek => CLIAgent::DeepSeek,
    })
}

pub(super) fn normalize_antigravity_status_input(input: &[u8]) -> Result<Vec<Vec<u8>>> {
    let raw: AntigravityStatusInput =
        serde_json::from_slice(input).context("Antigravity status input is not valid JSON")?;
    let Some(session_id) = raw
        .conversation_id
        .as_deref()
        .or(raw.session_id.as_deref())
        .filter(|session_id| !session_id.is_empty())
    else {
        return Ok(Vec::new());
    };
    validate_field("conversation_id", session_id, MAX_SESSION_ID_BYTES)?;
    let cwd = raw
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.current_dir.as_deref())
        .or(raw.cwd.as_deref());
    if let Some(cwd) = cwd {
        validate_field("cwd", cwd, MAX_CWD_BYTES)?;
    }

    if raw.tool_confirmation_pending {
        return Ok(vec![serialize_hook_bridge_event(HookBridgeEvent {
            v: 1,
            agent: "agy",
            event: "permission_request",
            session_id,
            cwd,
            summary: Some("Antigravity is waiting for approval"),
            tool_name: None,
        })?]);
    }

    let mut events = vec![serialize_hook_bridge_event(HookBridgeEvent {
        v: 1,
        agent: "agy",
        event: "permission_replied",
        session_id,
        cwd,
        summary: None,
        tool_name: None,
    })?];
    let event = match raw.agent_state.as_deref() {
        Some("initializing") => Some("session_start"),
        Some("thinking") => Some("prompt_submit"),
        Some("working" | "tool_use") => Some("pre_tool_use"),
        Some("idle") => Some("stop"),
        Some(_) | None => None,
    };
    if let Some(event) = event {
        events.push(serialize_hook_bridge_event(HookBridgeEvent {
            v: 1,
            agent: "agy",
            event,
            session_id,
            cwd,
            summary: None,
            tool_name: None,
        })?);
    }
    Ok(events)
}

fn serialize_hook_bridge_event(event: HookBridgeEvent<'_>) -> Result<Vec<u8>> {
    serde_json::to_vec(&event).context("failed to serialize hook event")
}

fn render_antigravity_title(input: &[u8]) -> Result<()> {
    let backup = antigravity_settings_path_for_current_user()
        .map(|settings| antigravity_title_backup_path(&settings))
        .and_then(|path| read_antigravity_title_backup(&path).ok())
        .flatten();
    if let Some(command) = backup
        .as_ref()
        .and_then(|backup| backup.title.as_ref())
        .and_then(Value::as_object)
        .filter(|title| title.get("type").and_then(Value::as_str) == Some("command"))
        .and_then(|title| title.get("command").and_then(Value::as_str))
        .filter(|command| !is_managed_hook_command(command, None))
    {
        if render_with_user_title_command(command, input).is_ok() {
            return Ok(());
        }
    }

    let raw: AntigravityStatusInput =
        serde_json::from_slice(input).context("Antigravity status input is not valid JSON")?;
    let workspace = raw
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.current_dir.as_deref())
        .or(raw.cwd.as_deref())
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str());
    let state = if raw.tool_confirmation_pending {
        "Needs approval"
    } else {
        match raw.agent_state.as_deref() {
            Some("initializing") => "Starting",
            Some("thinking") => "Thinking",
            Some("working") => "Working",
            Some("tool_use") => "Using a tool",
            Some("idle") => "Ready",
            Some(_) | None => "Active",
        }
    };
    let title = workspace.map_or_else(
        || format!("Antigravity · {state}"),
        |workspace| format!("Antigravity · {workspace} · {state}"),
    );
    io::stdout()
        .write_all(title.as_bytes())
        .context("failed to write Antigravity title")
}

fn render_with_user_title_command(command: &str, input: &[u8]) -> Result<()> {
    #[cfg(windows)]
    let mut child = Command::new("cmd.exe")
        .args(["/D", "/S", "/C", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start the previous Antigravity title command")?;
    #[cfg(not(windows))]
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start the previous Antigravity title command")?;

    child
        .stdin
        .take()
        .context("previous Antigravity title command has no stdin")?
        .write_all(input)
        .context("failed to forward Antigravity status input")?;
    let output = child
        .wait_with_output()
        .context("previous Antigravity title command failed")?;
    if !output.status.success() {
        bail!(
            "previous Antigravity title command exited with {0}",
            output.status
        );
    }
    if output.stdout.len() > MAX_ANTIGRAVITY_TITLE_BYTES {
        bail!("previous Antigravity title exceeds {MAX_ANTIGRAVITY_TITLE_BYTES} bytes");
    }
    io::stdout()
        .write_all(&output.stdout)
        .context("failed to relay previous Antigravity title")
}

fn normalize_deepseek_hook_environment(
    native_event: &str,
    session_id: Option<&str>,
    workspace: Option<&str>,
    tool_name: Option<&str>,
) -> Result<Vec<u8>> {
    let session_id = session_id.context("DEEPSEEK_SESSION_ID is missing")?;
    validate_field("session_id", session_id, MAX_SESSION_ID_BYTES)?;
    if let Some(workspace) = workspace {
        validate_field("workspace", workspace, MAX_CWD_BYTES)?;
    }
    if let Some(tool_name) = tool_name {
        validate_field("tool_name", tool_name, MAX_TOOL_NAME_BYTES)?;
    }

    let (event, include_tool_name) = match native_event {
        "session_start" => ("session_start", false),
        "message_submit" => ("prompt_submit", false),
        "tool_call_before" => ("pre_tool_use", true),
        "tool_call_after" => ("tool_complete", true),
        "turn_end" => ("stop", false),
        "session_end" => ("session_end", false),
        _ => bail!("unsupported DeepSeek hook event"),
    };

    serde_json::to_vec(&HookBridgeEvent {
        v: 1,
        agent: "deepseek",
        event,
        session_id,
        cwd: workspace,
        summary: None,
        tool_name: include_tool_name.then_some(tool_name).flatten(),
    })
    .context("failed to serialize DeepSeek hook event")
}

pub(super) fn normalize_hook_input(agent: CLIAgent, input: &[u8]) -> Result<Vec<u8>> {
    let agent_name = match agent {
        CLIAgent::Claude => "claude",
        CLIAgent::Codex => "codex",
        CLIAgent::Grok => "grok",
        CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::OpenCode
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::DeepSeek
        | CLIAgent::Antigravity
        | CLIAgent::Unknown => bail!("unsupported hook agent"),
    };
    let raw: NativeHookInput =
        serde_json::from_slice(input).context("hook input is not valid JSON")?;
    validate_field("session_id", &raw.session_id, MAX_SESSION_ID_BYTES)?;
    if let Some(cwd) = raw.cwd.as_deref() {
        validate_field("cwd", cwd, MAX_CWD_BYTES)?;
    }
    if let Some(tool_name) = raw.tool_name.as_deref() {
        validate_field("tool_name", tool_name, MAX_TOOL_NAME_BYTES)?;
    }

    let (event, summary, include_tool_name) = match raw.hook_event_name.as_str() {
        "SessionStart" | "session_start" => ("session_start", None, false),
        "PreToolUse" | "pre_tool_use" => ("pre_tool_use", None, true),
        "PermissionRequest" | "permission_request" => (
            "permission_request",
            Some("Agent is waiting for approval"),
            true,
        ),
        "PostToolUse" | "post_tool_use" | "PostToolUseFailure" | "post_tool_use_failure" => {
            ("tool_complete", None, true)
        }
        "PermissionDenied" | "permission_denied" => ("tool_complete", None, true),
        "UserPromptSubmit" | "user_prompt_submit" => ("prompt_submit", None, false),
        "Stop" | "stop" | "StopFailure" | "stop_failure" => ("stop", None, false),
        "SessionEnd" | "session_end" => ("session_end", None, false),
        "Notification" | "notification"
            if raw.notification_type.as_deref() == Some("permission_prompt") =>
        {
            (
                "permission_request",
                Some("Agent is waiting for approval"),
                false,
            )
        }
        // Notification is a generic event for Grok and for non-permission
        // Claude notifications. It is not evidence that approval is pending.
        "Notification" | "notification" => ("idle_prompt", None, false),
        _ => bail!("unsupported hook event"),
    };

    let event = HookBridgeEvent {
        v: 1,
        agent: agent_name,
        event,
        session_id: &raw.session_id,
        cwd: raw.cwd.as_deref(),
        summary,
        tool_name: include_tool_name
            .then_some(raw.tool_name.as_deref())
            .flatten(),
    };
    serde_json::to_vec(&event).context("failed to serialize hook event")
}

fn validate_field(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty() {
        bail!("{name} must not be empty");
    }
    if value.len() > max_bytes {
        bail!("{name} exceeds {max_bytes} bytes");
    }
    Ok(())
}

/// Refreshes only Zaplex-owned entries for one agent in a JSON hook file.
///
/// This helper does not edit trust state. The caller must leave Codex's normal
/// hook review flow intact.
fn refresh_hooks(
    hooks_path: &Path,
    zaplex_executable: &Path,
    agent: warp_cli::CliAgentHookAgent,
) -> Result<HookConfigChange> {
    if matches!(
        agent,
        warp_cli::CliAgentHookAgent::DeepSeek | warp_cli::CliAgentHookAgent::Antigravity
    ) {
        bail!("this agent does not use JSON hook groups");
    }
    mutate_json_file(hooks_path, true, |root| {
        remove_managed_hooks(root, Some(agent))?;
        append_managed_hooks(
            root,
            managed_hook_events(agent),
            &managed_hook_command(zaplex_executable, agent)?,
        )
    })
}

/// Removes only Zaplex-owned entries for one agent from a JSON hook file.
fn remove_hooks(hooks_path: &Path, agent: warp_cli::CliAgentHookAgent) -> Result<HookConfigChange> {
    if matches!(
        agent,
        warp_cli::CliAgentHookAgent::DeepSeek | warp_cli::CliAgentHookAgent::Antigravity
    ) {
        bail!("this agent does not use JSON hook groups");
    }
    mutate_json_file(hooks_path, false, |root| {
        remove_managed_hooks(root, Some(agent))
    })
}

fn refresh_agent_hooks(
    hooks_path: &Path,
    zaplex_executable: &Path,
    agent: warp_cli::CliAgentHookAgent,
    activate_deepseek: bool,
) -> Result<HookConfigChange> {
    match agent {
        warp_cli::CliAgentHookAgent::DeepSeek => {
            refresh_deepseek_hooks_with_activation(hooks_path, zaplex_executable, activate_deepseek)
        }
        warp_cli::CliAgentHookAgent::Antigravity => {
            refresh_antigravity_title(hooks_path, zaplex_executable)
        }
        warp_cli::CliAgentHookAgent::Claude
        | warp_cli::CliAgentHookAgent::Codex
        | warp_cli::CliAgentHookAgent::Grok => refresh_hooks(hooks_path, zaplex_executable, agent),
    }
}

fn remove_agent_hooks(
    hooks_path: &Path,
    agent: warp_cli::CliAgentHookAgent,
) -> Result<HookConfigChange> {
    match agent {
        warp_cli::CliAgentHookAgent::DeepSeek => remove_deepseek_hooks(hooks_path),
        warp_cli::CliAgentHookAgent::Antigravity => remove_antigravity_title(hooks_path),
        warp_cli::CliAgentHookAgent::Claude
        | warp_cli::CliAgentHookAgent::Codex
        | warp_cli::CliAgentHookAgent::Grok => remove_hooks(hooks_path, agent),
    }
}

fn has_agent_hooks(path: &Path, agent: warp_cli::CliAgentHookAgent) -> Result<bool> {
    match agent {
        warp_cli::CliAgentHookAgent::DeepSeek => has_managed_deepseek_hooks(path),
        warp_cli::CliAgentHookAgent::Antigravity => has_managed_antigravity_title(path),
        warp_cli::CliAgentHookAgent::Claude
        | warp_cli::CliAgentHookAgent::Codex
        | warp_cli::CliAgentHookAgent::Grok => has_managed_hooks(path, agent),
    }
}

#[cfg(test)]
fn refresh_codex_hooks(hooks_path: &Path, zaplex_executable: &Path) -> Result<HookConfigChange> {
    refresh_hooks(
        hooks_path,
        zaplex_executable,
        warp_cli::CliAgentHookAgent::Codex,
    )
}

#[cfg(test)]
fn remove_codex_hooks(hooks_path: &Path) -> Result<HookConfigChange> {
    remove_hooks(hooks_path, warp_cli::CliAgentHookAgent::Codex)
}

pub(crate) fn install_for_current_user(agent: warp_cli::CliAgentHookAgent) -> Result<()> {
    let executable = std::env::current_exe().context("failed to resolve the Zaplex executable")?;
    let paths = hook_paths_for_current_user(agent)?;
    for path in &paths {
        refresh_agent_hooks(path, &executable, agent, true)?;
    }
    for path in paths {
        println!("Installed {} hooks at {}", agent.as_str(), path.display());
    }
    Ok(())
}

pub(crate) fn remove_for_current_user(agent: warp_cli::CliAgentHookAgent) -> Result<()> {
    let paths = hook_paths_for_current_user(agent)?;
    for path in &paths {
        remove_agent_hooks(path, agent)?;
    }
    for path in paths {
        println!("Removed {} hooks from {}", agent.as_str(), path.display());
    }
    Ok(())
}

/// Refreshes existing Zaplex-managed hooks without opting an agent in.
pub(crate) fn refresh_installed_for_current_user() -> Result<()> {
    let executable = std::env::current_exe().context("failed to resolve the Zaplex executable")?;
    for agent in [
        warp_cli::CliAgentHookAgent::Claude,
        warp_cli::CliAgentHookAgent::Codex,
        warp_cli::CliAgentHookAgent::Grok,
        warp_cli::CliAgentHookAgent::Antigravity,
        warp_cli::CliAgentHookAgent::DeepSeek,
    ] {
        for path in hook_paths_for_current_user(agent)? {
            if has_agent_hooks(&path, agent)? {
                refresh_agent_hooks(&path, &executable, agent, false)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn is_installed_for_current_user(agent: warp_cli::CliAgentHookAgent) -> Result<bool> {
    hook_paths_for_current_user(agent)?
        .iter()
        .try_fold(false, |installed, path| {
            Ok(installed || has_agent_hooks(path, agent)?)
        })
}

pub(crate) fn supports_agent(agent: CLIAgent) -> bool {
    hook_agent(agent).is_some()
}

pub(crate) fn set_installed_for_agent(agent: CLIAgent, installed: bool) -> Result<()> {
    let agent = hook_agent(agent).context("this CLI agent has no native Zaplex hook bridge")?;
    if installed {
        install_for_current_user(agent)
    } else {
        remove_for_current_user(agent)
    }
}

pub(crate) fn is_installed_for_agent(agent: CLIAgent) -> Result<bool> {
    let Some(agent) = hook_agent(agent) else {
        return Ok(false);
    };
    is_installed_for_current_user(agent)
}

fn hook_agent(agent: CLIAgent) -> Option<warp_cli::CliAgentHookAgent> {
    Some(match agent {
        CLIAgent::Claude => warp_cli::CliAgentHookAgent::Claude,
        CLIAgent::Codex => warp_cli::CliAgentHookAgent::Codex,
        CLIAgent::Grok => warp_cli::CliAgentHookAgent::Grok,
        CLIAgent::DeepSeek => warp_cli::CliAgentHookAgent::DeepSeek,
        CLIAgent::Antigravity => warp_cli::CliAgentHookAgent::Antigravity,
        CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::OpenCode
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::Unknown => return None,
    })
}

fn hook_paths_for_current_user(agent: warp_cli::CliAgentHookAgent) -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    Ok(hook_paths(
        agent,
        &home,
        std::env::var_os("CLAUDE_CONFIG_DIR").as_deref(),
        std::env::var_os("CLAUDE_HOME").as_deref(),
        std::env::var_os("CODEX_HOME").as_deref(),
        std::env::var_os("GROK_HOME").as_deref(),
        std::env::var_os("CODEWHALE_CONFIG_PATH").as_deref(),
        std::env::var_os("DEEPSEEK_CONFIG_PATH").as_deref(),
    ))
}

fn hook_paths(
    agent: warp_cli::CliAgentHookAgent,
    home: &Path,
    claude_config_dir: Option<&std::ffi::OsStr>,
    claude_home: Option<&std::ffi::OsStr>,
    codex_home: Option<&std::ffi::OsStr>,
    grok_home: Option<&std::ffi::OsStr>,
    codewhale_config_path: Option<&std::ffi::OsStr>,
    deepseek_config_path: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut paths = match agent {
        warp_cli::CliAgentHookAgent::Claude => {
            let mut config_dirs = vec![home.join(".claude")];
            config_dirs.extend(
                zaplex_cockpit::claude::discover_accounts(
                    home,
                    claude_config_dir.and_then(std::ffi::OsStr::to_str),
                )
                .into_iter()
                .map(|account| account.config_dir),
            );
            for configured in [claude_config_dir, claude_home].into_iter().flatten() {
                let configured = PathBuf::from(configured);
                if !configured.as_os_str().is_empty() {
                    config_dirs.push(configured);
                }
            }
            config_dirs
                .into_iter()
                .map(|config_dir| config_dir.join("settings.json"))
                .collect()
        }
        warp_cli::CliAgentHookAgent::Codex => {
            let config_dir = codex_home
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            vec![config_dir.join("hooks.json")]
        }
        warp_cli::CliAgentHookAgent::Grok => {
            let config_dir = grok_home
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".grok"));
            vec![config_dir.join("hooks").join("zaplex.json")]
        }
        warp_cli::CliAgentHookAgent::Antigravity => {
            vec![home
                .join(".gemini")
                .join("antigravity-cli")
                .join("settings.json")]
        }
        warp_cli::CliAgentHookAgent::DeepSeek => {
            let current = home.join(".codewhale").join("config.toml");
            let legacy = home.join(".deepseek").join("config.toml");
            let configured = codewhale_config_path
                .filter(|path| !path.is_empty())
                .or_else(|| deepseek_config_path.filter(|path| !path.is_empty()))
                .map(PathBuf::from);
            vec![configured.unwrap_or_else(|| {
                if current.exists() || !legacy.exists() {
                    current
                } else {
                    legacy
                }
            })]
        }
    };
    paths.sort();
    paths.dedup();
    paths
}

fn managed_hook_events(agent: warp_cli::CliAgentHookAgent) -> &'static [&'static str] {
    match agent {
        warp_cli::CliAgentHookAgent::Claude => CLAUDE_MANAGED_HOOK_EVENTS,
        warp_cli::CliAgentHookAgent::Codex => CODEX_MANAGED_HOOK_EVENTS,
        warp_cli::CliAgentHookAgent::Grok => GROK_MANAGED_HOOK_EVENTS,
        warp_cli::CliAgentHookAgent::Antigravity | warp_cli::CliAgentHookAgent::DeepSeek => &[],
    }
}

fn managed_hook_command(
    zaplex_executable: &Path,
    agent: warp_cli::CliAgentHookAgent,
) -> Result<String> {
    let executable = zaplex_executable
        .to_str()
        .context("Zaplex executable path is not valid UTF-8")?;
    Ok(format!(
        "{} cli-agent-hook --agent {} --managed-by {}",
        quote_executable(executable),
        agent.as_str(),
        MANAGED_BY_MARKER
    ))
}

fn managed_deepseek_hook_command(zaplex_executable: &Path, event: &str) -> Result<String> {
    Ok(format!(
        "{} --event {event}",
        managed_hook_command(zaplex_executable, warp_cli::CliAgentHookAgent::DeepSeek)?
    ))
}

#[cfg(not(windows))]
fn quote_executable(executable: &str) -> String {
    shell_escape::escape(executable.into()).into_owned()
}

#[cfg(windows)]
fn quote_executable(executable: &str) -> String {
    format!("\"{executable}\"")
}

fn antigravity_settings_path_for_current_user() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".gemini")
            .join("antigravity-cli")
            .join("settings.json")
    })
}

fn antigravity_title_backup_path(settings_path: &Path) -> PathBuf {
    settings_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(ANTIGRAVITY_TITLE_BACKUP_FILE)
}

fn managed_antigravity_title(zaplex_executable: &Path) -> Result<Value> {
    Ok(json!({
        "type": "command",
        "command": managed_hook_command(
            zaplex_executable,
            warp_cli::CliAgentHookAgent::Antigravity,
        )?
    }))
}

fn is_managed_antigravity_title(title: Option<&Value>) -> bool {
    let Some(title) = title.and_then(Value::as_object) else {
        return false;
    };
    title.get("type").and_then(Value::as_str) == Some("command")
        && title
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| {
                is_managed_hook_command(command, Some(warp_cli::CliAgentHookAgent::Antigravity))
            })
}

fn write_antigravity_title_backup(path: &Path, title: Option<Value>) -> Result<()> {
    let backup = AntigravityTitleBackup { v: 1, title };
    let mut bytes = serde_json::to_vec_pretty(&backup)
        .context("failed to serialize Antigravity title backup")?;
    bytes.push(b'\n');
    let metadata = existing_config_metadata(path)?;
    persist_atomic(path, &bytes, metadata.as_ref())
}

fn read_antigravity_title_backup(path: &Path) -> Result<Option<AntigravityTitleBackup>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let backup = serde_json::from_slice::<AntigravityTitleBackup>(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if backup.v != 1 {
        bail!("unsupported Antigravity title backup version {}", backup.v);
    }
    Ok(Some(backup))
}

fn refresh_antigravity_title(
    settings_path: &Path,
    zaplex_executable: &Path,
) -> Result<HookConfigChange> {
    let managed_title = managed_antigravity_title(zaplex_executable)?;
    let backup_path = antigravity_title_backup_path(settings_path);
    mutate_json_file(settings_path, true, |root| {
        if !is_managed_antigravity_title(root.get("title")) {
            write_antigravity_title_backup(&backup_path, root.get("title").cloned())?;
        }
        root.insert("title".to_owned(), managed_title);
        Ok(())
    })
}

fn remove_antigravity_title(settings_path: &Path) -> Result<HookConfigChange> {
    let backup_path = antigravity_title_backup_path(settings_path);
    let backup = read_antigravity_title_backup(&backup_path)?;
    let mut restored = false;
    let change = mutate_json_file(settings_path, false, |root| {
        if !is_managed_antigravity_title(root.get("title")) {
            return Ok(());
        }
        match backup.as_ref().and_then(|backup| backup.title.clone()) {
            Some(title) => {
                root.insert("title".to_owned(), title);
            }
            None => {
                root.remove("title");
            }
        }
        restored = true;
        Ok(())
    })?;
    if restored {
        match fs::remove_file(&backup_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to remove {}", backup_path.display()));
            }
        }
    }
    Ok(change)
}

fn has_managed_antigravity_title(settings_path: &Path) -> Result<bool> {
    let bytes = match fs::read(settings_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", settings_path.display()));
        }
    };
    let document: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", settings_path.display()))?;
    Ok(is_managed_antigravity_title(document.get("title")))
}

fn refresh_deepseek_hooks(hooks_path: &Path, zaplex_executable: &Path) -> Result<HookConfigChange> {
    refresh_deepseek_hooks_with_activation(hooks_path, zaplex_executable, true)
}

fn refresh_deepseek_hooks_with_activation(
    hooks_path: &Path,
    zaplex_executable: &Path,
    activate: bool,
) -> Result<HookConfigChange> {
    mutate_toml_file(hooks_path, true, |document| {
        remove_managed_deepseek_hooks(document)?;
        let hooks = ensure_deepseek_hooks_table(document)?;
        // CodeWhale has one global hook switch. An explicit Zaplex install
        // activates it; launch-time refreshes preserve a later user disable.
        if activate || !hooks.contains_key("enabled") {
            hooks.insert("enabled", toml_value(true));
        }
        if !hooks.contains_key("hooks") {
            hooks.insert("hooks", TomlItem::ArrayOfTables(TomlArrayOfTables::new()));
        }
        let entries = hooks
            .get_mut("hooks")
            .and_then(TomlItem::as_array_of_tables_mut)
            .context("hooks.hooks must be an array of tables")?;
        for &event in DEEPSEEK_MANAGED_HOOK_EVENTS {
            let mut entry = TomlTable::new();
            entry.insert("name", toml_value(MANAGED_BY_MARKER));
            entry.insert("event", toml_value(event));
            entry.insert(
                "command",
                toml_value(managed_deepseek_hook_command(zaplex_executable, event)?),
            );
            entry.insert("timeout_secs", toml_value(3_i64));
            entry.insert("background", toml_value(false));
            entry.insert("continue_on_error", toml_value(true));
            entries.push(entry);
        }
        Ok(())
    })
}

fn remove_deepseek_hooks(hooks_path: &Path) -> Result<HookConfigChange> {
    mutate_toml_file(hooks_path, false, remove_managed_deepseek_hooks)
}

fn ensure_deepseek_hooks_table(document: &mut TomlDocument) -> Result<&mut TomlTable> {
    if !document.as_table().contains_key("hooks") {
        document
            .as_table_mut()
            .insert("hooks", TomlItem::Table(TomlTable::new()));
    }
    document
        .get_mut("hooks")
        .and_then(TomlItem::as_table_mut)
        .context("hooks must be a TOML table")
}

fn remove_managed_deepseek_hooks(document: &mut TomlDocument) -> Result<()> {
    let Some(hooks) = document.get_mut("hooks") else {
        return Ok(());
    };
    let hooks = hooks.as_table_mut().context("hooks must be a TOML table")?;
    let Some(entries) = hooks.get_mut("hooks") else {
        return Ok(());
    };
    let entries = entries
        .as_array_of_tables_mut()
        .context("hooks.hooks must be an array of tables")?;
    entries.retain(|entry| !is_managed_deepseek_hook(entry));
    Ok(())
}

fn is_managed_deepseek_hook(entry: &TomlTable) -> bool {
    entry.get("name").and_then(TomlItem::as_str) == Some(MANAGED_BY_MARKER)
        && entry
            .get("command")
            .and_then(TomlItem::as_str)
            .is_some_and(|command| {
                is_managed_hook_command(command, Some(warp_cli::CliAgentHookAgent::DeepSeek))
            })
}

fn has_managed_deepseek_hooks(path: &Path) -> Result<bool> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let document = source
        .parse::<TomlDocument>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(hooks) = document.get("hooks").and_then(TomlItem::as_table) else {
        return Ok(false);
    };
    if hooks.get("enabled").and_then(TomlItem::as_bool) != Some(true) {
        return Ok(false);
    }
    let Some(entries) = hooks.get("hooks").and_then(TomlItem::as_array_of_tables) else {
        return Ok(false);
    };
    let has_managed_hook = entries.iter().any(is_managed_deepseek_hook);
    Ok(has_managed_hook)
}

fn append_managed_hooks(
    root: &mut Map<String, Value>,
    events: &[&str],
    command: &str,
) -> Result<()> {
    let hooks = hooks_object_mut(root)?;
    for event in events {
        let groups = hooks
            .entry((*event).to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .with_context(|| format!("hooks.{event} must be an array"))?;
        groups.push(json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": 3
            }]
        }));
    }
    Ok(())
}

fn remove_managed_hooks(
    root: &mut Map<String, Value>,
    agent: Option<warp_cli::CliAgentHookAgent>,
) -> Result<()> {
    let Some(hooks_value) = root.get_mut("hooks") else {
        return Ok(());
    };
    let hooks = hooks_value
        .as_object_mut()
        .context("hooks must be a JSON object")?;

    let event_names = hooks.keys().cloned().collect::<Vec<_>>();
    for event_name in event_names {
        let groups = hooks
            .get_mut(&event_name)
            .and_then(Value::as_array_mut)
            .with_context(|| format!("hooks.{event_name} must be an array"))?;
        for group in groups.iter_mut() {
            let group = group
                .as_object_mut()
                .with_context(|| format!("hooks.{event_name} entries must be objects"))?;
            let handlers = group
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .with_context(|| format!("hooks.{event_name}[].hooks must be an array"))?;
            for handler in handlers.iter() {
                if !handler.is_object() {
                    bail!("hooks.{event_name}[].hooks entries must be objects");
                }
            }
            handlers.retain(|handler| !is_managed_hook_handler_for_agent(handler, agent));
        }
        groups.retain(|group| {
            let group = group
                .as_object()
                .expect("hook groups were validated as objects");
            let has_handlers = group
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|handlers| !handlers.is_empty());
            let has_foreign_fields = group.keys().any(|key| key != "hooks");
            has_handlers || has_foreign_fields
        });
        if groups.is_empty() {
            hooks.remove(&event_name);
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(())
}

fn hooks_object_mut(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>> {
    root.entry("hooks".to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("hooks must be a JSON object")
}

fn is_managed_hook_handler(handler: &Value) -> bool {
    is_managed_hook_handler_for_agent(handler, None)
}

fn is_managed_hook_handler_for_agent(
    handler: &Value,
    agent: Option<warp_cli::CliAgentHookAgent>,
) -> bool {
    let Some(handler) = handler.as_object() else {
        return false;
    };
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    is_managed_hook_command(command, agent)
}

fn is_managed_hook_command(command: &str, agent: Option<warp_cli::CliAgentHookAgent>) -> bool {
    let Ok(tokens) = shell_words::split(command) else {
        return false;
    };
    let executable_is_zaplex = tokens
        .first()
        .and_then(|token| Path::new(token).file_stem())
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("zaplex"));
    let requested_agent = tokens
        .windows(2)
        .find_map(|pair| (pair[0] == "--agent").then_some(pair[1].as_str()));
    executable_is_zaplex
        && tokens.get(1).is_some_and(|token| token == "cli-agent-hook")
        && requested_agent.is_some_and(|requested| {
            agent
                .map(|agent| requested == agent.as_str())
                .unwrap_or_else(|| {
                    ["claude", "codex", "grok", "antigravity", "deepseek"].contains(&requested)
                })
        })
        && tokens
            .windows(2)
            .any(|pair| pair[0] == "--managed-by" && pair[1] == MANAGED_BY_MARKER)
}

fn has_managed_hooks(path: &Path, agent: warp_cli::CliAgentHookAgent) -> Result<bool> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let document: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(hooks) = document.get("hooks").and_then(Value::as_object) else {
        return Ok(false);
    };
    Ok(hooks.values().any(|groups| {
        groups.as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers
                            .iter()
                            .any(|handler| is_managed_hook_handler_for_agent(handler, Some(agent)))
                    })
            })
        })
    }))
}

fn mutate_toml_file(
    path: &Path,
    create_if_missing: bool,
    mutate: impl FnOnce(&mut TomlDocument) -> Result<()>,
) -> Result<HookConfigChange> {
    let existing_metadata = existing_config_metadata(path)?;
    if existing_metadata.is_none() && !create_if_missing {
        return Ok(HookConfigChange::Unchanged);
    }

    let source = if existing_metadata.is_some() {
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?
    } else {
        String::new()
    };
    let mut document = source
        .parse::<TomlDocument>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let before = document.to_string();
    mutate(&mut document)?;
    let rendered = document.to_string();
    if rendered == before {
        return Ok(HookConfigChange::Unchanged);
    }
    let mut bytes = rendered.into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    persist_atomic(path, &bytes, existing_metadata.as_ref())?;
    Ok(HookConfigChange::Changed)
}

fn mutate_json_file(
    path: &Path,
    create_if_missing: bool,
    mutate: impl FnOnce(&mut Map<String, Value>) -> Result<()>,
) -> Result<HookConfigChange> {
    let existing_metadata = existing_config_metadata(path)?;
    if existing_metadata.is_none() && !create_if_missing {
        return Ok(HookConfigChange::Unchanged);
    }

    let mut root = if existing_metadata.is_some() {
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        value
            .as_object()
            .cloned()
            .with_context(|| format!("{} must contain a JSON object", path.display()))?
    } else {
        Map::new()
    };
    let before = root.clone();
    mutate(&mut root)?;
    if root == before {
        return Ok(HookConfigChange::Unchanged);
    }

    let mut bytes =
        serde_json::to_vec_pretty(&Value::Object(root)).context("failed to serialize hooks")?;
    bytes.push(b'\n');
    persist_atomic(path, &bytes, existing_metadata.as_ref())?;
    Ok(HookConfigChange::Changed)
}

fn existing_config_metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!("refusing to replace symlinked hook configuration");
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn persist_atomic(
    path: &Path,
    bytes: &[u8],
    existing_metadata: Option<&fs::Metadata>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    if let Some(metadata) = existing_metadata {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .with_context(|| format!("failed to preserve permissions for {}", path.display()))?;
    }
    temporary
        .as_file_mut()
        .write_all(bytes)
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .as_file_mut()
        .flush()
        .with_context(|| format!("failed to flush temporary file for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "hook_bridge_tests.rs"]
mod tests;
