//! BYOP one-shot non-streaming completion adapter layer.
//!
//! Used for "proactive AI" sub-paths (prompt suggestions / NLD predict / relevant files /
//! conversation title generation etc): need to send one short request to get text,
//! **no tool calling, no streaming, no persistence to task.messages**.
//!
//! Differences from `chat_stream::generate_byop_output` (main conversation flow):
//! - Here uses `Client::exec_chat` (non-streaming), gets `ChatResponse::first_text()` once.
//! - No `RequestParams` / `ResponseEvent` / `task_store`, pure string in string out.
//! - reasoning disabled by default (proactive AI should not trigger reasoning chain — wastes tokens + slow),
//!   only inject per capability gate when `OneshotOptions.allow_reasoning = true`.
//!
//! Model selection decided by caller: `resolve_active_ai_oneshot()` decodes `active_ai_model`
//! (profile falls back to base_model) to BYOP `OneshotConfig`,
//! decode failure (no BYOP configured / model not in BYOP encoding space) → return `None`,
//! caller silent no-op.

use anyhow::Context as _;
use futures::StreamExt;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent};
use warpui::{AppContext, EntityId, SingletonEntity as _};

use super::chat_stream;
use crate::ai::llms::LLMPreferences;
use crate::settings::{AgentProviderApiType, ReasoningEffortSetting};

/// Provider/model info needed for BYOP one-shot request.
#[derive(Debug, Clone)]
pub struct OneshotConfig {
    pub base_url: String,
    pub api_key: String,
    pub model_id: String,
    pub api_type: AgentProviderApiType,
    pub reasoning_effort: ReasoningEffortSetting,
}

/// Optional parameters for one-shot call.
#[derive(Debug, Clone, Default)]
pub struct OneshotOptions {
    /// User message character truncation limit (by char, protects CJK). `None` = default 8000.
    pub max_chars: Option<usize>,
    /// Temperature (genai `ChatOptions::temperature`), `None` = provider default.
    pub temperature: Option<f32>,
    /// Whether to request JSON output (OpenAI-compatible provider uses response_format).
    /// Note: unsupported adapters ignore this parameter, system prompt must self-require JSON.
    pub response_format_json: bool,
    /// Whether to allow reasoning. Default `false` (proactive AI is low-latency lightweight calls).
    pub allow_reasoning: bool,
}

const DEFAULT_MAX_CHARS: usize = 8000;

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    s.chars().take(max).collect()
}

fn build_oneshot_request(
    cfg: &OneshotConfig,
    system: &str,
    user: &str,
    opts: &OneshotOptions,
) -> (ChatRequest, ChatOptions) {
    let mut chat_opts = ChatOptions::default()
        .with_capture_content(true)
        .with_capture_usage(true);
    if let Some(t) = opts.temperature {
        chat_opts = chat_opts.with_temperature(t.into());
    }
    if opts.response_format_json {
        chat_opts = chat_opts.with_response_format(genai::chat::ChatResponseFormat::JsonMode);
    }
    if opts.allow_reasoning {
        if let Some(effort) = cfg.reasoning_effort.to_genai() {
            if super::reasoning::model_supports_reasoning(cfg.api_type, &cfg.model_id) {
                chat_opts = chat_opts.with_reasoning_effort(effort);
            }
        }
    }

    let max_chars = opts.max_chars.unwrap_or(DEFAULT_MAX_CHARS);
    let user_truncated = truncate_chars(user, max_chars);

    let chat_req = ChatRequest::from_messages(vec![ChatMessage::user(user_truncated)])
        .with_system(system.to_owned());

    (chat_req, chat_opts)
}

