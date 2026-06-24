//! `RunShellCommand` adapter.
//!
//! Corresponds to `api::message::tool_call::Tool::RunShellCommand` in warp,
//! after execution result is `ToolCallResultType::RunShellCommand(RunShellCommandResult)`.

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use warp_multi_agent_api as api;

use super::OpenAiTool;

#[derive(Debug, Deserialize)]
struct Args {
    command: String,
    #[serde(default)]
    is_read_only: bool,
    #[serde(default)]
    uses_pager: bool,
    #[serde(default)]
    is_risky: bool,
    /// `None` (default / true) = wait for command completion before returning; `Some(false)` = return immediately
    /// after startup with LongRunningCommandSnapshot, later use read/write_to_long_running_*
    /// tools to continue interaction (suitable for dev server / tail -f type long-running commands).
    #[serde(default)]
    wait_until_complete: Option<bool>,
}

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": {
                "type": "string",
                "description": "Shell command to execute (complete command line)."
            },
            "is_read_only": {
                "type": "boolean",
                "description": "Is command read-only, not modifying filesystem/external state (no user confirmation needed when true).",
                "default": false
            },
            "uses_pager": {
                "type": "boolean",
                "description": "Does command trigger pager (less/more etc). Recommend false, can append | cat to avoid blocking.",
                "default": false
            },
            "is_risky": {
                "type": "boolean",
                "description": "Is command risky (rm -rf, change global config etc). Set true for user more visible confirmation.",
                "default": false
            },
            "wait_until_complete": {
                "type": "boolean",
                "description": "Default true (wait for command end before returning, suitable for one-shot commands). Commands like dev server / background process / tail -f / interactive REPL that don't naturally exit must set false, otherwise current turn hangs and never gets result. After setting false, immediately return LongRunningCommandSnapshot, later turns use read/write_to_long_running_shell_command to continue interaction.",
                "default": true
            }
        },
        "required": ["command"],
        "additionalProperties": false
    })
}

fn from_args(args: &str) -> Result<api::message::tool_call::Tool> {
    use api::message::tool_call::run_shell_command::WaitUntilCompleteValue;
    let parsed: Args = serde_json::from_str(args)?;
    // When None, explicitly default to true (wait for command completion before returning), avoid implicit default behavior
    // on controller end that could cause ambiguity across different warp versions/paths. Model must explicitly pass false to want long-running mode.
    let wait_until_complete_value = Some(WaitUntilCompleteValue::WaitUntilComplete(
        parsed.wait_until_complete.unwrap_or(true),
    ));
    Ok(api::message::tool_call::Tool::RunShellCommand(
        api::message::tool_call::RunShellCommand {
            command: parsed.command,
            is_read_only: parsed.is_read_only,
            uses_pager: parsed.uses_pager,
            is_risky: parsed.is_risky,
            citations: vec![],
            wait_until_complete_value,
            risk_category: 0,
        },
    ))
}

fn result_to_json(result: &api::message::tool_call_result::Result) -> Option<Value> {
    use api::message::tool_call_result::Result as R;
    use api::run_shell_command_result::Result as ShellR;
    let r = match result {
        R::RunShellCommand(r) => r,
        _ => return None,
    };
    let value = match &r.result {
        Some(ShellR::CommandFinished(f)) => json!({
            "status": "completed",
            "command": r.command,
            "exit_code": f.exit_code,
            "output": f.output,
        }),
        // Long-running command: started but not finished. Expose snapshot to model so it can
        // decide to continue reading (read_shell_command_output) or writing (write_to_long_running_*).
        Some(ShellR::LongRunningCommandSnapshot(s)) => json!({
            "status": "running",
            "command": r.command,
            "command_id": s.command_id,
            "output": s.output,
            "is_alt_screen_active": s.is_alt_screen_active,
        }),
        Some(ShellR::PermissionDenied(_)) => json!({
            "status": "permission_denied",
            "command": r.command,
        }),
        None => json!({ "status": "cancelled", "command": r.command }),
    };
    Some(value)
}

pub static RUN_SHELL_COMMAND: OpenAiTool = OpenAiTool {
    name: "run_shell_command",
    description: include_str!("../prompts/tool_descriptions/run_shell_command.md"),
    parameters,
    from_args,
    result_to_json,
};
