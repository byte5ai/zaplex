//! Minimal subset of OpenAI-compatible client: currently only used to fetch `/models` list.
//!
//! Will be extended to full Chat Completions + tool call streaming
//! when multi-agent invocation is implemented in phase 2.

use serde::Deserialize;

use http_client::Client;

/// Single model entry returned by `/models` endpoint.
///
/// We only care about `id` (used as model name for Agent). Other fields (`object`/`created`/`owned_by`)
/// vary significantly across providers, so we ignore them here.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct OpenAiCompatibleModel {
    pub id: String,
    /// Owner inferred from `owned_by`, primarily for UI display, may be empty.
    #[serde(default)]
    pub owned_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<OpenAiCompatibleModel>,
}

/// Errors that may occur during fetch.
#[derive(Debug, thiserror::Error)]
pub enum OpenAiCompatibleError {
    #[error("Invalid base URL: {0}")]
    InvalidBaseUrl(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("HTTP status {status}: {body}")]
    Status { status: u16, body: String },

    #[error("Response parsing failed: {0}")]
    Decode(String),

    #[error("Network/streaming request failed: {0}")]
    Stream(String),

    #[error("Invocation failed: {0}")]
    Other(String),
}

/// Normalize user-input base_url into absolute URL form,
/// tolerating trailing `/`, missing `/v1`, `/openai/v1`, etc.
pub(crate) fn normalize_base_url(input: &str) -> Result<String, OpenAiCompatibleError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(OpenAiCompatibleError::InvalidBaseUrl(
            "base URL cannot be empty".to_string(),
        ));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(OpenAiCompatibleError::InvalidBaseUrl(format!(
            "base URL must start with http:// or https://: {trimmed}"
        )));
    }
    Ok(trimmed.to_string())
}

/// Invoke `${base_url}/models`, return model ID list (deduplicated + sorted alphabetically).
///
/// Auth: if `api_key` is non-empty, send as `Authorization: Bearer ...`.
/// Some local services (e.g., Ollama) allow unauthenticated access, so don't send header when key is empty.
pub async fn fetch_openai_compatible_models(
    client: Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<OpenAiCompatibleModel>, OpenAiCompatibleError> {
    let base = normalize_base_url(base_url)?;
    let url = format!("{base}/models");

    let mut req = client.get(&url);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key);
    }

    let response = req.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(OpenAiCompatibleError::Status {
            status: status.as_u16(),
            body,
        });
    }

    let parsed: ModelsResponse = response
        .json()
        .await
        .map_err(|e| OpenAiCompatibleError::Decode(e.to_string()))?;

    let mut models = parsed.data;
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}
