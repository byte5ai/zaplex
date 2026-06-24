//! Heuristic detection of a model's reasoning (chain-of-thought) capability.
//!
//! Background: in genai 0.6, the individual adapters do **not** capability-gate
//! models — as long as `ChatOptions::reasoning_effort` is non-empty, they inject
//! the thinking parameters regardless. For **models that do not support reasoning**
//! (claude-3-5-haiku / gpt-4o / gemini-1.5-pro) this makes the upstream API return
//! a 400 outright, so the client side must do the detection itself.
//!
//! The detection strategy follows opencode's `provider/transform.ts::variants()`
//! "hard-coded + substring match" approach: the model id a BYOP user fills in is an
//! arbitrary string, so we cannot rely on registry metadata and can only match
//! naming conventions.
//!
//! References:
//! - genai 0.6 anthropic adapter's SUPPORT_EFFORT_MODELS / SUPPORT_ADAPTTIVE_THINK_MODELS
//! - opencode v5's anthropicAdaptiveEfforts / OPENAI_EFFORTS lists
//! - the thinking-mode model lists in each provider's official documentation

use crate::settings::{AgentProviderApiType, ReasoningEffortSetting};
use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

/// Returns the list of reasoning effort levels actually available for the given
/// (api_type, model_id).
///
/// Empty list → the picker is hidden entirely (reasoning unsupported, or the client
/// cannot reliably inject it).
/// First item → the recommended default level for this model (the initial value the
/// first time the picker appears).
/// The last item is always [`ReasoningEffortSetting::Off`], meaning "explicitly turn
/// off thinking" (for models that support effort it sends the `none` level; for the
/// budget family it skips the thinking field).
///
/// Designed after opencode's `provider/transform.ts::variants()` — each provider's
/// levels are hard-coded, not sourced from models.dev. models.dev only gives a
/// "supports reasoning" boolean; the concrete levels are built into the client.
pub fn model_reasoning_variants(
    api_type: AgentProviderApiType,
    model_id: &str,
) -> Vec<ReasoningEffortSetting> {
    use ReasoningEffortSetting as R;
    let id = strip_effort_suffix(&model_id.to_ascii_lowercase()).to_string();

    match api_type {
        AgentProviderApiType::Anthropic => {
            if is_opus_4_7_or_higher(&id) {
                // Opus 4.7+: adaptive thinking + xhigh + max (supported by genai)
                return vec![R::High, R::Low, R::Medium, R::XHigh, R::Max, R::Off];
            }
            if id.contains("claude-opus-4-6") || id.contains("claude-sonnet-4-6") {
                // 4.6 series: adaptive thinking + max
                return vec![R::High, R::Low, R::Medium, R::Max, R::Off];
            }
            if is_anthropic_reasoning_model(&id) {
                // Legacy budget such as 4.5 / 3.7-sonnet, no max
                return vec![R::High, R::Low, R::Medium, R::Off];
            }
            vec![]
        }
        AgentProviderApiType::OpenAi | AgentProviderApiType::OpenAiResp => {
            if id.contains("gpt-5") || id.contains("codex") {
                // GPT-5 / codex: both minimal and xhigh are available
                return vec![R::Medium, R::Minimal, R::Low, R::High, R::XHigh, R::Off];
            }
            if is_openai_reasoning_model(&id) {
                // o-series: low/medium/high only
                return vec![R::Medium, R::Low, R::High, R::Off];
            }
            vec![]
        }
        AgentProviderApiType::Gemini => {
            if is_gemini_reasoning_model(&id) {
                // genai 0.6 uniformly sends a thinkingBudget value; 2.5/3.x do not distinguish levels
                return vec![R::Medium, R::Low, R::High, R::Off];
            }
            vec![]
        }
        // DeepSeek thinking-mode models (deepseek-reasoner / v4 / thinking / r1).
        // Zap's local fork (`lib/rust-genai`) relaxes the injection condition in
        // adapter_shared.rs so the top-level `reasoning_effort` field is sent per the
        // DeepSeek thinking_mode documentation.
        //
        // Ollama backend model ids are arbitrary; left empty conservatively.
        AgentProviderApiType::DeepSeek => {
            if is_deepseek_thinking_model(&id) {
                // DeepSeek officially offers only two thinking depths, high / max
                // (even if the server-side deserializer accepts low/medium/xhigh they are
                // just aliases of the same level, so the picker does not expose redundant items).
                // The Off level goes through "turn off thinking": the local genai fork now
                // supports ChatOptions::extra_body, and on DeepSeek+Off chat_stream instead sends
                // `extra_body = {"thinking": {"type": "disabled"}}` merged at the top level.
                vec![R::High, R::Max, R::Off]
            } else {
                vec![]
            }
        }
        AgentProviderApiType::Ollama => vec![],
    }
}

/// The recommended default level for this model (the initial value the first time the
/// picker appears); None means the model does not support reasoning.
pub fn default_reasoning_for(
    api_type: AgentProviderApiType,
    model_id: &str,
) -> Option<ReasoningEffortSetting> {
    model_reasoning_variants(api_type, model_id)
        .first()
        .copied()
}

