//! Tests for the full transcript parser (conversation viewer spine).

use super::*;

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
    assert_eq!(turns[0].tools, vec![ToolCall { name: "Bash".into() }]);
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
            tools: vec![ToolCall { name: "Bash".into() }, ToolCall { name: "Edit".into() }],
            model: Some("claude-opus-4-8".into()),
            usage: None,
            timestamp: None,
        },
    ];
    let md = format_transcript_markdown(&turns);
    assert!(md.contains("## You"));
    assert!(md.contains("## Claude · opus"), "model family in header: {md}");
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
