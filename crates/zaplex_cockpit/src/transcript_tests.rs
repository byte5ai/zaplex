//! Tests for the full transcript parser (conversation viewer spine).

use super::*;
use crate::types::{Provider, TaskItem, TaskState, TaskStatus};
use std::io::Write;

const CLAUDE_TASK_FIXTURE: &str = include_str!("fixture_claude_task_state.jsonl");
const CODEX_TASK_FIXTURE: &str = include_str!("fixture_codex_task_state.jsonl");

#[test]
fn parses_user_and_assistant_turns_in_order() {
    let jsonl = r#"
{"type":"user","message":{"content":"hello there"},"timestamp":"2026-07-06T10:00:00Z"}
{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"text","text":"hi!"}],"usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":5,"cache_creation_input_tokens":0}},"timestamp":"2026-07-06T10:00:05Z"}
"#;
    let turns = parse_transcript(jsonl);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, TurnRole::User);
    assert_eq!(turns[0].text, "hello there");
    assert_eq!(turns[1].role, TurnRole::Assistant);
    assert_eq!(turns[1].text, "hi!");
    assert_eq!(turns[1].model.as_deref(), Some("claude-opus-4-8"));
    let u = turns[1].usage.as_ref().unwrap();
    assert_eq!((u.input, u.output, u.cache_read), (10, 3, 5));
}

#[test]
fn extracts_thinking_and_tool_calls() {
    let jsonl = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"thinking","thinking":"let me check"},{"type":"tool_use","name":"Bash","input":{}},{"type":"text","text":"done"}]}}"#;
    let turns = parse_transcript(jsonl);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].thinking, "let me check");
    assert_eq!(
        turns[0].tools,
        vec![ToolCall {
            name: "Bash".into()
        }]
    );
    assert_eq!(turns[0].text, "done");
}

#[test]
fn drops_meta_plumbing_and_tool_results() {
    let jsonl = r#"
{"type":"user","isMeta":true,"message":{"content":"agent bookkeeping"}}
{"type":"user","message":{"content":"<system-reminder>ignore me</system-reminder>"}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"stdout"}]}}
{"type":"file-history-snapshot","foo":1}
{"type":"user","message":{"content":"real question"}}
"#;
    let turns = parse_transcript(jsonl);
    assert_eq!(turns.len(), 1, "only the real user message survives");
    assert_eq!(turns[0].text, "real question");
}

#[test]
fn skips_malformed_lines_and_blank_lines() {
    let jsonl = "not json\n\n{\"type\":\"user\",\"message\":{\"content\":\"ok\"}}\n{partial";
    let turns = parse_transcript(jsonl);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].text, "ok");
}

#[test]
fn empty_assistant_turn_is_dropped() {
    // Pure tool_result echo / no visible content → nothing to show.
    let jsonl = r#"{"type":"assistant","message":{"model":"m","content":[]}}"#;
    assert!(parse_transcript(jsonl).is_empty());
}

#[test]
fn strip_hook_json_removes_trailing_naming_payload() {
    let text = "Here is the answer.\n{\"tab_title\":\"X\",\"session_name\":\"Y\",\"mode\":\"z\"}";
    assert_eq!(strip_hook_json(text), "Here is the answer.");
    // Leading payload too.
    let lead = "{\"session_name\":\"Y\"} real text";
    assert_eq!(strip_hook_json(lead), "real text");
    // No payload → unchanged.
    assert_eq!(strip_hook_json("plain answer"), "plain answer");
}

#[test]
fn hook_json_stripped_from_parsed_assistant_text() {
    let jsonl = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"The result.\n{\"tab_title\":\"t\",\"session_name\":\"s\"}"}]}}"#;
    let turns = parse_transcript(jsonl);
    assert_eq!(turns[0].text, "The result.");
}

#[test]
fn bare_string_assistant_content_is_read_as_text() {
    let jsonl = r#"{"type":"assistant","message":{"model":"m","content":"plain string answer"}}"#;
    let turns = parse_transcript(jsonl);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].text, "plain string answer");
}

// ── format_transcript_markdown ─────────────────────────────────────────────

