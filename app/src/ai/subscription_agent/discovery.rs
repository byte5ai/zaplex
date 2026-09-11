use super::{
    AgentCapability, ClaudeProtocol, CodexProtocol, InstallationIdentity, JsonLineProcess,
    ProcessLaunch, ProcessLocation, SubscriptionAgent,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

const CLAUDE_INITIALIZE_ID: &str = "zaplex-initialize";
const CODEX_INITIALIZE_ID: u64 = 1;
const CODEX_ACCOUNT_ID: u64 = 2;
const CODEX_ACCOUNT_RATE_LIMITS_ID: u64 = 3;
const CODEX_MODELS_ID: u64 = 4;

pub(crate) async fn discover_capabilities(
    installation: InstallationIdentity,
    working_directory: PathBuf,
    location: ProcessLocation,
) -> Result<AgentCapability> {
    let launch = ProcessLaunch::for_discovery(&installation, working_directory, location);
    let mut process = JsonLineProcess::spawn(&launch)?;
    let result = match installation.agent {
        SubscriptionAgent::ClaudeCode => discover_claude(&mut process, installation).await,
        SubscriptionAgent::Codex => discover_codex(&mut process, installation).await,
    };
    if let Err(error) = process.terminate().await {
        log::warn!("Failed to end subscription-agent discovery process: {error:#}");
    }
    result
}

async fn discover_claude(
    process: &mut JsonLineProcess,
    installation: InstallationIdentity,
) -> Result<AgentCapability> {
    process
        .send(&ClaudeProtocol::initialize_request(CLAUDE_INITIALIZE_ID))
        .await?;
    loop {
        let frame = process
            .receive()
            .await?
            .context("Claude Code exited before capability discovery completed")?;
        if frame.get("type").and_then(Value::as_str) == Some("control_response")
            && frame
                .pointer("/response/request_id")
                .and_then(Value::as_str)
                == Some(CLAUDE_INITIALIZE_ID)
        {
            let capability = ClaudeProtocol::parse_capability(&frame, installation);
            return match capability {
                Err(error)
                    if error
                        .downcast_ref::<super::SubscriptionAuthenticationError>()
                        .is_some() =>
                {
                    Err(error)
                }
                result => result.context(
                    "installed Claude Code does not support the required structured protocol",
                ),
            };
        }
    }
}

async fn discover_codex(
    process: &mut JsonLineProcess,
    installation: InstallationIdentity,
) -> Result<AgentCapability> {
    process
        .send(&CodexProtocol::initialize_request(
            CODEX_INITIALIZE_ID,
            env!("CARGO_PKG_VERSION"),
        ))
        .await?;
    let initialize = receive_response(process, CODEX_INITIALIZE_ID).await?;
    if let Some(error) = initialize.get("error") {
        return Err(anyhow!(
            "installed Codex {} does not support the required app-server protocol: {}",
            installation.version,
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("initialization failed")
        ));
    }
    process
        .send(&CodexProtocol::initialized_notification())
        .await?;
    process
        .send(&CodexProtocol::account_request(CODEX_ACCOUNT_ID))
        .await?;
    let account = receive_response(process, CODEX_ACCOUNT_ID).await?;
    process
        .send(&CodexProtocol::account_rate_limits_request(
            CODEX_ACCOUNT_RATE_LIMITS_ID,
        ))
        .await?;
    let account_rate_limits = receive_response(process, CODEX_ACCOUNT_RATE_LIMITS_ID).await?;
    if let Some(error) = account_rate_limits.get("error") {
        return Err(anyhow!(
            "installed Codex {} cannot verify the selected subscription account; upgrade Codex: {}",
            installation.version,
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("account/rateLimits/read failed")
        ));
    }

    let mut cursor = None;
    let mut models = Vec::new();
    loop {
        process
            .send(&CodexProtocol::model_list_request(
                CODEX_MODELS_ID,
                cursor.as_deref(),
            ))
            .await?;
        let response = receive_response(process, CODEX_MODELS_ID).await?;
        if let Some(error) = response.get("error") {
            return Err(anyhow!(
                "installed Codex {} cannot list models: {}",
                installation.version,
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("model/list failed")
            ));
        }
        let result = response
            .get("result")
            .context("Codex model/list response is missing result")?;
        models.extend(
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
    let model_response = json!({
        "id": CODEX_MODELS_ID,
        "result": {
            "data": models,
            "nextCursor": null
        }
    });
    CodexProtocol::parse_capability(
        &account,
        &account_rate_limits,
        &model_response,
        installation,
    )
}

async fn receive_response(process: &mut JsonLineProcess, id: u64) -> Result<Value> {
    loop {
        let frame = process
            .receive()
            .await?
            .with_context(|| format!("app-server exited while waiting for response {id}"))?;
        if frame.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(frame);
        }
    }
}
