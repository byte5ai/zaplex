use super::{
    ApprovalDecision, ClaudeProtocol, CodexProtocol, JsonLineProcess, ProcessLaunch,
    ProcessLocation, SessionIdentity, SubscriptionAgent, SubscriptionEvent, SubscriptionTarget,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};

const CLAUDE_INITIALIZE_ID: &str = "zaplex-session-initialize";

pub(crate) struct SubscriptionSession {
    process: JsonLineProcess,
    target: SubscriptionTarget,
    session: Option<SessionIdentity>,
    current_turn_id: Option<String>,
    next_request_id: u64,
    pending_approvals: HashMap<String, Value>,
    queued_events: VecDeque<SubscriptionEvent>,
}

impl SubscriptionSession {
    pub(crate) async fn open(
        target: SubscriptionTarget,
        resume: Option<SessionIdentity>,
        location: ProcessLocation,
    ) -> Result<Self> {
        validate_resume_agent(target.installation.agent, resume.as_ref())?;
        let resume_id = resume.as_ref().map(session_id);
        let launch = ProcessLaunch::for_session(&target, resume_id, location);
        let process = JsonLineProcess::spawn(&launch)?;
        let mut session = Self {
            process,
            target,
            session: resume,
            current_turn_id: None,
            next_request_id: 1,
            pending_approvals: HashMap::new(),
            queued_events: VecDeque::new(),
        };
        match session.target.installation.agent {
            SubscriptionAgent::ClaudeCode => session.initialize_claude().await?,
            SubscriptionAgent::Codex => session.initialize_codex().await?,
        }
        Ok(session)
    }

    pub(crate) fn identity(&self) -> Option<&SessionIdentity> {
        self.session.as_ref()
    }

    pub(crate) async fn send_prompt(&mut self, prompt: &str) -> Result<()> {
        match self.target.installation.agent {
            SubscriptionAgent::ClaudeCode => {
                self.process
                    .send(&ClaudeProtocol::user_message(
                        prompt,
                        self.session.as_ref().map(session_id),
                    ))
                    .await
            }
            SubscriptionAgent::Codex => {
                let thread_id = self
                    .session
                    .as_ref()
                    .map(session_id)
                    .context("Codex thread has not been initialized")?
                    .to_string();
                let request_id = self.request_id();
                self.process
                    .send(&CodexProtocol::turn_start_request(
                        request_id,
                        &self.target,
                        &thread_id,
                        prompt,
                    ))
                    .await
            }
        }
    }

    pub(crate) async fn next_event(&mut self) -> Result<Option<SubscriptionEvent>> {
        if let Some(event) = self.queued_events.pop_front() {
            return Ok(Some(event));
        }
        loop {
            let Some(frame) = self.process.receive().await? else {
                return Ok(Some(SubscriptionEvent::Error {
                    message: format!(
                        "{} exited unexpectedly",
                        self.target.installation.agent.display_name()
                    ),
                    recoverable: self.session.is_some(),
                    session: self.session.clone(),
                }));
            };
            if self.target.installation.agent == SubscriptionAgent::Codex {
                if self.capture_codex_response(&frame) {
                    continue;
                }
                if self.reject_unsupported_codex_request(&frame).await? {
                    continue;
                }
            }
            let mut events = match self.target.installation.agent {
                SubscriptionAgent::ClaudeCode => ClaudeProtocol::parse_event(&frame)?,
                SubscriptionAgent::Codex => CodexProtocol::parse_event(&frame)?,
            };
            self.capture_session_and_approvals(&frame, &events);
            if events.is_empty() {
                continue;
            }
            let first = events.remove(0);
            self.queued_events.extend(events);
            return Ok(Some(first));
        }
    }

