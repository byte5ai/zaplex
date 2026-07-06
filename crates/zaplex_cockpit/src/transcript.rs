//! Full transcript parser for the conversation viewer (audit (g), no regression
//! vs claudeplex/-desktop's transcript view + watch/adopt mode).
//!
//! [`sessions`] reads only the transcript *tail* to derive state/model/context.
//! This module reads a whole Claude Code session `.jsonl` into an ordered list
//! of [`TranscriptTurn`]s — the authoritative content channel (assistant text,
//! thinking, tool calls, per-turn model + token usage) written by Claude to disk,
//! not scraped from the screen. Mirrors claudeplex's `transcript.ts`.
//!
//! Each line is one JSON object. We keep `type:"assistant"` and `type:"user"`
//! turns; the many bookkeeping kinds (agent-name, mode, attachment,
//! file-history-snapshot, …) are ignored. Meta/system-reminder/command wrappers
//! are dropped so the viewer shows the real conversation, not plumbing.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Who produced a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRole {
    User,
    Assistant,
}

/// Per-turn token usage (assistant turns only).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
}

/// A tool invocation on an assistant turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
}

/// One conversation turn, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptTurn {
    pub role: TurnRole,
    /// Concatenated visible text parts.
    pub text: String,
    /// Concatenated thinking parts (assistant only), if any.
    pub thinking: String,
    /// Tool calls on this turn.
    pub tools: Vec<ToolCall>,
    /// Model id reported on the turn (assistant only).
    pub model: Option<String>,
    pub usage: Option<TurnUsage>,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Strip a trailing/leading PAI session-naming hook payload
/// (`{"tab_title":…,"session_name":…}`) from assistant text — machine
/// bookkeeping, not content. Conservative: only excises a JSON object that
/// actually carries `tab_title`/`session_name`. Mirrors claudeplex `stripHookJson`.
pub fn strip_hook_json(text: &str) -> String {
    let key = text
        .rfind("\"tab_title\"")
        .max(text.rfind("\"session_name\""));
    let Some(key_idx) = key else {
        return text.to_string();
    };
    let Some(open) = text[..key_idx].rfind('{') else {
        return text.to_string();
    };
    // Brace-count from the opening brace to its real close.
    let mut depth = 0usize;
    let mut close = None;
    for (i, ch) in text[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return text.to_string();
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..open]);
    out.push_str(&text[close + 1..]);
    out.trim().to_string()
}

/// True for user content that is plumbing rather than a real user message
/// (system reminders, local-command echoes) — dropped from the viewer.
fn is_plumbing_text(text: &str) -> bool {
    text.contains("<system-reminder>")
        || text.contains("<local-command")
        || text.contains("<command-")
}

/// Pull the concatenated text, thinking, and tool calls out of an assistant
/// `message.content` array.
fn parse_assistant_content(content: &Value) -> (String, String, Vec<ToolCall>) {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tools = Vec::new();
    let Some(parts) = content.as_array() else {
        // Some transcripts store a bare string.
        if let Some(s) = content.as_str() {
            text.push_str(s);
        }
        return (text, thinking, tools);
    };
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(s) = part.get("text").and_then(Value::as_str) {
                    text.push_str(s);
                }
            }
            Some("thinking") => {
                if let Some(s) = part.get("thinking").and_then(Value::as_str) {
                    thinking.push_str(s);
                }
            }
            Some("tool_use") => {
                if let Some(name) = part.get("name").and_then(Value::as_str) {
                    tools.push(ToolCall {
                        name: name.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    (text, thinking, tools)
}

fn parse_usage(message: &Value) -> Option<TurnUsage> {
    let usage = message.get("usage")?;
    let t = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(TurnUsage {
        input: t("input_tokens"),
        output: t("output_tokens"),
        cache_read: t("cache_read_input_tokens"),
        cache_create: t("cache_creation_input_tokens"),
    })
}

fn parse_ts(v: &Value) -> Option<DateTime<Utc>> {
    let ts = v.get("timestamp").and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Parse a whole session `.jsonl` into ordered conversation turns. Malformed
/// lines are skipped (the file is appended live, so a trailing partial line is
/// normal). Empty/plumbing turns are dropped so the viewer shows real content.
pub fn parse_transcript(jsonl: &str) -> Vec<TranscriptTurn> {
    let mut turns = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let Some(message) = v.get("message") else {
                    continue;
                };
                let content = message.get("content").unwrap_or(&Value::Null);
                let (text, thinking, tools) = parse_assistant_content(content);
                let text = strip_hook_json(&text);
                // Drop turns with nothing to show (pure bookkeeping).
                if text.is_empty() && thinking.is_empty() && tools.is_empty() {
                    continue;
                }
                turns.push(TranscriptTurn {
                    role: TurnRole::Assistant,
                    text,
                    thinking,
                    tools,
                    model: message
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    usage: parse_usage(message),
                    timestamp: parse_ts(&v),
                });
            }
            Some("user") => {
                if v.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
                    continue;
                }
                let content = v.get("message").and_then(|m| m.get("content"));
                // A tool_result (array) is Claude's own continuation, not a user
                // message — skip it in the viewer.
                let Some(Value::String(text)) = content else {
                    continue;
                };
                if text.trim().is_empty() || is_plumbing_text(text) {
                    continue;
                }
                turns.push(TranscriptTurn {
                    role: TurnRole::User,
                    text: text.clone(),
                    thinking: String::new(),
                    tools: Vec::new(),
                    model: None,
                    usage: None,
                    timestamp: parse_ts(&v),
                });
            }
            _ => {}
        }
    }
    turns
}

/// Short model family for a turn header (`claude-opus-4-8` → `opus`), falling
/// back to the raw id. Empty string for an unknown/missing model.
fn model_family(model: &str) -> &str {
    for fam in ["opus", "sonnet", "haiku", "fable"] {
        if model.to_ascii_lowercase().contains(fam) {
            return fam;
        }
    }
    model
}

/// Render parsed turns as a readable Markdown document for the transcript
/// viewer (opened in the existing code/text pane — no bespoke pane type).
/// User turns become `## You`, assistant turns `## Claude · <model>`; thinking
/// is a collapsed detail, tool calls a compact list. This is the human-facing
/// projection of [`parse_transcript`]; the structured turns stay available for
/// other consumers (usage, watch mode).
pub fn format_transcript_markdown(turns: &[TranscriptTurn]) -> String {
    let mut out = String::new();
    for turn in turns {
        match turn.role {
            TurnRole::User => out.push_str("## You\n\n"),
            TurnRole::Assistant => {
                let fam = turn.model.as_deref().map(model_family).unwrap_or("");
                if fam.is_empty() {
                    out.push_str("## Claude\n\n");
                } else {
                    out.push_str(&format!("## Claude · {fam}\n\n"));
                }
            }
        }
        if !turn.thinking.is_empty() {
            // Collapsed so the answer stays front-and-center, available on demand.
            out.push_str("<details><summary>thinking</summary>\n\n");
            out.push_str(turn.thinking.trim());
            out.push_str("\n\n</details>\n\n");
        }
        if !turn.tools.is_empty() {
            let names: Vec<&str> = turn.tools.iter().map(|t| t.name.as_str()).collect();
            out.push_str(&format!("`⚙ {}`\n\n", names.join(", ")));
        }
        if !turn.text.is_empty() {
            out.push_str(turn.text.trim());
            out.push_str("\n\n");
        }
        out.push_str("---\n\n");
    }
    out.truncate(out.trim_end().len());
    out
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