#[test]
fn markdown_renders_roles_model_thinking_tools() {
    let turns = vec![
        TranscriptTurn {
            role: TurnRole::User,
            text: "do the thing".into(),
            thinking: String::new(),
            tools: vec![],
            model: None,
            usage: None,
            timestamp: None,
        },
        TranscriptTurn {
            role: TurnRole::Assistant,
            text: "done".into(),
            thinking: "plan it".into(),
            tools: vec![
                ToolCall {
                    name: "Bash".into(),
                },
                ToolCall {
                    name: "Edit".into(),
                },
            ],
            model: Some("claude-opus-4-8".into()),
            usage: None,
            timestamp: None,
        },
    ];
    let md = format_transcript_markdown(&turns);
    assert!(md.contains("## You"));
    assert!(
        md.contains("## Claude · opus"),
        "model family in header: {md}"
    );
    assert!(md.contains("<details><summary>thinking</summary>"));
    assert!(md.contains("plan it"));
    assert!(md.contains("`⚙ Bash, Edit`"));
    assert!(md.contains("done"));
}

#[test]
fn markdown_assistant_without_model_and_empty_is_clean() {
    let turns = vec![TranscriptTurn {
        role: TurnRole::Assistant,
        text: "hi".into(),
        thinking: String::new(),
        tools: vec![],
        model: None,
        usage: None,
        timestamp: None,
    }];
    let md = format_transcript_markdown(&turns);
    assert!(md.contains("## Claude\n"));
    assert!(!md.contains("· "), "no family separator when model unknown");
    // No trailing separator whitespace.
    assert!(!md.ends_with("\n\n"));
}

#[test]
fn empty_transcript_formats_to_empty_string() {
    assert_eq!(format_transcript_markdown(&[]), "");
}

// ── structured external task state ──────────────────────────────────────────

#[test]
fn claude_task_fixture_orders_ids_updates_duplicates_and_filters_internal_rows() {
    let state = parse_task_state(Provider::Claude, CLAUDE_TASK_FIXTURE).unwrap();
    assert_eq!(
        state,
        TaskState {
            tasks: vec![
                TaskItem {
                    id: "2".into(),
                    title: "Wire transcript state".into(),
                    status: TaskStatus::Completed,
                },
                TaskItem {
                    id: "10".into(),
                    title: "Run final checks".into(),
                    status: TaskStatus::InProgress,
                },
            ],
        },
        "Task ids sort numerically, repeated results/updates never append rows, \
         updates win, and metadata._internal rows stay hidden"
    );
}

#[test]
fn claude_todowrite_preserves_emitted_order_and_normalizes_statuses() {
    let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"  Read\nschema ","status":"completed"},{"content":"Wire rows","status":"in_progress"},{"content":"Verify","status":"unknown"}]}}]}}"#;
    let state = parse_task_state(Provider::Claude, jsonl).unwrap();
    assert_eq!(
        state.tasks,
        vec![
            TaskItem {
                id: "0".into(),
                title: "Read schema".into(),
                status: TaskStatus::Completed,
            },
            TaskItem {
                id: "1".into(),
                title: "Wire rows".into(),
                status: TaskStatus::InProgress,
            },
            TaskItem {
                id: "2".into(),
                title: "Verify".into(),
                status: TaskStatus::Pending,
            },
        ]
    );
}

#[test]
fn claude_metadata_update_can_make_an_internal_task_visible() {
    let jsonl = r#"
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_hidden","name":"TaskCreate","input":{"subject":"Hidden","description":"Hidden task","metadata":{"_internal":true}}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_hidden","content":"Task #7 created successfully: Hidden"}]},"toolUseResult":{"task":{"id":"7","subject":"Hidden"}}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"taskId":"7","subject":"Visible now","status":"in_progress","metadata":{"_internal":null}}}]}}
"#;
    assert_eq!(
        parse_task_state(Provider::Claude, jsonl).unwrap().tasks,
        vec![TaskItem {
            id: "7".into(),
            title: "Visible now".into(),
            status: TaskStatus::InProgress,
        }]
    );
}

#[test]
fn claude_deleted_task_leaves_an_explicit_empty_state() {
    let jsonl = r#"
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_delete","name":"TaskCreate","input":{"subject":"Temporary","description":"Delete after completion"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_delete","content":"Task #4 created successfully: Temporary"}]},"toolUseResult":{"task":{"id":"4","subject":"Temporary"}}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"taskId":"4","status":"deleted"}}]}}
"#;
    assert_eq!(
        parse_task_state(Provider::Claude, jsonl),
        Some(TaskState { tasks: Vec::new() })
    );
}

