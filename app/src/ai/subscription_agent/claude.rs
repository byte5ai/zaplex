use super::{
    AgentCapability, ApprovalDecision, InstallationIdentity, ModelCapability, ModelEffort,
    SessionIdentity, SubscriptionEvent, Usage,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};

pub(crate) struct ClaudeProtocol;

impl ClaudeProtocol {
    pub(crate) fn initialize_request(request_id: &str) -> Value {
        json!({
            "type": "control_request",
            "request_id": request_id,
            "request": {
                "subtype": "initialize"
            }
        })
    }

    pub(crate) fn user_message(prompt: &str, session_id: Option<&str>) -> Value {
        let mut frame = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": prompt
                }]
            },
            "parent_tool_use_id": null
        });
        if let Some(session_id) = session_id {
            frame["session_id"] = Value::String(session_id.to_string());
        }
        frame
    }

    pub(crate) fn interrupt_request(request_id: &str) -> Value {
        json!({
            "type": "control_request",
            "request_id": request_id,
            "request": {
                "subtype": "interrupt"
            }
        })
    }

    pub(crate) fn approval_response(
        request_id: &str,
        decision: ApprovalDecision,
        input: &Value,
    ) -> Value {
        let response = match decision {
            ApprovalDecision::Allow | ApprovalDecision::AllowForSession => json!({
                "behavior": "allow",
                "updatedInput": input
            }),
            ApprovalDecision::Deny | ApprovalDecision::Cancel => json!({
                "behavior": "deny",
                "message": if decision == ApprovalDecision::Cancel {
                    "Cancelled by user"
                } else {
                    "Denied by user"
                }
            }),
        };
        json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response
            }
        })
    }

    pub(crate) fn parse_capability(
        frame: &Value,
        mut installation: InstallationIdentity,
    ) -> Result<AgentCapability> {
        let payload = frame
            .pointer("/response/response")
            .or_else(|| frame.get("response"))
            .context("Claude initialize response is missing its payload")?;
        let models = payload
            .get("models")
            .and_then(Value::as_array)
            .context("Claude initialize response did not report models")?;
        let account = payload
            .get("account")
            .and_then(Value::as_object)
            .context("Claude Code is not signed in")?;

        if let Some(display_name) = claude_account_display_name(account) {
            installation.account.display_name = display_name.clone();
        }

        let models = models.iter().map(parse_model).collect::<Result<Vec<_>>>()?;
        if models.is_empty() {
            return Err(anyhow!("Claude Code reported no available models"));
        }
        Ok(AgentCapability {
            installation,
            models,
        })
    }

    pub(crate) fn parse_event(frame: &Value) -> Result<Vec<SubscriptionEvent>> {
        let frame_type = frame
            .get("type")
            .and_then(Value::as_str)
            .context("Claude protocol frame has no type")?;
        match frame_type {
            "system" => Ok(parse_system_event(frame)),
            "assistant" => parse_content(frame.pointer("/message/content"), false),
            "user" => parse_content(frame.pointer("/message/content"), true),
            "stream_event" => Ok(parse_stream_event(frame)),
            "control_request" => parse_control_request(frame),
            "result" => Ok(parse_result(frame)),
            "control_response" | "rate_limit_event" | "auth_status" => Ok(Vec::new()),
            unknown => Err(anyhow!("unsupported Claude protocol frame type: {unknown}")),
        }
    }
}

fn claude_account_display_name(account: &Map<String, Value>) -> Option<String> {
    account
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| account.get("organizationName").and_then(Value::as_str))
        .or_else(|| account.get("subscriptionType").and_then(Value::as_str))
        .map(str::to_string)
}

fn parse_model(value: &Value) -> Result<ModelCapability> {
    let id = value
        .get("value")
        .and_then(Value::as_str)
        .context("Claude model is missing value")?
        .to_string();
    let supported_efforts = value
        .get("supportedEffortLevels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|effort| {
            effort
                .as_str()
                .map(|id| ModelEffort {
                    id: id.to_string(),
                    display_name: title_case(id),
                })
                .or_else(|| {
                    Some(ModelEffort {
                        id: effort.get("value")?.as_str()?.to_string(),
                        display_name: effort
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or_else(|| effort.get("value").and_then(Value::as_str).unwrap())
                            .to_string(),
                    })
                })
        })
        .collect();
    Ok(ModelCapability {
        is_default: id == "default",
        display_name: value
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        resolved_model: value
            .get("resolvedModel")
            .and_then(Value::as_str)
            .map(str::to_string),
        default_effort: value
            .get("defaultEffort")
            .and_then(Value::as_str)
            .map(str::to_string),
        context_window: value.get("contextWindow").and_then(Value::as_u64),
        supported_efforts,
        id,
    })
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn parse_system_event(frame: &Value) -> Vec<SubscriptionEvent> {
    if frame.get("subtype").and_then(Value::as_str) != Some("init") {
        return Vec::new();
    }
    frame
        .get("session_id")
        .and_then(Value::as_str)
        .map(|session_id| {
            vec![SubscriptionEvent::SessionStarted(
                SessionIdentity::ClaudeCode(session_id.to_string()),
            )]
        })
        .unwrap_or_default()
}

