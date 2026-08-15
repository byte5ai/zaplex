//! Full transcript parser for the conversation viewer (audit (g), no regression
//! vs claudeplex/-desktop's transcript view + watch/adopt mode).
//!
//! [`sessions`] reads the transcript tail for coarse state/model/context and
//! replays the complete transcript for structured task state. This module also
//! reads a whole Claude Code session `.jsonl` into an ordered list of
//! [`TranscriptTurn`]s — the authoritative content channel (assistant text,
//! thinking, tool calls, per-turn model + token usage) written by Claude to disk,
//! not scraped from the screen. It reconstructs provider-neutral structured
//! task state from Claude task tools and Codex `update_plan` calls. Mirrors
//! claudeplex's `transcript.ts`.
//!
//! Each line is one JSON object. We keep `type:"assistant"` and `type:"user"`
//! turns; the many bookkeeping kinds (agent-name, mode, attachment,
//! file-history-snapshot, …) are ignored. Meta/system-reminder/command wrappers
//! are dropped so the viewer shows the real conversation, not plumbing.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::types::{Provider, TaskItem, TaskState, TaskStatus};

const TASK_STATE_CACHE_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    len: u64,
    modified: SystemTime,
}

pub(crate) fn file_fingerprint(path: &Path) -> std::io::Result<FileFingerprint> {
    let metadata = std::fs::metadata(path)?;
    Ok(FileFingerprint {
        len: metadata.len(),
        modified: metadata.modified()?,
    })
}

#[derive(Clone, Debug)]
struct CachedTaskState {
    provider: Provider,
    fingerprint: FileFingerprint,
    state: Option<TaskState>,
    last_used: u64,
}

/// Bounded cache for the full-transcript replay needed by structured task state.
///
/// Every lookup still stats the file, but an unchanged `(mtime, size)` pair
/// avoids reopening and reparsing the transcript. The cache is process-local,
/// contains only the same task titles already exposed on `SessionSnapshot`,
/// and evicts the least recently used entry once the fixed bound is exceeded.
#[derive(Clone, Debug, Default)]
pub struct TaskStateCache {
    entries: HashMap<PathBuf, CachedTaskState>,
    clock: u64,
}

impl TaskStateCache {
    pub fn parse_file(&mut self, provider: Provider, path: &Path) -> Option<TaskState> {
        let fingerprint = match file_fingerprint(path) {
            Ok(fingerprint) => fingerprint,
            Err(_) => {
                self.entries.remove(path);
                return None;
            }
        };
        self.clock = self.clock.wrapping_add(1);
        if let Some(cached) = self.entries.get_mut(path) {
            if cached.provider == provider && cached.fingerprint == fingerprint {
                cached.last_used = self.clock;
                return cached.state.clone();
            }
        }

        let state = match std::fs::read_to_string(path) {
            Ok(jsonl) => parse_task_state(provider, &jsonl),
            Err(_) => {
                self.entries.remove(path);
                return None;
            }
        };
        self.entries.insert(
            path.to_path_buf(),
            CachedTaskState {
                provider,
                fingerprint,
                state: state.clone(),
                last_used: self.clock,
            },
        );
        self.evict_lru();
        state
    }

    fn evict_lru(&mut self) {
        while self.entries.len() > TASK_STATE_CACHE_LIMIT {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone())
            else {
                return;
            };
            self.entries.remove(&oldest);
        }
    }
}

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

#[derive(Debug, Clone)]
struct ClaudeTask {
    item: TaskItem,
    metadata: Map<String, Value>,
}