/// Opus 4.7 and higher versions (`claude-opus-4-7` / `claude-opus-5-0` ...).
/// Same semantics as the `is_opus_4_7_or_higher` regex in the genai anthropic adapter.
fn is_opus_4_7_or_higher(model_name: &str) -> bool {
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"claude-opus-(\d+)-(\d+)").ok());
    let Some(re) = re.as_ref() else {
        return false;
    };
    let Some(caps) = re.captures(model_name) else {
        return false;
    };
    let major = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok());
    let minor = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if (major, minor) >= (4, 7))
}

/// Determines whether the given (api_type, model_name) combination supports reasoning
/// (chain-of-thought).
///
/// Only when this returns `true` is `reasoning_effort` injected into genai; otherwise an
/// ordinary chat request is sent as-is, avoiding the upstream rejection that occurs when
/// thinking parameters are injected for older models (such as claude-3-5-haiku / gpt-4o).
///
/// Naming conventions follow each provider's model id style (lowercased, then substring
/// match):
/// - **Anthropic**: `claude-opus-4` / `claude-sonnet-4` / `claude-haiku-4` /
///   `claude-3-7-sonnet` (the starting point of extended thinking) and newer versions
/// - **OpenAI / OpenAIResp**: the `o1` / `o3` / `o4` series, `gpt-5`, `codex`
/// - **Gemini**: `gemini-2.5*` / `gemini-3*` (thinking from 2.5 onward, the entire 3.x family)
/// - **DeepSeek**: `deepseek-reasoner` / `deepseek-r1` / `deepseek-v4*` /
///   `deepseek-thinking` (officially two levels: high / max go through the top-level
///   `reasoning_effort` field, the Off level turns off thinking via
///   `extra_body.thinking.type=disabled`)
/// - **Ollama**: goes through the OpenAI-compatible path; the backend model id is not
///   controllable, so it **conservatively returns `false`** (if the user really is running
///   a thinking model, they can set the level explicitly in Settings; this can be relaxed later)
pub fn model_supports_reasoning(api_type: AgentProviderApiType, model_id: &str) -> bool {
    !model_reasoning_variants(api_type, model_id).is_empty()
}

fn strip_effort_suffix(id: &str) -> &str {
    if let Some((prefix, last)) = id.rsplit_once('-') {
        if matches!(
            last,
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "zero"
        ) {
            return prefix;
        }
    }
    id
}

fn is_anthropic_reasoning_model(id: &str) -> bool {
    // claude-3-7-sonnet is the starting point of extended thinking (released 2025-02).
    if id.contains("claude-3-7-sonnet") {
        return true;
    }
    // The entire claude-opus-4* / claude-sonnet-4* / claude-haiku-4* family is supported.
    // Also tolerates all three separator styles: `4.5` / `4-5` / `4_5`.
    let four_series = ["claude-opus-4", "claude-sonnet-4", "claude-haiku-4"];
    if four_series.iter().any(|prefix| id.contains(prefix)) {
        return true;
    }
    false
}

fn is_openai_reasoning_model(id: &str) -> bool {
    // o-series reasoning models (o1 / o1-mini / o1-pro / o3 / o3-mini / o4 / o4-mini).
    // Note that `o1-mini` is excluded in opencode's azure case, but official OpenAI accepts
    // reasoning_effort, so it is kept here following upstream OpenAI behavior.
    let o_series_prefixes = ["o1", "o3", "o4"];
    for prefix in o_series_prefixes {
        if id == prefix
            || id.starts_with(&format!("{prefix}-"))
            || id.starts_with(&format!("{prefix}_"))
        {
            return true;
        }
    }
    // The GPT-5 series (the entire family does reasoning) + codex variants (gpt-5-codex / codex-* / o*-codex, etc.).
    if id.contains("gpt-5") || id.contains("codex") {
        return true;
    }
    false
}

fn is_deepseek_thinking_model(id: &str) -> bool {
    // DeepSeek thinking-mode model name conventions: reasoner / r1 / v4* / *-thinking.
    // The `deepseek-v4` substring covers later variants such as `deepseek-v4-flash`.
    id.contains("deepseek-reasoner")
        || id.contains("deepseek-v4")
        || id.contains("deepseek-thinking")
        || id.contains("deepseek-r1")
}

fn is_gemini_reasoning_model(id: &str) -> bool {
    // Thinking mode from gemini-2.5-* onward (flash-thinking-exp / pro / pro-thinking).
    // The entire gemini-3.* family (opencode distinguishes 3 / 3.1 at the levels layer).
    if id.contains("gemini-2.5") || id.contains("gemini-3") {
        return true;
    }
    // The legacy thinking exp channel (2.0 flash-thinking-exp counts too).
    if id.contains("thinking") {
        return true;
    }
    false
}

/// Aligned with opencode `model.capabilities.interleaved.field`
/// (`provider/provider.ts:1182-1187`, `provider/transform.ts:217-249`): some
/// thinking-mode models require past reasoning to be attached back onto the assistant
/// message under a specific field name.
///
/// opencode's two valid values are `"reasoning_content"` and `"reasoning_details"`:
/// - `reasoning_content`: the top-level string field used by the vast majority of
///   domestic OpenAI-compatible thinking models (DeepSeek/Kimi/MiMo/Qwen3/
///   GLM-thinking/MiniMax/Hunyuan/Ernie/Doubao …).
/// - `reasoning_details`: the array form used by aggregator providers such as OpenRouter;
///   the genai 0.6 OpenAI adapter does not yet support it (it can only hoist the top-level
///   `reasoning_content` string) — kept as an enum placeholder, and when matched it falls
///   back to serializing as `ReasoningContent` (enough to cover most compatible endpoints).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReasoningInterleavedField {
    /// The top-level `reasoning_content` string field.
    ReasoningContent,
    /// The top-level `reasoning_details` array field (reserved; the current serialization path falls back).
    ReasoningDetails,
}

