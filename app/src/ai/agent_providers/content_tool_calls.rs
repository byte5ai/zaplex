//! Fallback parsing for tool calls emitted as Ollama assistant text.
//!
//! Some local models serialize a tool call into the assistant `content` field
//! instead of using the native `tool_calls` field. This parser is deliberately
//! strict because accepted output becomes executable: it only accepts a complete
//! JSON response, exact tool names exposed for the current request, and object
//! arguments.

use std::collections::HashSet;

use genai::chat::ToolCall;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::settings::AgentProviderApiType;

/// Extract fallback tool calls only for Ollama responses without native calls.
pub(super) fn extract_ollama_fallback_tool_calls(
    api_type: AgentProviderApiType,
    has_native_tool_calls: bool,
    assistant_text: &str,
    available_tool_names: &[String],
) -> Vec<ToolCall> {
    if api_type != AgentProviderApiType::Ollama || has_native_tool_calls {
        return Vec::new();
    }

    extract_tool_calls_from_assistant_text(assistant_text, available_tool_names)
}

/// Identify the raw assistant message paired with a persisted fallback ToolCall.
pub(super) fn is_persisted_ollama_fallback_text(
    api_type: AgentProviderApiType,
    same_turn_has_tool_call: bool,
    assistant_text: &str,
    available_tool_names: &[String],
) -> bool {
    api_type == AgentProviderApiType::Ollama
        && same_turn_has_tool_call
        && !extract_tool_calls_from_assistant_text(assistant_text, available_tool_names).is_empty()
}

/// Parse a complete assistant response containing one call or a `tool_calls` wrapper.
///
/// Prose containing JSON is intentionally rejected. A single `json` code fence is
/// accepted because local models commonly add that formatting even when instructed
/// to return only the object.
pub(super) fn extract_tool_calls_from_assistant_text(
    text: &str,
    available_tool_names: &[String],
) -> Vec<ToolCall> {
    let Some(value) = parse_complete_json_response(text) else {
        return Vec::new();
    };
    let Some(values) = tool_call_values(&value) else {
        return Vec::new();
    };
    let allowed: HashSet<&str> = available_tool_names.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut calls = Vec::with_capacity(values.len());

    for value in values {
        let Some(call) = parse_tool_call_value(value, &allowed) else {
            return Vec::new();
        };
        let signature = (call.fn_name.clone(), call.fn_arguments.to_string());
        if seen.insert(signature) {
            calls.push(call);
        }
    }

    calls
}

fn parse_complete_json_response(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let payload = if trimmed.starts_with("```") {
        fenced_json_payload(trimmed)?
    } else {
        trimmed
    };
    serde_json::from_str(payload).ok()
}

fn fenced_json_payload(text: &str) -> Option<&str> {
    let after_open = text.strip_prefix("```")?;
    let header_end = after_open.find('\n')?;
    let language = after_open[..header_end].trim();
    if !language.is_empty() && !language.eq_ignore_ascii_case("json") {
        return None;
    }
    let body_with_close = &after_open[header_end + 1..];
    let body = body_with_close.strip_suffix("```")?.trim();
    if body.is_empty() || body.contains("```") {
        return None;
    }
    Some(body)
}

fn tool_call_values(value: &Value) -> Option<Vec<&Value>> {
    let object = value.as_object()?;
    if let Some(tool_calls) = object.get("tool_calls") {
        if object.len() != 1 {
            return None;
        }
        return Some(tool_calls.as_array()?.iter().collect());
    }
    Some(vec![value])
}

fn parse_tool_call_value(value: &Value, allowed: &HashSet<&str>) -> Option<ToolCall> {
    let object = value.as_object()?;
    let (name, arguments) = if let Some(function) = object.get("function") {
        parse_nested_function(object, function)?
    } else {
        parse_direct_function(object)?
    };
    if !allowed.contains(name) {
        return None;
    }

    Some(ToolCall {
        call_id: Uuid::new_v4().to_string(),
        fn_name: name.to_owned(),
        fn_arguments: arguments,
        thought_signatures: None,
    })
}

fn parse_nested_function<'a>(
    object: &'a Map<String, Value>,
    function: &'a Value,
) -> Option<(&'a str, Value)> {
    if !keys_are_allowed(object, &["id", "type", "function"])
        || object.get("type").and_then(Value::as_str) != Some("function")
    {
        return None;
    }
    let function = function.as_object()?;
    if !keys_are_allowed(function, &["name", "arguments", "parameters"]) {
        return None;
    }
    let name = function.get("name")?.as_str()?;
    let arguments = function
        .get("arguments")
        .or_else(|| function.get("parameters"))?;
    Some((name, object_arguments(arguments)?))
}

fn parse_direct_function(object: &Map<String, Value>) -> Option<(&str, Value)> {
    if !keys_are_allowed(
        object,
        &["id", "type", "name", "arguments", "parameters", "input"],
    ) {
        return None;
    }
    if object
        .get("type")
        .is_some_and(|value| value.as_str() != Some("function"))
    {
        return None;
    }
    let name = object.get("name")?.as_str()?;
    let arguments = object
        .get("arguments")
        .or_else(|| object.get("parameters"))
        .or_else(|| object.get("input"))?;
    Some((name, object_arguments(arguments)?))
}

fn object_arguments(value: &Value) -> Option<Value> {
    match value {
        Value::Object(_) => Some(value.clone()),
        Value::String(raw) => {
            let parsed = serde_json::from_str::<Value>(raw).ok()?;
            parsed.is_object().then_some(parsed)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    }
}

fn keys_are_allowed(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

#[cfg(test)]
#[path = "content_tool_calls_tests.rs"]
mod tests;