impl ClaudeTask {
    fn is_internal(&self) -> bool {
        self.metadata
            .get("_internal")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
struct PendingClaudeTask {
    title: String,
    metadata: Map<String, Value>,
}

#[derive(Debug, Default)]
struct ClaudeTaskAccumulator {
    tasks: BTreeMap<String, ClaudeTask>,
    pending_creates: HashMap<String, PendingClaudeTask>,
    displayed: Option<Vec<TaskItem>>,
}

impl ClaudeTaskAccumulator {
    fn apply(&mut self, object: &Value) {
        match object.get("type").and_then(Value::as_str) {
            Some("assistant") => self.apply_assistant(object),
            Some("user") => self.apply_user(object),
            Some(_) | None => {}
        }
    }

    fn apply_assistant(&mut self, object: &Value) {
        let Some(content) = object
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for part in content {
            if part.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            match part.get("name").and_then(Value::as_str) {
                Some("TaskCreate") => self.apply_task_create(part),
                Some("TaskUpdate") => self.apply_task_update(part),
                Some("TodoWrite") => self.apply_todo_write(part),
                Some(_) | None => {}
            }
        }
    }

    fn apply_task_create(&mut self, tool_use: &Value) {
        let Some(input) = tool_use.get("input").and_then(Value::as_object) else {
            return;
        };
        let Some(title) = normalized_title(input.get("subject").and_then(Value::as_str)) else {
            return;
        };
        let metadata = input
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(tool_use_id) = value_id(tool_use.get("id")) else {
            return;
        };
        self.pending_creates
            .insert(tool_use_id, PendingClaudeTask { title, metadata });
    }

    fn apply_task_update(&mut self, tool_use: &Value) {
        let Some(input) = tool_use.get("input").and_then(Value::as_object) else {
            return;
        };
        let Some(task_id) = ["taskId", "id", "task_id"]
            .iter()
            .find_map(|key| value_id(input.get(*key)))
        else {
            return;
        };
        if input.get("status").and_then(Value::as_str) == Some("deleted") {
            if self.tasks.remove(&task_id).is_some() {
                self.refresh_task_items();
            }
            return;
        }
        if let Some(task) = self.tasks.get_mut(&task_id) {
            if let Some(title) = normalized_title(input.get("subject").and_then(Value::as_str)) {
                task.item.title = title;
            }
            if let Some(status) = input.get("status").and_then(Value::as_str) {
                task.item.status = external_task_status(Some(status));
            }
            if let Some(metadata) = input.get("metadata").and_then(Value::as_object) {
                for (key, value) in metadata {
                    if value.is_null() {
                        task.metadata.remove(key);
                    } else {
                        task.metadata.insert(key.clone(), value.clone());
                    }
                }
            }
            self.refresh_task_items();
            return;
        }
        let Some(title) = normalized_title(input.get("subject").and_then(Value::as_str)) else {
            return;
        };
        self.tasks.insert(
            task_id.clone(),
            ClaudeTask {
                item: TaskItem {
                    id: task_id,
                    title,
                    status: external_task_status(input.get("status").and_then(Value::as_str)),
                },
                metadata: input
                    .get("metadata")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            },
        );
        self.refresh_task_items();
    }

    fn apply_todo_write(&mut self, tool_use: &Value) {
        let Some(todos) = tool_use
            .get("input")
            .and_then(|input| input.get("todos"))
            .and_then(Value::as_array)
        else {
            return;
        };
        let rows = todos
            .iter()
            .enumerate()
            .filter_map(|(index, todo)| {
                let title = normalized_title(todo.get("content").and_then(Value::as_str))?;
                Some(TaskItem {
                    id: index.to_string(),
                    title,
                    status: external_task_status(todo.get("status").and_then(Value::as_str)),
                })
            })
            .collect();
        if self.task_items().is_empty() {
            self.displayed = Some(rows);
        }
    }

    fn apply_user(&mut self, object: &Value) {
        let Some(result_task) = object
            .get("toolUseResult")
            .and_then(|result| result.get("task"))
            .and_then(Value::as_object)
        else {
            return;
        };
        let Some(task_id) = value_id(result_task.get("id")) else {
            return;
        };
        let Some(content) = object
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for part in content {
            if part.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = value_id(part.get("tool_use_id")) else {
                continue;
            };
            let Some(pending) = self.pending_creates.remove(&tool_use_id) else {
                continue;
            };
            let title = normalized_title(result_task.get("subject").and_then(Value::as_str))
                .unwrap_or(pending.title);
            self.tasks.insert(
                task_id.clone(),
                ClaudeTask {
                    item: TaskItem {
                        id: task_id.clone(),
                        title,
                        status: TaskStatus::Pending,
                    },
                    metadata: pending.metadata,
                },
            );
            self.refresh_task_items();
            return;
        }
    }

    fn task_items(&self) -> Vec<TaskItem> {
        let mut tasks: Vec<&ClaudeTask> = self
            .tasks
            .values()
            .filter(|task| !task.is_internal())
            .collect();
        tasks.sort_by(|left, right| compare_task_ids(&left.item.id, &right.item.id));
        tasks.into_iter().map(|task| task.item.clone()).collect()
    }

    fn refresh_task_items(&mut self) {
        self.displayed = Some(self.task_items());
    }

    fn finish(self) -> Option<TaskState> {
        self.displayed.map(|tasks| TaskState { tasks })
    }
}

fn compare_task_ids(left: &str, right: &str) -> Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

fn value_id(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(id) = value.as_str() {
        return normalized_title(Some(id));
    }
    value.as_u64().map(|id| id.to_string())
}

fn normalized_title(text: Option<&str>) -> Option<String> {
    let normalized = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn external_task_status(status: Option<&str>) -> TaskStatus {
    let normalized = status.unwrap_or("").trim().to_ascii_lowercase();
    match normalized.as_str() {
        "completed" | "complete" | "done" => TaskStatus::Completed,
        "in_progress" | "in-progress" | "inprogress" | "running" | "active" => {
            TaskStatus::InProgress
        }
        _ => TaskStatus::Pending,
    }
}

fn parse_claude_task_state(jsonl: &str) -> Option<TaskState> {
    let mut accumulator = ClaudeTaskAccumulator::default();
    for line in jsonl.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(object) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        accumulator.apply(&object);
    }
    accumulator.finish()
}

fn parse_codex_task_state(jsonl: &str) -> Option<TaskState> {
    let mut latest = None;
    for line in jsonl.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(object) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = object.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("function_call")
            || payload.get("name").and_then(Value::as_str) != Some("update_plan")
        {
            continue;
        }
        let Some(arguments) = payload.get("arguments") else {
            continue;
        };
        let parsed_arguments = if let Some(arguments) = arguments.as_str() {
            let Ok(parsed) = serde_json::from_str::<Value>(arguments) else {
                continue;
            };
            parsed
        } else if arguments.is_object() {
            arguments.clone()
        } else {
            continue;
        };
        let Some(plan) = parsed_arguments.get("plan").and_then(Value::as_array) else {
            continue;
        };
        let tasks: Vec<TaskItem> = plan
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let title = normalized_title(row.get("step").and_then(Value::as_str))?;
                Some(TaskItem {
                    id: index.to_string(),
                    title,
                    status: external_task_status(row.get("status").and_then(Value::as_str)),
                })
            })
            .collect();
        // A valid empty plan explicitly clears the state. A non-empty plan made
        // solely of malformed rows cannot be trusted to erase the last good one.
        if plan.is_empty() || !tasks.is_empty() {
            latest = Some(TaskState { tasks });
        }
    }
    latest
}

/// Reconstruct the latest structured task state from one provider transcript.
///
/// Both formats are append-only. Claude's task protocol is incremental and is
/// replayed by stable task id; Codex `update_plan` calls are full replacements,
/// so the latest valid call wins. Malformed/unknown records are ignored and
/// never clear a previously valid state.
pub fn parse_task_state(provider: Provider, jsonl: &str) -> Option<TaskState> {
    match provider {
        Provider::Claude => parse_claude_task_state(jsonl),
        Provider::Codex => parse_codex_task_state(jsonl),
        Provider::Antigravity => None,
    }
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