/// Substring match table of model_ids for domestic / third-party OpenAI-compatible
/// thinking models.
///
/// Designed after the `capabilities.interleaved` data field in opencode's `models.dev` —
/// each thinking model explicitly declares its field in the catalog, and the client looks
/// up the model in the table to decide the echo-back form. warp has no external catalog, so
/// the table is hard-coded here; it can later be made a configurable override.
///
/// Rule: **a match occurs when the lowercased model_id contains the needle as a substring**.
/// Order does not matter (short and long substrings do not shadow each other; the first
/// match is enough). Maintenance just requires adding a row to the table without changing
/// the control flow.
const INTERLEAVED_RULES: &[(&str, ReasoningInterleavedField)] = {
    use ReasoningInterleavedField::ReasoningContent as RC;
    &[
        // The entire DeepSeek thinking family (users often configure the official OpenAI-compatible endpoint as OpenAi api_type)
        ("deepseek-reasoner", RC),
        ("deepseek-v4", RC),
        ("deepseek-r1", RC),
        ("deepseek-thinking", RC),
        // Moonshot Kimi series
        ("kimi", RC),
        ("moonshot", RC),
        // Xiaomi MiMo (source of the bug issue: `mimo-v2.5-pro`)
        ("mimo", RC),
        // Alibaba Qwen thinking / QwQ (DashScope OpenAI-compatible endpoint + enable_thinking)
        ("qwen3", RC),
        ("qwq", RC),
        // Zhipu GLM thinking (z.ai / Zhipu Open Platform)
        ("zai-glm", RC),
        ("glm-4.5-thinking", RC),
        ("glm-4.6-thinking", RC),
        ("glm-4.7", RC),
        // MiniMax M1 thinking (uses the reasoning_content field)
        ("minimax-m1", RC),
        // MiniMax M3: reasoning is transmitted via <think> tags embedded in content;
        // the multi-turn echo format is still to be confirmed (RC vs. <think>-in-content), so no RC entry is added for now.
        // The display fix is handled by the model_uses_think_tags_in_content allowlist + streaming extraction.
        // Tencent Hunyuan T1 thinking
        ("hunyuan-t1", RC),
        // Baidu Ernie X1 / thinking
        ("ernie-x1", RC),
        ("ernie-thinking", RC),
        // StepFun Step thinking
        ("step-r-mini", RC),
        ("step-thinking", RC),
        // ByteDance Doubao thinking
        ("doubao-thinking", RC),
        ("doubao-1-5-thinking", RC),
        // 01.AI Yi thinking
        ("yi-thinking", RC),
    ]
};

/// Allowlist of OpenAI-compatible thinking models that transmit reasoning embedded in
/// `/delta/content` as `<think>...</think>` tags (rather than in a separate
/// `/delta/reasoning_content` field).
///
/// For models matching this table, the chat_stream streaming layer performs `<think>` tag
/// extraction on Chunk events, routing the tag contents to the reasoning channel to be
/// displayed as a grey thinking block.
/// Models that do not match retain the original text output behavior, avoiding accidentally
/// swallowing normal output that contains a literal `<think>`.
const THINK_TAG_IN_CONTENT_MODELS: &[&str] = &[
    // MiniMax M3: reasoning is transmitted via <think> tags in content.
    "minimax-m3",
];

/// Returns whether the given model conveys reasoning via `<think>` tags in content
/// (rather than the reasoning_content field).
///
/// The chat_stream streaming layer uses this function to decide whether to perform
/// `<think>` tag extraction on Chunk events.
pub fn model_uses_think_tags_in_content(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    THINK_TAG_IN_CONTENT_MODELS
        .iter()
        .any(|&needle| id.contains(needle))
}

/// Runtime latch set: records which (api_type, model_id) emitted a `ReasoningChunk` during
/// some stream — i.e. a precise heuristic signal that "this endpoint's server recognizes
/// the reasoning_content field".
///
/// This is the key difference from opencode: opencode statically declares
/// `capabilities.interleaved` via the external `models.dev` catalog; warp has no catalog and
/// instead uses stream probing — an endpoint that has emitted a reasoning chunk necessarily
/// recognizes reasoning_content, while strict providers such as **Cerebras / Groq /
/// OpenRouter / Together AI / SambaNova** that do not emit that chunk are never latched,
/// automatically avoiding the kind of spurious 400 in zerx-lab/warp #25.
///
/// The signal is kept in memory only across streams/turns and is cleared on process restart
/// (it will re-latch the next time a reasoning chunk is seen). It is only meaningful for the
/// OpenAi / OpenAiResp api_type — the entire DeepSeek adapter echoes by default; Anthropic /
/// Gemini each go through thinking blocks / thought signatures and do not need the top-level
/// `reasoning_content` field even if the stream emits a reasoning chunk.
static REASONING_ECHO_LATCH: OnceLock<RwLock<HashSet<(AgentProviderApiType, String)>>> =
    OnceLock::new();

