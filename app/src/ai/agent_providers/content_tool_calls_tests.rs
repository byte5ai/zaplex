use super::{
    extract_ollama_fallback_tool_calls, extract_tool_calls_from_assistant_text,
    is_persisted_ollama_fallback_text,
};
use crate::settings::AgentProviderApiType;

fn offered_tools() -> Vec<String> {
    vec!["run_shell_command".to_owned(), "read_files".to_owned()]
}

#[test]
fn extracts_exact_ollama_tool_json_once() {
    let text = r#"{"name":"run_shell_command","arguments":{"command":"echo hello"}}"#;

    let calls = extract_ollama_fallback_tool_calls(
        AgentProviderApiType::Ollama,
        false,
        text,
        &offered_tools(),
    );

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].fn_name, "run_shell_command");
    assert_eq!(calls[0].fn_arguments["command"], "echo hello");
}

#[test]
fn extracts_single_json_fence_and_openai_wrapper() {
    let text = r#"```json
{"tool_calls":[{"type":"function","function":{"name":"read_files","arguments":{"paths":["README.md"]}}}]}
```"#;

    let calls = extract_tool_calls_from_assistant_text(text, &offered_tools());

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].fn_name, "read_files");
    assert_eq!(calls[0].fn_arguments["paths"][0], "README.md");
}

#[test]
fn extracts_ollama_function_parameters_shape() {
    let text = r#"{"type":"function","name":"run_shell_command","parameters":{"command":"pwd"}}"#;

    let calls = extract_tool_calls_from_assistant_text(text, &offered_tools());

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].fn_name, "run_shell_command");
    assert_eq!(calls[0].fn_arguments["command"], "pwd");
}

#[test]
fn native_tool_calls_take_priority_without_duplication() {
    let text = r#"{"name":"run_shell_command","arguments":{"command":"echo hello"}}"#;

    let calls = extract_ollama_fallback_tool_calls(
        AgentProviderApiType::Ollama,
        true,
        text,
        &offered_tools(),
    );

    assert!(calls.is_empty());
}

#[test]
fn persisted_transport_json_is_filtered_only_with_same_turn_call() {
    let text = r#"{"name":"run_shell_command","arguments":{"command":"echo hello"}}"#;

    assert!(is_persisted_ollama_fallback_text(
        AgentProviderApiType::Ollama,
        true,
        text,
        &offered_tools(),
    ));
    assert!(!is_persisted_ollama_fallback_text(
        AgentProviderApiType::Ollama,
        false,
        text,
        &offered_tools(),
    ));
}

#[test]
fn fallback_is_not_enabled_for_other_providers() {
    let text = r#"{"name":"run_shell_command","arguments":{"command":"echo hello"}}"#;

    let calls = extract_ollama_fallback_tool_calls(
        AgentProviderApiType::OpenAi,
        false,
        text,
        &offered_tools(),
    );

    assert!(calls.is_empty());
}

#[test]
fn unknown_or_unoffered_tools_are_rejected() {
    for text in [
        r#"{"name":"totally_made_up_tool","arguments":{"command":"echo unsafe"}}"#,
        r#"{"name":"websearch","arguments":{"query":"example"}}"#,
    ] {
        assert!(extract_tool_calls_from_assistant_text(text, &offered_tools()).is_empty());
    }
}

#[test]
fn prose_json_examples_are_not_executed() {
    let text = r#"For example, a tool call can look like this:
```json
{"name":"run_shell_command","arguments":{"command":"echo example"}}
```"#;

    assert!(extract_tool_calls_from_assistant_text(text, &offered_tools()).is_empty());
}

#[test]
fn reasoning_markup_is_not_executed() {
    let text =
        r#"<think>{"name":"run_shell_command","arguments":{"command":"echo private"}}</think>"#;

    assert!(extract_tool_calls_from_assistant_text(text, &offered_tools()).is_empty());
}

#[test]
fn malformed_or_tool_shaped_application_json_is_rejected() {
    for text in [
        r#"{"name":"run_shell_command","arguments":{"command":"unterminated"}"#,
        r#"{"name":"run_shell_command","version":"1.0.0"}"#,
        r#"{"name":"run_shell_command","arguments":["echo wrong shape"]}"#,
    ] {
        assert!(extract_tool_calls_from_assistant_text(text, &offered_tools()).is_empty());
    }
}

#[test]
fn one_invalid_call_rejects_the_entire_wrapper() {
    let text = r#"{"tool_calls":[
        {"type":"function","function":{"name":"read_files","arguments":{"paths":["README.md"]}}},
        {"type":"function","function":{"name":"unknown","arguments":{}}}
    ]}"#;

    assert!(extract_tool_calls_from_assistant_text(text, &offered_tools()).is_empty());
}

#[test]
fn duplicate_fallback_calls_execute_only_once() {
    let text = r#"{"tool_calls":[
        {"type":"function","function":{"name":"run_shell_command","arguments":{"command":"echo once"}}},
        {"type":"function","function":{"name":"run_shell_command","arguments":{"command":"echo once"}}}
    ]}"#;

    let calls = extract_tool_calls_from_assistant_text(text, &offered_tools());

    assert_eq!(calls.len(), 1);
}