/// Send one BYOP non-streaming chat completion, return plain text of model reply.
///
/// Error handling decided by caller — only propagate `anyhow::Error` here, no logging.
pub async fn byop_oneshot_completion(
    cfg: &OneshotConfig,
    system: &str,
    user: &str,
    opts: &OneshotOptions,
) -> anyhow::Result<String> {
    let client = chat_stream::build_client(cfg.api_type, cfg.base_url.clone(), cfg.api_key.clone());
    let (chat_req, chat_opts) = build_oneshot_request(cfg, system, user, opts);

    let resp = client
        .exec_chat(&cfg.model_id, chat_req, Some(&chat_opts))
        .await
        .with_context(|| format!("byop oneshot exec_chat failed (model={})", cfg.model_id))?;

    Ok(resp.first_text().unwrap_or("").to_owned())
}

/// Send one BYOP streaming chat completion, aggregate all text chunks and return.
///
/// For OpenAI Responses-compatible proxies that only accept `stream=true`. Caller still gets complete
/// string, so can reuse one-shot title cleanup / JSON parsing logic.
pub async fn byop_oneshot_streaming_completion(
    cfg: &OneshotConfig,
    system: &str,
    user: &str,
    opts: &OneshotOptions,
) -> anyhow::Result<String> {
    let client = chat_stream::build_client(cfg.api_type, cfg.base_url.clone(), cfg.api_key.clone());
    let (chat_req, chat_opts) = build_oneshot_request(cfg, system, user, opts);
    let mut resp = client
        .exec_chat_stream(&cfg.model_id, chat_req, Some(&chat_opts))
        .await
        .with_context(|| {
            format!(
                "byop oneshot exec_chat_stream failed (model={})",
                cfg.model_id
            )
        })?
        .stream;

    let mut text = String::new();
    while let Some(event) = resp.next().await {
        match event.with_context(|| {
            format!(
                "byop oneshot exec_chat_stream event failed (model={})",
                cfg.model_id
            )
        })? {
            ChatStreamEvent::Chunk(chunk) => {
                text.push_str(&chunk.content);
            }
            ChatStreamEvent::Start
            | ChatStreamEvent::ReasoningChunk(_)
            | ChatStreamEvent::ThoughtSignatureChunk(_)
            | ChatStreamEvent::ToolCallChunk(_)
            | ChatStreamEvent::End(_) => {}
        }
    }

    Ok(text)
}

/// Parse current active profile's `active_ai_model` (fallback to `base_model`),
/// if decode as valid BYOP encoding → return `OneshotConfig`, else `None` (caller silent no-op).
pub fn resolve_active_ai_oneshot(
    app: &AppContext,
    terminal_view_id: Option<EntityId>,
) -> Option<OneshotConfig> {
    let llm_prefs = LLMPreferences::as_ref(app);
    let id = llm_prefs
        .get_active_ai_model(app, terminal_view_id)
        .id
        .clone();
    let (provider, api_key, model_id) = super::lookup_byop(app, &id)?;
    let reasoning_effort =
        llm_prefs.get_reasoning_effort(terminal_view_id, provider.api_type, &model_id);
    Some(OneshotConfig {
        base_url: provider.base_url,
        api_key,
        model_id,
        api_type: provider.api_type,
        reasoning_effort,
    })
}

/// Parse current active profile's `next_command_model` (fallback to `base_model`),
/// if decode as valid BYOP encoding → return `OneshotConfig`, else `None`.
pub fn resolve_next_command_oneshot(
    app: &AppContext,
    terminal_view_id: Option<EntityId>,
) -> Option<OneshotConfig> {
    let llm_prefs = LLMPreferences::as_ref(app);
    let id = llm_prefs
        .get_active_next_command_model(app, terminal_view_id)
        .id
        .clone();
    let (provider, api_key, model_id) = super::lookup_byop(app, &id)?;
    let reasoning_effort =
        llm_prefs.get_reasoning_effort(terminal_view_id, provider.api_type, &model_id);
    Some(OneshotConfig {
        base_url: provider.base_url,
        api_key,
        model_id,
        api_type: provider.api_type,
        reasoning_effort,
    })
}
