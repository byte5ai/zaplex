//! Two-way translation registry for OpenAI tool calling in BYOP mode.
//!
//! Each built-in warp tool (a variant of `api::message::tool_call::Tool`) corresponds to one
//! [`OpenAiTool`] descriptor: function name + JSON Schema + reverse-parsing args + serializing execution
//! results into strings for the upstream model.
//!
//! ## Current subset of implemented tools (Phase 3a first batch)
//!
//! - `run_shell_command`
//! - `read_files`
//!
//! Future iterations will add: `grep` / `file_glob_v2` / `apply_file_diffs` / `call_mcp_tool`, etc.
//!
//! ## Closed-loop flow explanation
//!
//! Model returns `tool_calls` → `from_args` translates to `tool_call::Tool` → we emit
//! `Message::ToolCall { tool_call_id, tool }` → warp's own `convert_from.rs`
//! auto-translates to `AIAgentAction` → executor checks profile permissions/shows dialog → executes → result
//! auto-written back to conversation → triggers next byop request → our `result_to_json`
//! serializes result as `role=tool, tool_call_id=...` content for upstream.

pub mod ask;
pub mod coerce;
pub mod documents;
pub mod edit;
pub mod exa;
pub mod files;
pub mod long_shell;
pub mod markers;
pub mod mcp;
pub mod search;
pub mod shell;
pub mod skill;
pub mod suggest;
pub mod todowrite;
pub mod web_runtime;
pub mod webfetch;
pub mod websearch;

use anyhow::Result;
use serde_json::Value;
use warp_multi_agent_api as api;

use crate::ai::agent::AIAgentActionResult;

/// Two-way adapter description for a single tool.
///
/// **Naming history**: Originally BYOP only accepted OpenAI-compatible protocol, then switched to using
/// genai SDK across 5 adapters (OpenAI / OpenAIResp / Gemini / Anthropic / Ollama). Struct name retains
/// `OpenAiTool` to preserve git blame, but the underlying JSON Schema follows OpenAPI standard; each
/// adapter is internally auto-rewritten by genai into its native format (e.g., Anthropic input_schema,
/// Gemini function_declarations).
pub struct OpenAiTool {
    /// Function name for the upstream LLM (model invokes by this name in responses).
    pub name: &'static str,
    /// Description for the LLM.
    pub description: &'static str,
    /// Parameter JSON Schema (OpenAPI standard). Returns a closure to avoid constructing serde_json::Value in const context.
    pub parameters: fn() -> Value,
    /// Reverse-parsing: upstream model's returned args JSON string → warp internal `tool_call::Tool` variant.
    pub from_args: fn(args: &str) -> Result<api::message::tool_call::Tool>,
    /// Converts the `Result` variant in ToolCallResult corresponding to this tool into JSON readable by the upstream model.
    /// Returns `None` when no matching variant is found (allowing caller to fall back to generic serialization).
    pub result_to_json: fn(&api::message::tool_call_result::Result) -> Option<Value>,
}

impl OpenAiTool {
    /// Convert to genai `Tool` (for feeding to `ChatRequest.tools`).
    pub fn to_genai_tool(&self) -> genai::chat::Tool {
        genai::chat::Tool::new(self.name)
            .with_description(self.description)
            .with_schema((self.parameters)())
    }
}

/// Registry: all supported BYOP tools.
pub const REGISTRY: &[&OpenAiTool] = &[
    &shell::RUN_SHELL_COMMAND,
    &files::READ_FILES,
    &search::GREP,
    &search::FILE_GLOB_V2,
    &edit::APPLY_FILE_DIFFS,
    &long_shell::WRITE_TO_LONG_RUNNING_SHELL_COMMAND,
    &long_shell::READ_SHELL_COMMAND_OUTPUT,
    &ask::ASK_USER_QUESTION,
    &skill::READ_SKILL,
    // Local document system (AIDocumentModel)
    &documents::READ_DOCUMENTS,
    &documents::EDIT_DOCUMENTS,
    &documents::CREATE_DOCUMENTS,
    // User suggestions (local channel + UI)
    &suggest::SUGGEST_NEW_CONVERSATION,
    &suggest::SUGGEST_PROMPT,
    // UI markers (no side effects, signal to front-end)
    &markers::OPEN_CODE_REVIEW,
    &markers::TRANSFER_SHELL_CONTROL,
    // Local todo list (BYOP auto-synthesizes Message::UpdateTodos, does not go through protobuf executor)
    &todowrite::TODOWRITE,
    // BYOP-only network tools: not mapped to protobuf executor variant, intercepted by chat_stream
    // before parse_incoming_tool_call by name, directly calls web_runtime for HTTP.
    // Gating: when profile.web_search_enabled=false, build_tools_array will filter these out.
    &webfetch::WEBFETCH,
    &websearch::WEBSEARCH,
];

/// Reverse-lookup in registry by OpenAI function name.
pub fn lookup(name: &str) -> Option<&'static OpenAiTool> {
    REGISTRY.iter().copied().find(|t| t.name == name)
}