fn latch_set() -> &'static RwLock<HashSet<(AgentProviderApiType, String)>> {
    REASONING_ECHO_LATCH.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Called when a `ReasoningChunk` is received during a stream, marking (api_type, lowercased
/// model_id) as "needs to echo back reasoning_content". On the next
/// [`model_reasoning_interleaved`] / [`model_requires_reasoning_echo`] query it preferentially
/// returns `Some(ReasoningContent)` / `true`, regardless of whether it is in the static
/// [`INTERLEAVED_RULES`] table.
///
/// Only actually persists writes for the OpenAi / OpenAiResp api_type (other api_types already
/// have native reasoning channels, so latching yields no benefit and would pollute the set);
/// all other paths return quickly.
pub fn note_reasoning_seen(api_type: AgentProviderApiType, model_id: &str) {
    if !matches!(
        api_type,
        AgentProviderApiType::OpenAi | AgentProviderApiType::OpenAiResp
    ) {
        return;
    }
    let key = (api_type, model_id.to_ascii_lowercase());
    if let Ok(s) = latch_set().read() {
        if s.contains(&key) {
            return;
        }
    }
    if let Ok(mut s) = latch_set().write() {
        s.insert(key);
    }
}

fn latch_contains(api_type: AgentProviderApiType, model_id_lower: &str) -> bool {
    latch_set()
        .read()
        .map(|s| s.contains(&(api_type, model_id_lower.to_string())))
        .unwrap_or(false)
}

/// For testing: clears the latch. Production code should not call this.
#[cfg(test)]
fn reset_reasoning_latch() {
    if let Ok(mut s) = latch_set().write() {
        s.clear();
    }
}

/// Looks up the reasoning interleaved field the model should use; `None` means this endpoint
/// should not echo back `reasoning_content` — even if the stream received real reasoning, it
/// is discarded on replay, avoiding rejection with a 400 `wrong_api_format` by strict-schema
/// providers such as **Cerebras / Groq / OpenRouter / Together AI / SambaNova / official OpenAI**.
///
/// Aligned with the `capabilities.interleaved` semantics in opencode
/// `provider/transform.ts:217-249`, enhanced into a two-stage decision (precision first →
/// recall fallback):
///
/// 1. **Runtime latch** (precise): this (api_type, model_id) emitted a `ReasoningChunk` in a
///    past stream → this endpoint's server necessarily recognizes the reasoning_content field →
///    return `Some(ReasoningContent)`. Covers any domestic / third-party thinking model outside
///    the [`INTERLEAVED_RULES`] table, with no allowlist to maintain.
/// 2. **Static hint** (cold start): when the latch does not match, fall back to looking up the
///    [`INTERLEAVED_RULES`] substring table and the api_type default:
///    - **DeepSeek api_type**: the entire adapter is DeepSeek-specific, so all models echo
///      (consistent with opencode's default `apiID.includes("deepseek") → { field: "reasoning_content" }`)
///    - **OpenAI / OpenAiResp**: use the substring table, covering mainstream domestic thinking models
///    - **Anthropic / Gemini / Ollama**: `None` (Anthropic goes through thinking blocks,
///      Gemini through thought signatures, Ollama through native reasoning; none of them need this echo)
pub fn model_reasoning_interleaved(
    api_type: AgentProviderApiType,
    model_id: &str,
) -> Option<ReasoningInterleavedField> {
    use AgentProviderApiType as T;
    let id = model_id.to_ascii_lowercase();
    // (1) Runtime latch — if a previous stream emitted a reasoning chunk, lock in echo
    if matches!(api_type, T::OpenAi | T::OpenAiResp) && latch_contains(api_type, &id) {
        return Some(ReasoningInterleavedField::ReasoningContent);
    }
    // (2) Static hint — fallback for cold start / first turn (not yet streamed)
    match api_type {
        T::DeepSeek => Some(ReasoningInterleavedField::ReasoningContent),
        T::OpenAi | T::OpenAiResp => INTERLEAVED_RULES
            .iter()
            .find(|(needle, _)| id.contains(needle))
            .map(|(_, f)| *f),
        T::Anthropic | T::Gemini | T::Ollama => None,
    }
}

/// Determines whether the given (api_type, model_id) needs to echo back the
/// `reasoning_content` field on every assistant message (including an empty-string
/// placeholder). Equivalent to [`model_reasoning_interleaved`] `.is_some()`; the old name is
/// kept for compatibility with existing call sites.
///
/// Background: new-generation thinking-mode models such as `deepseek-v4-flash` /
/// `mimo-v2.5-pro` tightened server-side validation from "an assistant containing only
/// tool_calls must carry reasoning_content" to "in thinking-mode every assistant must carry
/// reasoning_content, and its absence is a 400
/// `The reasoning_content in the thinking mode must be passed back to the API`".
/// The genai 0.6 serialization layer (`adapter_shared.rs:368-373`) only echoes an existing
/// `ContentPart::ReasoningContent` and **does not auto-fill missing ones**, so the client
/// layer must forcibly attach the placeholder field (an empty string works too — genai
/// inserts it as-is and the server only validates field presence).
pub fn model_requires_reasoning_echo(api_type: AgentProviderApiType, model_id: &str) -> bool {
    model_reasoning_interleaved(api_type, model_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_supported() {
        let t = AgentProviderApiType::Anthropic;
        assert!(model_supports_reasoning(t, "claude-opus-4-5"));
        assert!(model_supports_reasoning(t, "claude-sonnet-4-6"));
        assert!(model_supports_reasoning(t, "claude-opus-4-7"));
        assert!(model_supports_reasoning(t, "claude-3-7-sonnet-20250219"));
        // The suffix does not affect detection
        assert!(model_supports_reasoning(t, "claude-sonnet-4-5-high"));
        assert!(model_supports_reasoning(t, "claude-opus-4-7-max"));
    }

    #[test]
    fn anthropic_unsupported() {
        let t = AgentProviderApiType::Anthropic;
        assert!(!model_supports_reasoning(t, "claude-3-5-haiku-20241022"));
        assert!(!model_supports_reasoning(t, "claude-3-5-sonnet-20241022"));
        assert!(!model_supports_reasoning(t, "claude-3-opus-20240229"));
        assert!(!model_supports_reasoning(t, "claude-2.1"));
    }

    #[test]
    fn openai_supported() {
        let t = AgentProviderApiType::OpenAi;
        assert!(model_supports_reasoning(t, "o1"));
        assert!(model_supports_reasoning(t, "o1-mini"));
        assert!(model_supports_reasoning(t, "o3-mini"));
        assert!(model_supports_reasoning(t, "o4-mini"));
        assert!(model_supports_reasoning(t, "gpt-5"));
        assert!(model_supports_reasoning(t, "gpt-5-codex"));
        assert!(model_supports_reasoning(t, "gpt-5-codex-high"));
    }

    #[test]
    fn openai_unsupported() {
        let t = AgentProviderApiType::OpenAi;
        assert!(!model_supports_reasoning(t, "gpt-4o"));
        assert!(!model_supports_reasoning(t, "gpt-4-turbo"));
        assert!(!model_supports_reasoning(t, "gpt-3.5-turbo"));
    }

    #[test]
    fn gemini_supported() {
        let t = AgentProviderApiType::Gemini;
        assert!(model_supports_reasoning(t, "gemini-2.5-pro"));
        assert!(model_supports_reasoning(t, "gemini-2.5-flash"));
        assert!(model_supports_reasoning(t, "gemini-3-pro"));
        assert!(model_supports_reasoning(t, "gemini-2.0-flash-thinking-exp"));
    }

    #[test]
    fn gemini_unsupported() {
        let t = AgentProviderApiType::Gemini;
        assert!(!model_supports_reasoning(t, "gemini-1.5-pro"));
        assert!(!model_supports_reasoning(t, "gemini-1.5-flash"));
        assert!(!model_supports_reasoning(t, "gemini-2.0-flash"));
    }

    #[test]
    fn deepseek_thinking_models_supported() {
        let t = AgentProviderApiType::DeepSeek;
        assert!(model_supports_reasoning(t, "deepseek-reasoner"));
        assert!(model_supports_reasoning(t, "deepseek-v4"));
        assert!(model_supports_reasoning(t, "deepseek-v4-flash"));
        assert!(model_supports_reasoning(t, "deepseek-thinking"));
        assert!(model_supports_reasoning(t, "deepseek-r1"));
        // Ordinary chat models do not have thinking
        assert!(!model_supports_reasoning(t, "deepseek-chat"));
        assert!(!model_supports_reasoning(t, "deepseek-coder"));
    }

    #[test]
    fn ollama_always_false() {
        assert!(!model_supports_reasoning(
            AgentProviderApiType::Ollama,
            "qwq-32b"
        ));
    }

    #[test]
    fn requires_reasoning_echo_deepseek() {
        // The DeepSeek api_type always echoes, regardless of model
        assert!(model_requires_reasoning_echo(
            AgentProviderApiType::DeepSeek,
            "deepseek-v4-flash"
        ));
        assert!(model_requires_reasoning_echo(
            AgentProviderApiType::DeepSeek,
            "deepseek-chat"
        ));
        assert!(model_requires_reasoning_echo(
            AgentProviderApiType::DeepSeek,
            "deepseek-reasoner"
        ));
    }

    #[test]
    fn requires_reasoning_echo_kimi_via_openai() {
        let t = AgentProviderApiType::OpenAi;
        assert!(model_requires_reasoning_echo(t, "kimi-k2-thinking"));
        assert!(model_requires_reasoning_echo(t, "moonshot-v1-32k"));
        assert!(model_requires_reasoning_echo(
            AgentProviderApiType::OpenAiResp,
            "Kimi-Latest"
        ));
        // Ordinary OpenAI models do not echo
        assert!(!model_requires_reasoning_echo(t, "gpt-5"));
        assert!(!model_requires_reasoning_echo(t, "o3-mini"));
    }

    #[test]
    fn requires_reasoning_echo_deepseek_via_openai() {
        // The official DeepSeek endpoint is OpenAI-compatible, and users often configure it as a
        // BYOP provider with OpenAI api_type. Thinking models must echo back `reasoning_content`, otherwise 400.
        let t = AgentProviderApiType::OpenAi;
        assert!(model_requires_reasoning_echo(t, "deepseek-v4-flash"));
        assert!(model_requires_reasoning_echo(t, "deepseek-v4"));
        assert!(model_requires_reasoning_echo(t, "deepseek-reasoner"));
        assert!(model_requires_reasoning_echo(t, "deepseek-r1"));
        assert!(model_requires_reasoning_echo(t, "deepseek-thinking"));
        // Case-insensitive
        assert!(model_requires_reasoning_echo(t, "DeepSeek-V4-Flash"));
        // OpenAiResp comes from the same source
        assert!(model_requires_reasoning_echo(
            AgentProviderApiType::OpenAiResp,
            "deepseek-r1"
        ));
        // Non-thinking DeepSeek models (deepseek-chat / deepseek-coder) do not enter
        // thinking-mode validation when going through the OpenAI-compatible path, so no echo is needed
        assert!(!model_requires_reasoning_echo(t, "deepseek-chat"));
        assert!(!model_requires_reasoning_echo(t, "deepseek-coder"));
    }

    #[test]
    fn opus_4_7_variants_have_xhigh_and_max() {
        let v =
            model_reasoning_variants(AgentProviderApiType::Anthropic, "claude-opus-4-7-20260101");
        assert!(v.contains(&ReasoningEffortSetting::XHigh));
        assert!(v.contains(&ReasoningEffortSetting::Max));
        assert_eq!(v.first().copied(), Some(ReasoningEffortSetting::High));
        assert_eq!(v.last().copied(), Some(ReasoningEffortSetting::Off));
    }

    #[test]
    fn opus_5_0_variants_treated_as_4_7_plus() {
        let v = model_reasoning_variants(AgentProviderApiType::Anthropic, "claude-opus-5-0");
        assert!(v.contains(&ReasoningEffortSetting::XHigh));
        assert!(v.contains(&ReasoningEffortSetting::Max));
    }

    #[test]
    fn sonnet_4_6_variants_have_max_no_xhigh() {
        let v = model_reasoning_variants(AgentProviderApiType::Anthropic, "claude-sonnet-4-6");
        assert!(v.contains(&ReasoningEffortSetting::Max));
        assert!(!v.contains(&ReasoningEffortSetting::XHigh));
    }

    #[test]
    fn sonnet_4_5_variants_legacy_no_max_no_xhigh() {
        let v = model_reasoning_variants(AgentProviderApiType::Anthropic, "claude-sonnet-4-5");
        assert!(!v.contains(&ReasoningEffortSetting::Max));
        assert!(!v.contains(&ReasoningEffortSetting::XHigh));
        assert!(v.contains(&ReasoningEffortSetting::High));
    }

    #[test]
    fn claude_3_5_haiku_variants_empty() {
        let v =
            model_reasoning_variants(AgentProviderApiType::Anthropic, "claude-3-5-haiku-20241022");
        assert!(v.is_empty());
    }

    #[test]
    fn gpt_5_variants_have_minimal_and_xhigh() {
        let v = model_reasoning_variants(AgentProviderApiType::OpenAi, "gpt-5");
        assert!(v.contains(&ReasoningEffortSetting::Minimal));
        assert!(v.contains(&ReasoningEffortSetting::XHigh));
        assert_eq!(v.first().copied(), Some(ReasoningEffortSetting::Medium));
    }

    #[test]
    fn o3_variants_no_minimal_no_xhigh() {
        let v = model_reasoning_variants(AgentProviderApiType::OpenAi, "o3-mini");
        assert!(!v.contains(&ReasoningEffortSetting::Minimal));
        assert!(!v.contains(&ReasoningEffortSetting::XHigh));
        assert!(v.contains(&ReasoningEffortSetting::High));
    }

    #[test]
    fn gpt_4o_variants_empty() {
        let v = model_reasoning_variants(AgentProviderApiType::OpenAi, "gpt-4o");
        assert!(v.is_empty());
    }

    #[test]
    fn gemini_2_5_variants_three_levels() {
        let v = model_reasoning_variants(AgentProviderApiType::Gemini, "gemini-2.5-pro");
        assert_eq!(v.len(), 4); // Medium, Low, High, Off
        assert!(v.contains(&ReasoningEffortSetting::Off));
    }

    #[test]
    fn gemini_1_5_variants_empty() {
        let v = model_reasoning_variants(AgentProviderApiType::Gemini, "gemini-1.5-pro");
        assert!(v.is_empty());
    }

    #[test]
    fn deepseek_thinking_variants_two_levels_plus_off() {
        let v = model_reasoning_variants(AgentProviderApiType::DeepSeek, "deepseek-reasoner");
        // DeepSeek official: only two levels, high / max, plus Off
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], ReasoningEffortSetting::High);
        assert_eq!(v[1], ReasoningEffortSetting::Max);
        assert_eq!(v[2], ReasoningEffortSetting::Off);
        // Redundant aliases should not be exposed
        assert!(!v.contains(&ReasoningEffortSetting::Medium));
        assert!(!v.contains(&ReasoningEffortSetting::Low));
        assert!(!v.contains(&ReasoningEffortSetting::XHigh));
    }

    #[test]
    fn deepseek_chat_variants_empty() {
        assert!(
            model_reasoning_variants(AgentProviderApiType::DeepSeek, "deepseek-chat").is_empty()
        );
    }

    #[test]
    fn ollama_variants_empty() {
        assert!(model_reasoning_variants(AgentProviderApiType::Ollama, "qwq-32b").is_empty());
    }

    #[test]
    fn default_reasoning_for_consistency() {
        // The default should equal the first item of the variants list
        assert_eq!(
            default_reasoning_for(AgentProviderApiType::Anthropic, "claude-opus-4-7"),
            Some(ReasoningEffortSetting::High)
        );
        assert_eq!(
            default_reasoning_for(AgentProviderApiType::OpenAi, "gpt-5"),
            Some(ReasoningEffortSetting::Medium)
        );
        assert_eq!(
            default_reasoning_for(AgentProviderApiType::OpenAi, "gpt-4o"),
            None
        );
    }

    #[test]
    fn supports_reasoning_consistent_with_variants() {
        // Single source of truth: supports == !variants.is_empty()
        for (t, m) in [
            (AgentProviderApiType::Anthropic, "claude-opus-4-7"),
            (AgentProviderApiType::Anthropic, "claude-3-5-haiku"),
            (AgentProviderApiType::OpenAi, "gpt-5"),
            (AgentProviderApiType::OpenAi, "gpt-4o"),
            (AgentProviderApiType::Gemini, "gemini-2.5-pro"),
            (AgentProviderApiType::Gemini, "gemini-1.5-pro"),
            (AgentProviderApiType::DeepSeek, "deepseek-reasoner"),
        ] {
            assert_eq!(
                model_supports_reasoning(t, m),
                !model_reasoning_variants(t, m).is_empty(),
                "{t:?}/{m}"
            );
        }
    }

    #[test]
    fn requires_reasoning_echo_domestic_thinking_models() {
        // Domestic OpenAI-compatible thinking models must echo `reasoning_content`,
        // otherwise the server returns 400 `The reasoning_content in the thinking mode must be passed back`.
        // The test matches under the OpenAi api_type (the most common BYOP configuration by users).
        let t = AgentProviderApiType::OpenAi;
        // Xiaomi MiMo (the model that triggered this issue)
        assert!(model_requires_reasoning_echo(t, "mimo-v2.5-pro"));
        assert!(model_requires_reasoning_echo(t, "mimo-vl-7b"));
        // Alibaba Qwen3 thinking / QwQ
        assert!(model_requires_reasoning_echo(
            t,
            "qwen3-235b-a22b-thinking-2507"
        ));
        assert!(model_requires_reasoning_echo(t, "qwq-32b-preview"));
        // Zhipu GLM thinking
        assert!(model_requires_reasoning_echo(t, "zai-glm-4.7"));
        assert!(model_requires_reasoning_echo(t, "glm-4.6-thinking"));
        assert!(model_requires_reasoning_echo(t, "glm-4.5-thinking"));
        // MiniMax / Hunyuan / Ernie / Step / Doubao / Yi
        assert!(model_requires_reasoning_echo(t, "minimax-m1-80k"));
        assert!(model_requires_reasoning_echo(t, "hunyuan-t1-latest"));
        assert!(model_requires_reasoning_echo(t, "ernie-x1-turbo-32k"));
        assert!(model_requires_reasoning_echo(t, "step-r-mini"));
        assert!(model_requires_reasoning_echo(t, "doubao-1-5-thinking-pro"));
        assert!(model_requires_reasoning_echo(t, "yi-thinking-v1"));
        // OpenAiResp comes from the same source
        let r = AgentProviderApiType::OpenAiResp;
        assert!(model_requires_reasoning_echo(r, "MiMo-V2.5-Pro"));
        assert!(model_requires_reasoning_echo(r, "Qwen3-Coder-Thinking"));
    }

    #[test]
    fn reasoning_interleaved_field_for_domestic_models() {
        // model_reasoning_interleaved must return ReasoningContent (currently all INTERLEAVED_RULES
        // are ReasoningContent; ReasoningDetails is a reserved enum placeholder).
        let t = AgentProviderApiType::OpenAi;
        assert_eq!(
            model_reasoning_interleaved(t, "mimo-v2.5-pro"),
            Some(ReasoningInterleavedField::ReasoningContent)
        );
        assert_eq!(
            model_reasoning_interleaved(t, "deepseek-v4-flash"),
            Some(ReasoningInterleavedField::ReasoningContent)
        );
        // All models under the DeepSeek api_type (including the non-thinking chat / coder) return
        // ReasoningContent — the adapter is DeepSeek-specific, aligned with opencode's default
        // `apiID.includes("deepseek") → { field: "reasoning_content" }`.
        let d = AgentProviderApiType::DeepSeek;
        assert_eq!(
            model_reasoning_interleaved(d, "deepseek-chat"),
            Some(ReasoningInterleavedField::ReasoningContent)
        );
        // Undeclared models / non-OpenAI families → None
        assert_eq!(model_reasoning_interleaved(t, "gpt-5"), None);
        assert_eq!(model_reasoning_interleaved(t, "gpt-4o"), None);
        assert_eq!(
            model_reasoning_interleaved(AgentProviderApiType::Anthropic, "claude-opus-4-7"),
            None
        );
        assert_eq!(
            model_reasoning_interleaved(AgentProviderApiType::Gemini, "gemini-2.5-pro"),
            None
        );
        assert_eq!(
            model_reasoning_interleaved(AgentProviderApiType::Ollama, "qwq-32b"),
            None
        );
    }

    #[test]
    fn requires_reasoning_echo_strict_providers_excluded() {
        // Official OpenAI / Anthropic / Gemini / ordinary OpenAI models → do not attach reasoning_content,
        // avoiding a 400 `wrong_api_format` from strict OpenAI providers such as Cerebras / Groq / OpenRouter
        // (zerx-lab/warp #25).
        let t = AgentProviderApiType::OpenAi;
        assert!(!model_requires_reasoning_echo(t, "gpt-5"));
        assert!(!model_requires_reasoning_echo(t, "gpt-4o"));
        assert!(!model_requires_reasoning_echo(t, "o3-mini"));
        // An arbitrary BYOP model whose name contains no known thinking substring and is not the DeepSeek api_type
        assert!(!model_requires_reasoning_echo(t, "llama-3.3-70b-instruct"));
        assert!(!model_requires_reasoning_echo(t, "mistral-large-2407"));
    }

    #[test]
    fn runtime_latch_overrides_static_table() {
        // For any domestic/third-party thinking model not in INTERLEAVED_RULES,
        // once a stream has emitted a reasoning chunk → it auto-echoes from the next turn on.
        // Use a deliberately "nonexistent" model id to verify the latch really works.
        let t = AgentProviderApiType::OpenAi;
        let exotic = "totally-new-thinking-model-2099";
        reset_reasoning_latch();
        assert!(
            !model_requires_reasoning_echo(t, exotic),
            "Before latching, models outside the allowlist should not echo"
        );
        note_reasoning_seen(t, exotic);
        assert!(
            model_requires_reasoning_echo(t, exotic),
            "After latching, must echo"
        );
        assert_eq!(
            model_reasoning_interleaved(t, exotic),
            Some(ReasoningInterleavedField::ReasoningContent)
        );
        // Case-insensitive
        assert!(model_requires_reasoning_echo(
            t,
            "Totally-New-Thinking-Model-2099"
        ));
        // OpenAiResp and OpenAi are independent keys — but both endpoint categories should latch their own
        let r = AgentProviderApiType::OpenAiResp;
        assert!(
            !model_requires_reasoning_echo(r, exotic),
            "Another api_type should not cross-contaminate"
        );
        note_reasoning_seen(r, exotic);
        assert!(model_requires_reasoning_echo(r, exotic));
        reset_reasoning_latch();
    }

    #[test]
    fn runtime_latch_never_writes_for_strict_api_types() {
        // Anthropic / Gemini / Ollama each go through their native reasoning channel; even if someone
        // mistakenly calls note_reasoning_seen, it must not pollute the latch (otherwise a model_id shared
        // across api_types could spuriously match on the OpenAi path — we already isolate via a composite
        // (api_type, id) key, but as extra semantic insurance: these api_types do not enter the latch).
        reset_reasoning_latch();
        for at in [
            AgentProviderApiType::Anthropic,
            AgentProviderApiType::Gemini,
            AgentProviderApiType::Ollama,
            AgentProviderApiType::DeepSeek,
        ] {
            note_reasoning_seen(at, "some-model");
        }
        // No OpenAi/OpenAiResp query should be matched by this noise
        assert!(!model_requires_reasoning_echo(
            AgentProviderApiType::OpenAi,
            "some-model"
        ));
        assert!(!model_requires_reasoning_echo(
            AgentProviderApiType::OpenAiResp,
            "some-model"
        ));
        reset_reasoning_latch();
    }

    #[test]
    fn requires_reasoning_echo_others_false() {
        assert!(!model_requires_reasoning_echo(
            AgentProviderApiType::Anthropic,
            "claude-opus-4-7"
        ));
        assert!(!model_requires_reasoning_echo(
            AgentProviderApiType::Gemini,
            "gemini-2.5-pro"
        ));
        assert!(!model_requires_reasoning_echo(
            AgentProviderApiType::Ollama,
            "qwq-32b"
        ));
    }

    #[test]
    fn think_tag_in_content_models() {
        // MiniMax M3 matches
        assert!(model_uses_think_tags_in_content("minimax-m3"));
        assert!(model_uses_think_tags_in_content("MiniMax-M3-80k"));
        assert!(model_uses_think_tags_in_content("MINIMAX-M3"));
        // MiniMax M1 does not match (uses the reasoning_content field)
        assert!(!model_uses_think_tags_in_content("minimax-m1"));
        // Other thinking models do not match (each goes through the reasoning_content field)
        assert!(!model_uses_think_tags_in_content("deepseek-r1"));
        assert!(!model_uses_think_tags_in_content("gpt-5"));
        assert!(!model_uses_think_tags_in_content("qwen3-235b"));
        assert!(!model_uses_think_tags_in_content("kimi-k2-thinking"));
        // Ordinary non-thinking models do not match
        assert!(!model_uses_think_tags_in_content("gpt-4o"));
        assert!(!model_uses_think_tags_in_content("claude-opus-4-7"));
    }
}
