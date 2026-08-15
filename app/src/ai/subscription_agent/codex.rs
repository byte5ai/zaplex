use super::{
    AgentCapability, ApprovalDecision, InstallationIdentity, ModelCapability, ModelEffort,
    SessionIdentity, SubscriptionEvent, SubscriptionTarget, Usage,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

pub(crate) struct CodexProtocol;

impl CodexProtocol {
    pub(crate) fn initialize_request(id: u64, client_version: &str) -> Value {
        request(
            id,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "zaplex",
                    "title": "Zaplex",
                    "version": client_version
                },
                "capabilities": {
                    "experimentalApi": false
                }
            }),
        )
    }

    pub(crate) fn initialized_notification() -> Value {
        json!({
            "method": "initialized",
            "params": {}
        })
    }

    pub(crate) fn account_request(id: u64) -> Value {
        request(id, "account/read", json!({"refreshToken": false}))
    }

    pub(crate) fn model_list_request(id: u64, cursor: Option<&str>) -> Value {
        request(
            id,
            "model/list",
            json!({
                "cursor": cursor,
                "includeHidden": false
            }),
        )
    }

    pub(crate) fn thread_start_request(id: u64, target: &SubscriptionTarget) -> Value {
        request(
            id,
            "thread/start",
            json!({
                "cwd": target.working_directory,
                "model": target.model.id,
                "sandbox": "workspace-write",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "ephemeral": false
            }),
        )
    }

    pub(crate) fn thread_resume_request(
        id: u64,
        target: &SubscriptionTarget,
        thread_id: &str,
    ) -> Value {
        request(
            id,
            "thread/resume",
            json!({
                "threadId": thread_id,
                "cwd": target.working_directory,
                "model": target.model.id,
                "sandbox": "workspace-write",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user"
            }),
        )
    }

    pub(crate) fn turn_start_request(
        id: u64,
        target: &SubscriptionTarget,
        thread_id: &str,
        prompt: &str,
    ) -> Value {
        request(
            id,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{
                    "type": "text",
                    "text": prompt
                }],
                "cwd": target.working_directory,
                "model": target.model.id,
                "effort": target.effort,
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user"
            }),
        )
    }

    pub(crate) fn interrupt_request(id: u64, thread_id: &str, turn_id: &str) -> Value {
        request(
            id,
            "turn/interrupt",
            json!({
                "threadId": thread_id,
                "turnId": turn_id
            }),
        )
    }

    pub(crate) fn approval_response(
        request_id: Value,
        method: &str,
        input: &Value,
        decision: ApprovalDecision,
    ) -> Value {
        let result = match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let decision = match decision {
                    ApprovalDecision::Allow => "accept",
                    ApprovalDecision::AllowForSession => "acceptForSession",
                    ApprovalDecision::Deny => "decline",
                    ApprovalDecision::Cancel => "cancel",
                };
                json!({ "decision": decision })
            }
            "item/permissions/requestApproval" => {
                let permissions = match decision {
                    ApprovalDecision::Allow | ApprovalDecision::AllowForSession => input
                        .get("permissions")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                    ApprovalDecision::Deny | ApprovalDecision::Cancel => json!({}),
                };
                let scope = if decision == ApprovalDecision::AllowForSession {
                    "session"
                } else {
                    "turn"
                };
                json!({
                    "permissions": permissions,
                    "scope": scope
                })
            }
            _ => json!({ "decision": "decline" }),
        };
        json!({
            "id": request_id,
            "result": result
        })
    }

    pub(crate) fn method_not_supported(request_id: Value, method: &str) -> Value {
        json!({
            "id": request_id,
            "error": {
                "code": -32601,
                "message": format!("Zaplex does not support server request {method}")
            }
        })
    }

    pub(crate) fn parse_capability(
        account_frame: &Value,
        model_frame: &Value,
        mut installation: InstallationIdentity,
    ) -> Result<AgentCapability> {
        let account = result(account_frame)?
            .get("account")
            .and_then(Value::as_object)
            .context("Codex is not signed in")?;
        if account.get("type").and_then(Value::as_str) != Some("chatgpt") {
            return Err(anyhow!("Codex is not using a ChatGPT subscription account"));
        }
        if let Some(email) = account.get("email").and_then(Value::as_str) {
            installation.account.display_name = email.to_string();
        } else {
            let plan = account
                .get("planType")
                .and_then(Value::as_str)
                .unwrap_or("ChatGPT");
            installation.account.display_name = plan.to_string();
        }

        let models = result(model_frame)?
            .get("data")
            .and_then(Value::as_array)
            .context("Codex model/list response is missing data")?
            .iter()
            .filter(|model| {
                !model
                    .get("hidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .map(parse_model)
            .collect::<Result<Vec<_>>>()?;
        if models.is_empty() {
            return Err(anyhow!("Codex reported no available models"));
        }
        Ok(AgentCapability {
            installation,
            models,
        })
    }

    pub(crate) fn parse_thread_response(frame: &Value) -> Result<SessionIdentity> {
        let thread_id = result(frame)?
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex thread response is missing thread.id")?;
        Ok(SessionIdentity::Codex(thread_id.to_string()))
    }

    pub(crate) fn parse_event(frame: &Value) -> Result<Vec<SubscriptionEvent>> {
        if frame.get("error").is_some() && frame.get("method").is_none() {
            return Ok(vec![SubscriptionEvent::Error {
                message: error_message(frame.get("error")),
                recoverable: true,
                session: None,
            }]);
        }
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            return Ok(Vec::new());
        };
        let params = frame.get("params").unwrap_or(&Value::Null);
        match method {
            "thread/started" => session_event(params),
            "item/agentMessage/delta" => {
                string_event(params, "delta", SubscriptionEvent::TextDelta)
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                string_event(params, "delta", SubscriptionEvent::ReasoningDelta)
            }
            "item/started" => parse_item_started(params),
            "item/completed" => parse_item_completed(params),
            "item/commandExecution/outputDelta"
            | "item/fileChange/outputDelta"
            | "item/mcpToolCall/progress" => parse_tool_delta(params),
            "turn/diff/updated" | "item/fileChange/patchUpdated" => {
                let diff = params
                    .get("diff")
                    .or_else(|| params.get("patch"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Ok((!diff.is_empty())
                    .then(|| SubscriptionEvent::Diff(diff.to_string()))
                    .into_iter()
                    .collect())
            }
            "thread/tokenUsage/updated" => Ok(parse_usage(params)
                .map(SubscriptionEvent::Usage)
                .into_iter()
                .collect()),
            "turn/completed" => parse_turn_completed(params),
            "error" => Ok(vec![SubscriptionEvent::Error {
                message: error_message(params.get("error").or(Some(params))),
                recoverable: true,
                session: session_from_params(params),
            }]),
            "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval" => parse_approval(frame, method, params),
            "turn/started"
            | "turn/plan/updated"
            | "item/plan/delta"
            | "item/reasoning/summaryPartAdded"
            | "account/updated"
            | "account/rateLimits/updated"
            | "model/rerouted"
            | "model/verification" => Ok(Vec::new()),
            unknown => Err(anyhow!("unsupported Codex app-server method: {unknown}")),
        }
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "id": id,
        "method": method,
        "params": params
    })
}

fn result(frame: &Value) -> Result<&Value> {
    if let Some(error) = frame.get("error") {
        return Err(anyhow!(error_message(Some(error))));
    }
    frame
        .get("result")
        .context("Codex app-server response is missing result")
}

fn parse_model(value: &Value) -> Result<ModelCapability> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .context("Codex model is missing id")?;
    let supported_efforts = value
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|effort| {
            let id = effort
                .get("reasoningEffort")
                .and_then(Value::as_str)
                .context("Codex reasoning effort is missing reasoningEffort")?;
            Ok(ModelEffort {
                id: id.to_string(),
                display_name: effort
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ModelCapability {
        id: id.to_string(),
        display_name: value
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        resolved_model: value
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_default: value
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default_effort: value
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_string),
        context_window: value.get("contextWindow").and_then(Value::as_u64),
        supported_efforts,
    })
}

fn session_event(params: &Value) -> Result<Vec<SubscriptionEvent>> {
    let session = session_from_params(params).context("Codex thread event is missing thread id")?;
    Ok(vec![SubscriptionEvent::SessionStarted(session)])
}

fn session_from_params(params: &Value) -> Option<SessionIdentity> {
    params
        .pointer("/thread/id")
        .or_else(|| params.get("threadId"))
        .and_then(Value::as_str)
        .map(|id| SessionIdentity::Codex(id.to_string()))
}

fn string_event(
    params: &Value,
    field: &str,
    event: impl FnOnce(String) -> SubscriptionEvent,
) -> Result<Vec<SubscriptionEvent>> {
    Ok(params
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| event(value.to_string()))
        .into_iter()
        .collect())
}

fn parse_item_started(params: &Value) -> Result<Vec<SubscriptionEvent>> {
    let Some(item) = params.get("item") else {
        return Ok(Vec::new());
    };
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("tool");
    if matches!(item_type, "agentMessage" | "reasoning" | "plan") {
        return Ok(Vec::new());
    }
    Ok(vec![SubscriptionEvent::ToolStarted {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .context("Codex item is missing id")?
            .to_string(),
        name: item_type.to_string(),
        input: item.clone(),
    }])
}

fn parse_item_completed(params: &Value) -> Result<Vec<SubscriptionEvent>> {
    let Some(item) = params.get("item") else {
        return Ok(Vec::new());
    };
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if matches!(item_type, "agentMessage" | "reasoning" | "plan") {
        return Ok(Vec::new());
    }
    Ok(vec![SubscriptionEvent::ToolOutput {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .context("completed Codex item is missing id")?
            .to_string(),
        output: item
            .get("aggregatedOutput")
            .or_else(|| item.get("result"))
            .or_else(|| item.get("output"))
            .map(value_text)
            .unwrap_or_default(),
        is_error: matches!(
            item.get("status").and_then(Value::as_str),
            Some("failed" | "declined")
        ),
    }])
}

fn parse_tool_delta(params: &Value) -> Result<Vec<SubscriptionEvent>> {
    let id = params
        .get("itemId")
        .and_then(Value::as_str)
        .context("Codex tool delta is missing itemId")?;
    let output = params
        .get("delta")
        .or_else(|| params.get("message"))
        .map(value_text)
        .unwrap_or_default();
    Ok((!output.is_empty())
        .then(|| SubscriptionEvent::ToolOutput {
            id: id.to_string(),
            output,
            is_error: false,
        })
        .into_iter()
        .collect())
}

fn parse_approval(frame: &Value, method: &str, params: &Value) -> Result<Vec<SubscriptionEvent>> {
    let request_id = frame
        .get("id")
        .context("Codex approval request is missing id")?;
    Ok(vec![SubscriptionEvent::ApprovalRequested {
        request_id: request_id.to_string(),
        kind: method.to_string(),
        description: params
            .get("reason")
            .and_then(Value::as_str)
            .or_else(|| params.get("command").and_then(Value::as_str))
            .unwrap_or(method)
            .to_string(),
        input: params.clone(),
    }])
}

fn parse_turn_completed(params: &Value) -> Result<Vec<SubscriptionEvent>> {
    let session =
        session_from_params(params).context("Codex turn completion is missing its thread id")?;
    let status = params
        .pointer("/turn/status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    if status == "completed" {
        Ok(vec![SubscriptionEvent::TurnCompleted { session }])
    } else {
        Ok(vec![SubscriptionEvent::Error {
            message: params
                .pointer("/turn/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Codex turn failed")
                .to_string(),
            recoverable: true,
            session: Some(session),
        }])
    }
}

fn parse_usage(params: &Value) -> Option<Usage> {
    let usage = params.pointer("/tokenUsage/last")?;
    Some(Usage {
        input_tokens: usage
            .get("inputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cached_input_tokens: usage
            .get("cachedInputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn error_message(error: Option<&Value>) -> String {
    error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .or_else(|| error.and_then(Value::as_str))
        .unwrap_or("Codex app-server request failed")
        .to_string()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(values) => values.iter().map(value_text).collect::<Vec<_>>().join("\n"),
        Value::Object(object) => object
            .get("text")
            .map(value_text)
            .unwrap_or_else(|| Value::Object(object.clone()).to_string()),
        Value::Null => String::new(),
        Value::Bool(_) | Value::Number(_) => value.to_string(),
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