#[test]
fn claude_task_update_accepts_raw_id_aliases_repaired_by_the_cli() {
    let jsonl = r#"
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_alias","name":"TaskCreate","input":{"subject":"Original","description":"Alias coverage"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_alias","content":"Task #7 created successfully: Original"}]},"toolUseResult":{"task":{"id":"7","subject":"Original"}}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"id":"7","subject":"Renamed"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"task_id":"7","status":"completed"}}]}}
"#;
    assert_eq!(
        parse_task_state(Provider::Claude, jsonl).unwrap().tasks,
        vec![TaskItem {
            id: "7".into(),
            title: "Renamed".into(),
            status: TaskStatus::Completed,
        }]
    );
}

#[test]
fn codex_latest_valid_update_plan_replaces_earlier_rows_without_accumulating_duplicates() {
    let state = parse_task_state(Provider::Codex, CODEX_TASK_FIXTURE).unwrap();
    assert_eq!(
        state.tasks,
        vec![
            TaskItem {
                id: "0".into(),
                title: "Inspect transcript schema".into(),
                status: TaskStatus::Completed,
            },
            TaskItem {
                id: "1".into(),
                title: "Carry typed task state".into(),
                status: TaskStatus::InProgress,
            },
            TaskItem {
                id: "2".into(),
                title: "Run static checks".into(),
                status: TaskStatus::Pending,
            },
        ],
        "the second full update replaces the first; malformed later records do not clear it"
    );
}

#[test]
fn task_state_cache_reuses_unchanged_files_and_invalidates_on_append_or_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("tasks.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[{\"step\":\"First\",\"status\":\"pending\"}]}"}}"#,
            "\n"
        ),
    )
    .unwrap();
    let mut cache = TaskStateCache::default();

    let first = cache.parse_file(Provider::Codex, &path).unwrap();
    assert_eq!(first.tasks[0].title, "First");
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(
        cache.parse_file(Provider::Codex, &path),
        Some(first),
        "an unchanged fingerprint must reuse the cached result"
    );

    let appended = serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "update_plan",
            "arguments": serde_json::to_string(&serde_json::json!({
                "plan": [{"step": "Second", "status": "in_progress"}]
            }))
            .unwrap()
        }
    });
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap(),
        "{appended}"
    )
    .unwrap();
    let refreshed = cache.parse_file(Provider::Codex, &path).unwrap();
    assert_eq!(refreshed.tasks[0].title, "Second");
    assert_eq!(refreshed.tasks[0].status, TaskStatus::InProgress);

    std::fs::remove_file(&path).unwrap();
    assert_eq!(cache.parse_file(Provider::Codex, &path), None);
    assert!(cache.entries.is_empty());
}

#[test]
fn claude_task_cache_reads_only_appended_bytes_after_growth() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("claude-tasks.jsonl");
    let mut initial = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"task_id":"7","subject":"Initial","status":"pending"}}]}}"#,
        "\n"
    )
    .to_string();
    initial.push_str(&"{\"type\":\"unknown\"}\n".repeat(32));
    std::fs::write(&path, initial).unwrap();
    let mut cache = TaskStateCache::default();
    let first = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(first.tasks[0].title, "Initial");
    assert!(std::fs::metadata(&path).unwrap().len() > 256);

    let appended = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"task_id":"7","status":"completed"}}]}}"#,
        "\n"
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(appended.as_bytes())
        .unwrap();
    cache.bytes_read = 0;

    let refreshed = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(refreshed.tasks[0].status, TaskStatus::Completed);
    assert_eq!(cache.bytes_read, 256 + appended.len() as u64);
}