    pub(crate) async fn respond_to_approval(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<()> {
        let raw = self
            .pending_approvals
            .remove(request_id)
            .with_context(|| format!("approval request {request_id} is no longer pending"))?;
        match self.target.installation.agent {
            SubscriptionAgent::ClaudeCode => {
                let input = raw
                    .get("request")
                    .and_then(|request| request.get("input"))
                    .cloned()
                    .unwrap_or(Value::Null);
                self.process
                    .send(&ClaudeProtocol::approval_response(
                        request_id, decision, &input,
                    ))
                    .await
            }
            SubscriptionAgent::Codex => {
                let rpc_id = raw
                    .get("id")
                    .cloned()
                    .context("Codex approval request is missing its JSON-RPC id")?;
                let method = raw
                    .get("method")
                    .and_then(Value::as_str)
                    .context("Codex approval request is missing its method")?;
                let input = raw.get("params").unwrap_or(&Value::Null);
                self.process
                    .send(&CodexProtocol::approval_response(
                        rpc_id, method, input, decision,
                    ))
                    .await?;
                Ok(())
            }
        }
    }

    pub(crate) async fn cancel(&mut self) -> Result<()> {
        match self.target.installation.agent {
            SubscriptionAgent::ClaudeCode => {
                let request_id = format!("zaplex-interrupt-{}", self.request_id());
                self.process
                    .send(&ClaudeProtocol::interrupt_request(&request_id))
                    .await
            }
            SubscriptionAgent::Codex => {
                let thread_id = self
                    .session
                    .as_ref()
                    .map(session_id)
                    .context("Codex thread has not been initialized")?
                    .to_string();
                let turn_id = self
                    .current_turn_id
                    .as_deref()
                    .context("Codex has no active turn to interrupt")?
                    .to_string();
                let request_id = self.request_id();
                self.process
                    .send(&CodexProtocol::interrupt_request(
                        request_id, &thread_id, &turn_id,
                    ))
                    .await
            }
        }
    }

    pub(crate) async fn end(&mut self) -> Result<()> {
        self.pending_approvals.clear();
        let termination = self.process.terminate().await?;
        log::debug!("Subscription-agent process ended: {termination:?}");
        Ok(())
    }

    async fn initialize_claude(&mut self) -> Result<()> {
        self.process
            .send(&ClaudeProtocol::initialize_request(CLAUDE_INITIALIZE_ID))
            .await?;
        loop {
            let frame = self
                .process
                .receive()
                .await?
                .context("Claude Code exited during structured initialization")?;
            if frame.get("type").and_then(Value::as_str) == Some("control_response")
                && frame
                    .pointer("/response/request_id")
                    .and_then(Value::as_str)
                    == Some(CLAUDE_INITIALIZE_ID)
            {
                let capability =
                    ClaudeProtocol::parse_capability(&frame, self.target.installation.clone())?;
                if !capability
                    .models
                    .iter()
                    .any(|model| model.id == self.target.model.id)
                {
                    return Err(anyhow!(
                        "selected Claude model {} is no longer available",
                        self.target.model.id
                    ));
                }
                return Ok(());
            }
        }
    }

    async fn initialize_codex(&mut self) -> Result<()> {
        let initialize_id = self.request_id();
        self.process
            .send(&CodexProtocol::initialize_request(
                initialize_id,
                env!("CARGO_PKG_VERSION"),
            ))
            .await?;
        ensure_success(
            receive_response(&mut self.process, initialize_id).await?,
            "Codex app-server initialize",
        )?;
        self.process
            .send(&CodexProtocol::initialized_notification())
            .await?;

        let account_id = self.request_id();
        self.process
            .send(&CodexProtocol::account_request(account_id))
            .await?;
        let account = receive_response(&mut self.process, account_id).await?;
        let mut cursor = None;
        let mut model_data = Vec::new();
        loop {
            let models_id = self.request_id();
            self.process
                .send(&CodexProtocol::model_list_request(
                    models_id,
                    cursor.as_deref(),
                ))
                .await?;
            let models = receive_response(&mut self.process, models_id).await?;
            ensure_success(models.clone(), "Codex model/list")?;
            let result = models
                .get("result")
                .context("Codex model/list response is missing result")?;
            model_data.extend(
                result
                    .get("data")
                    .and_then(Value::as_array)
                    .context("Codex model/list response is missing data")?
                    .iter()
                    .cloned(),
            );
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        let models = json!({
            "result": {
                "data": model_data,
                "nextCursor": null,
            }
        });
        let capability =
            CodexProtocol::parse_capability(&account, &models, self.target.installation.clone())?;
        if !capability
            .models
            .iter()
            .any(|model| model.id == self.target.model.id)
        {
            return Err(anyhow!(
                "selected Codex model {} is no longer available",
                self.target.model.id
            ));
        }

        let thread_request_id = self.request_id();
        let request = match self.session.as_ref() {
            Some(SessionIdentity::Codex(thread_id)) => {
                CodexProtocol::thread_resume_request(thread_request_id, &self.target, thread_id)
            }
            None => CodexProtocol::thread_start_request(thread_request_id, &self.target),
            Some(SessionIdentity::ClaudeCode(_)) => {
                return Err(anyhow!("cannot resume a Claude session with Codex"));
            }
        };
        self.process.send(&request).await?;
        let response = receive_response(&mut self.process, thread_request_id).await?;
        self.session = Some(CodexProtocol::parse_thread_response(&response)?);
        Ok(())
    }

    fn request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    fn capture_codex_response(&mut self, frame: &Value) -> bool {
        if frame.get("method").is_some() || frame.get("id").is_none() {
            return false;
        }
        if let Some(turn_id) = frame.pointer("/result/turn/id").and_then(Value::as_str) {
            self.current_turn_id = Some(turn_id.to_string());
        }
        if frame.get("error").is_some() {
            match CodexProtocol::parse_event(frame) {
                Ok(events) => self.queued_events.extend(events),
                Err(error) => self.queued_events.push_back(SubscriptionEvent::Error {
                    message: error.to_string(),
                    recoverable: self.session.is_some(),
                    session: self.session.clone(),
                }),
            }
        }
        true
    }

    async fn reject_unsupported_codex_request(&mut self, frame: &Value) -> Result<bool> {
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };
        let Some(request_id) = frame.get("id").cloned() else {
            return Ok(false);
        };
        if matches!(
            method,
            "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "item/permissions/requestApproval"
        ) {
            return Ok(false);
        }
        self.process
            .send(&CodexProtocol::method_not_supported(request_id, method))
            .await?;
        Ok(true)
    }

    fn capture_session_and_approvals(&mut self, frame: &Value, events: &[SubscriptionEvent]) {
        for event in events {
            match event {
                SubscriptionEvent::SessionStarted(session)
                | SubscriptionEvent::TurnCompleted { session } => {
                    self.session = Some(session.clone());
                    if matches!(event, SubscriptionEvent::TurnCompleted { .. }) {
                        self.current_turn_id = None;
                    }
                }
                SubscriptionEvent::ApprovalRequested { request_id, .. } => {
                    self.pending_approvals
                        .insert(request_id.clone(), frame.clone());
                }
                SubscriptionEvent::TextDelta(_)
                | SubscriptionEvent::ReasoningDelta(_)
                | SubscriptionEvent::ToolStarted { .. }
                | SubscriptionEvent::ToolOutput { .. }
                | SubscriptionEvent::Diff(_)
                | SubscriptionEvent::Usage(_)
                | SubscriptionEvent::Error { .. } => {}
            }
        }
    }
}

fn validate_resume_agent(
    agent: SubscriptionAgent,
    session: Option<&SessionIdentity>,
) -> Result<()> {
    match (agent, session) {
        (SubscriptionAgent::ClaudeCode, Some(SessionIdentity::Codex(_))) => {
            Err(anyhow!("cannot resume a Codex thread with Claude Code"))
        }
        (SubscriptionAgent::Codex, Some(SessionIdentity::ClaudeCode(_))) => {
            Err(anyhow!("cannot resume a Claude session with Codex"))
        }
        (SubscriptionAgent::ClaudeCode, Some(SessionIdentity::ClaudeCode(_)) | None)
        | (SubscriptionAgent::Codex, Some(SessionIdentity::Codex(_)) | None) => Ok(()),
    }
}

fn session_id(session: &SessionIdentity) -> &str {
    match session {
        SessionIdentity::ClaudeCode(id) | SessionIdentity::Codex(id) => id,
    }
}

fn ensure_success(frame: Value, action: &str) -> Result<Value> {
    if let Some(error) = frame.get("error") {
        return Err(anyhow!(
            "{action} failed: {}",
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown app-server error")
        ));
    }
    Ok(frame)
}

async fn receive_response(process: &mut JsonLineProcess, id: u64) -> Result<Value> {
    loop {
        let frame = process
            .receive()
            .await?
            .with_context(|| format!("Codex app-server exited while waiting for response {id}"))?;
        if frame.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(frame);
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