fn parse_content(content: Option<&Value>, tool_result: bool) -> Result<Vec<SubscriptionEvent>> {
    let Some(content) = content.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut events = Vec::new();
    for block in content {
        let block_type = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    events.push(SubscriptionEvent::TextDelta(text.to_string()));
                }
            }
            "thinking" => {
                if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                    events.push(SubscriptionEvent::ReasoningDelta(thinking.to_string()));
                }
            }
            "tool_use" => events.push(SubscriptionEvent::ToolStarted {
                id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .context("Claude tool use is missing id")?
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(Value::as_str)
                    .context("Claude tool use is missing name")?
                    .to_string(),
                input: block.get("input").cloned().unwrap_or(Value::Null),
            }),
            "tool_result" if tool_result => {
                events.push(SubscriptionEvent::ToolOutput {
                    id: block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .context("Claude tool result is missing tool_use_id")?
                        .to_string(),
                    output: content_text(block.get("content")),
                    is_error: block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
            "tool_result" | "redacted_thinking" => {}
            unknown => return Err(anyhow!("unsupported Claude content block: {unknown}")),
        }
    }
    Ok(events)
}

fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

fn parse_stream_event(frame: &Value) -> Vec<SubscriptionEvent> {
    let Some(event) = frame.get("event") else {
        return Vec::new();
    };
    if event.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return Vec::new();
    }
    let Some(delta) = event.get("delta") else {
        return Vec::new();
    };
    match delta.get("type").and_then(Value::as_str) {
        Some("text_delta") => delta
            .get("text")
            .and_then(Value::as_str)
            .map(|text| vec![SubscriptionEvent::TextDelta(text.to_string())])
            .unwrap_or_default(),
        Some("thinking_delta") => delta
            .get("thinking")
            .and_then(Value::as_str)
            .map(|thinking| vec![SubscriptionEvent::ReasoningDelta(thinking.to_string())])
            .unwrap_or_default(),
        Some("input_json_delta") | Some("signature_delta") | None => Vec::new(),
        Some(_) => Vec::new(),
    }
}

fn parse_control_request(frame: &Value) -> Result<Vec<SubscriptionEvent>> {
    let request = frame
        .get("request")
        .context("Claude control request is missing request")?;
    if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
        return Ok(Vec::new());
    }
    let request_id = frame
        .get("request_id")
        .and_then(Value::as_str)
        .context("Claude approval request is missing request_id")?;
    let tool_name = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    Ok(vec![SubscriptionEvent::ApprovalRequested {
        request_id: request_id.to_string(),
        kind: tool_name.to_string(),
        description: request
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(tool_name)
            .to_string(),
        input: request
            .get("input")
            .cloned()
            .unwrap_or(Value::Object(Map::new())),
    }])
}

fn parse_result(frame: &Value) -> Vec<SubscriptionEvent> {
    let session = frame
        .get("session_id")
        .and_then(Value::as_str)
        .map(|session_id| SessionIdentity::ClaudeCode(session_id.to_string()));
    let mut events = Vec::new();
    if let Some(usage) = parse_usage(frame.get("usage")) {
        events.push(SubscriptionEvent::Usage(usage));
    }
    if frame
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        events.push(SubscriptionEvent::Error {
            message: frame
                .get("result")
                .and_then(Value::as_str)
                .or_else(|| frame.get("error").and_then(Value::as_str))
                .unwrap_or("Claude Code turn failed")
                .to_string(),
            recoverable: session.is_some(),
            session,
        });
    } else if let Some(session) = session {
        events.push(SubscriptionEvent::TurnCompleted { session });
    }
    events
}

fn parse_usage(value: Option<&Value>) -> Option<Usage> {
    let value = value?;
    Some(Usage {
        input_tokens: value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cached_input_tokens: value
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
