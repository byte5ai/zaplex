//! `webfetch` BYOP tool descriptor.
//!
//! The actual HTTP execution lives in `web_runtime::run_webfetch`. This descriptor is provided to the genai SDK
//! to send the tool description to the upstream LLM (name + description + JSON Schema).
//!
//! ## Does not go through the protobuf executor
//!
//! `from_args` always returns `Err("intercepted at byop layer")`, because `chat_stream::
//! parse_incoming_tool_call` matches by name beforehand and calls `web_runtime` directly. `result_to_json`
//! likewise always returns `None` (there is no corresponding protobuf result variant). These two stub functions only
//! satisfy the field constraints of the `OpenAiTool` struct.
//!
//! The parameter schema is aligned with opencode `webfetch.ts:12-20`.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use warp_multi_agent_api as api;

use super::OpenAiTool;

pub const TOOL_NAME: &str = "webfetch";

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "The URL to fetch content from. Must use HTTPS (https://)."
            },
            "format": {
                "type": "string",
                "enum": ["markdown", "text", "html"],
                "description": "Output format. 'markdown' (default) converts HTML to Markdown. 'text' strips formatting. 'html' returns the raw HTML.",
                "default": "markdown"
            },
            "timeout": {
                "type": "integer",
                "description": "Optional timeout in seconds. Default 30, capped at 120.",
                "minimum": 1,
                "maximum": 120
            }
        },
        "required": ["url"],
        "additionalProperties": false
    })
}

fn from_args(_args: &str) -> Result<api::message::tool_call::Tool> {
    Err(anyhow!(
        "webfetch is intercepted by chat_stream BYOP web tool dispatcher; \
         from_args should never be called"
    ))
}

fn result_to_json(_result: &api::message::tool_call_result::Result) -> Option<Value> {
    None
}

pub static WEBFETCH: OpenAiTool = OpenAiTool {
    name: TOOL_NAME,
    description: include_str!("../prompts/tool_descriptions/webfetch.md"),
    parameters,
    from_args,
    result_to_json,
};