#[test]
fn claude_task_cache_preserves_accumulator_across_linear_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("claude-linear.jsonl");
    let mut initial = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"create-7","name":"TaskCreate","input":{"subject":"Created"}}]}}"#,
        "\n"
    )
    .to_string();
    initial.push_str(&"{\"type\":\"unknown\"}\n".repeat(32));
    std::fs::write(&path, initial).unwrap();
    let mut cache = TaskStateCache::default();
    assert_eq!(cache.parse_file(Provider::Claude, &path), None);
    assert!(std::fs::metadata(&path).unwrap().len() > 256);
    cache.bytes_read = 0;

    let created = concat!(
        r#"{"type":"user","toolUseResult":{"task":{"id":"7","subject":"Created"}},"message":{"content":[{"type":"tool_result","tool_use_id":"create-7"}]}}"#,
        "\n"
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(created.as_bytes())
        .unwrap();
    let state = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(state.tasks[0].title, "Created");
    assert_eq!(state.tasks[0].status, TaskStatus::Pending);
    assert_eq!(cache.bytes_read, 256 + created.len() as u64);

    let completed = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"task_id":"7","subject":"Renamed","status":"completed"}}]}}"#,
        "\n"
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(completed.as_bytes())
        .unwrap();
    let state = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(state.tasks[0].title, "Renamed");
    assert_eq!(state.tasks[0].status, TaskStatus::Completed);
    assert_eq!(
        cache.bytes_read,
        512 + created.len() as u64 + completed.len() as u64
    );
}

#[test]
fn claude_task_cache_waits_for_a_split_jsonl_line() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("claude-split.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"task_id":"7","subject":"Initial","status":"pending"}}]}}"#,
            "\n"
        ),
    )
    .unwrap();
    let mut cache = TaskStateCache::default();
    cache.parse_file(Provider::Claude, &path).unwrap();
    let update = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskUpdate","input":{"task_id":"7","subject":"Split","status":"completed"}}]}}"#;
    let split = update.len() / 2;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(&update.as_bytes()[..split]).unwrap();
    drop(file);
    let state = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(state.tasks[0].title, "Initial");
    assert_eq!(state.tasks[0].status, TaskStatus::Pending);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(&update.as_bytes()[split..]).unwrap();
    file.write_all(b"\n").unwrap();
    drop(file);
    let state = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(state.tasks[0].title, "Split");
    assert_eq!(state.tasks[0].status, TaskStatus::Completed);
}

#[test]
fn claude_task_cache_full_replays_after_truncate_or_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("claude-replaced.jsonl");
    let task = |title: &str| {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use",
                "name": "TaskUpdate",
                "input": {"task_id": "7", "subject": title, "status": "pending"}
            }]}
        })
        .to_string()
            + "\n"
    };
    std::fs::write(
        &path,
        task("Original") + &"{\"type\":\"unknown\"}\n".repeat(32),
    )
    .unwrap();
    let mut cache = TaskStateCache::default();
    cache.parse_file(Provider::Claude, &path).unwrap();

    std::fs::write(&path, task("Truncated")).unwrap();
    let state = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(state.tasks[0].title, "Truncated");

    let replacement = path.with_extension("replacement");
    std::fs::write(&replacement, task("Replacement")).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(replacement, &path).unwrap();
    let state = cache.parse_file(Provider::Claude, &path).unwrap();
    assert_eq!(state.tasks[0].title, "Replacement");
}

#[test]
fn malformed_unknown_and_incomplete_records_never_clear_last_valid_state() {
    let jsonl = r#"
{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[{\"step\":\"Keep me\",\"status\":\"in_progress\"}]}"}}
{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[{\"status\":\"completed\"}]}"}}
{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{"}}
{"type":"response_item","payload":{"type":"function_call","name":"other","arguments":"{\"plan\":[]}"}}
not-json
"#;
    assert_eq!(
        parse_task_state(Provider::Codex, jsonl).unwrap().tasks,
        vec![TaskItem {
            id: "0".into(),
            title: "Keep me".into(),
            status: TaskStatus::InProgress,
        }]
    );
    assert_eq!(
        parse_task_state(Provider::Claude, "not-json\n{\"type\":\"unknown\"}"),
        None
    );
}

#[test]
fn explicit_empty_provider_state_is_distinct_from_no_task_state() {
    let claude_empty = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[]}}]}}"#;
    let codex_empty = r#"{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[]}"}}"#;
    assert_eq!(
        parse_task_state(Provider::Claude, claude_empty),
        Some(TaskState { tasks: Vec::new() })
    );
    assert_eq!(
        parse_task_state(Provider::Codex, codex_empty),
        Some(TaskState { tasks: Vec::new() })
    );
    assert_eq!(parse_task_state(Provider::Claude, ""), None);
    assert_eq!(parse_task_state(Provider::Codex, ""), None);
}