/// Given a ToolCallResult, first try to find the corresponding tool in REGISTRY and serialize using its `result_to_json`;
/// if not found, attempt MCP generic serialization; as last resort, fall back to a short description to avoid panic.
pub fn serialize_result(result: &api::message::ToolCallResult) -> String {
    let inner = match &result.result {
        Some(r) => r,
        None => return r#"{"status":"cancelled"}"#.to_owned(),
    };
    for t in REGISTRY {
        if let Some(json) = (t.result_to_json)(inner) {
            return serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_owned());
        }
    }
    if let Some(json) = mcp::serialize_result(inner) {
        return serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_owned());
    }
    // Fallback: unrecognized variant (tools not yet registered in subsequent rounds also fall through here).
    r#"{"status":"unsupported_tool_result"}"#.to_owned()
}

/// Serialize an `AIAgentActionResult` completed by *current-round client execution* into a JSON string
/// to feed to the upstream model (as role=tool content).
///
/// ## Why not use `AIAgentActionResultType::Display` directly?
///
/// `Display` impl renders structured results (especially `LongRunningCommandSnapshot`) into
/// single-line strings like `"Command 'bun repl' is long-running"`, **completely discarding critical fields
/// like block_id (=command_id), grid_contents, is_alt_screen_active**, causing the next model round to lose
/// the command_id and be unable to continue read/write_to_long_running_*, rendering long-running commands unusable.
///
/// ## How it works
///
/// 1. Reuse existing `TryFrom<AIAgentActionResult> for api::request::input::user_inputs::user_input::Input`
///    in `app/src/ai/agent/api/convert_to.rs` (covers 25+ ActionResult variants), obtain `Input::ToolCallResult { result, .. }`
/// 2. Inner `*Result` types (e.g., `RunShellCommandResult`) and `api::message::tool_call_result::Result`
///    share the same protobuf message; only the outer enum namespace differs, so rewrap the outer enum
///    and reuse per-tool `result_to_json` from `tools::REGISTRY`
///    (see `shell.rs::result_to_json` serializing `LongRunningCommandSnapshot` into complete JSON
///    including command_id/output/is_alt_screen_active)
/// 3. Return `None` for unrecognized variants; caller falls back to Display
///
/// ## Maintenance note
///
/// When adding a new BYOP tool, **the enum match here must be updated with the variant**, otherwise that tool's
/// current-round ActionResult will fall back to Display, losing structured fields.
pub fn serialize_action_result(action: &AIAgentActionResult) -> Option<String> {
    let msg_side = action_result_to_msg_result(action)?;
    for t in REGISTRY {
        if let Some(json) = (t.result_to_json)(&msg_side) {
            return Some(serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_owned()));
        }
    }
    if let Some(json) = mcp::serialize_result(&msg_side) {
        return Some(serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_owned()));
    }
    None
}

/// Convert an `AIAgentActionResult` completed by current-round client execution to
/// `api::message::tool_call_result::Result` enum for BYOP to persist as task.message.
///
/// Shares the ReqR → MsgR mapping from `serialize_action_result`; caller wraps the result as
/// `Message::ToolCallResult { result: Some(...), context: None, tool_call_id }`.
pub fn action_result_to_msg_result(
    action: &AIAgentActionResult,
) -> Option<api::message::tool_call_result::Result> {
    use api::message::tool_call_result::Result as MsgR;
    use api::request::input::tool_call_result::Result as ReqR;
    use api::request::input::user_inputs::user_input::Input;

    let input: Input = action.clone().try_into().ok()?;
    let req_input: ReqR = match input {
        Input::ToolCallResult(tcr) => tcr.result?,
        _ => return None,
    };
    let msg_side = match req_input {
        ReqR::RunShellCommand(r) => MsgR::RunShellCommand(r),
        ReqR::WriteToLongRunningShellCommand(r) => MsgR::WriteToLongRunningShellCommand(r),
        ReqR::ReadShellCommandOutput(r) => MsgR::ReadShellCommandOutput(r),
        ReqR::ReadFiles(r) => MsgR::ReadFiles(r),
        ReqR::Grep(r) => MsgR::Grep(r),
        ReqR::FileGlobV2(r) => MsgR::FileGlobV2(r),
        ReqR::ApplyFileDiffs(r) => MsgR::ApplyFileDiffs(r),
        ReqR::CallMcpTool(r) => MsgR::CallMcpTool(r),
        ReqR::ReadMcpResource(r) => MsgR::ReadMcpResource(r),
        ReqR::AskUserQuestion(r) => MsgR::AskUserQuestion(r),
        ReqR::ReadSkill(r) => MsgR::ReadSkill(r),
        ReqR::ReadDocuments(r) => MsgR::ReadDocuments(r),
        ReqR::EditDocuments(r) => MsgR::EditDocuments(r),
        ReqR::CreateDocuments(r) => MsgR::CreateDocuments(r),
        ReqR::SuggestNewConversation(r) => MsgR::SuggestNewConversation(r),
        ReqR::SuggestPrompt(r) => MsgR::SuggestPrompt(r),
        ReqR::OpenCodeReview(r) => MsgR::OpenCodeReview(r),
        ReqR::TransferShellCommandControlToUser(r) => MsgR::TransferShellCommandControlToUser(r),
        _ => return None,
    };
    Some(msg_side)
}
