//! Custom AI provider support.
//!
//! This module is responsible for:
//! - Securely storing each Provider's `api_key` in OS keychain (secure_storage),
//!   while Provider metadata (name/base_url/model list) goes through regular settings.toml.
//! - Calling `${base_url}/models` via `OpenAiCompatibleClient`
//!   to fetch the list of available upstream models (for UI "Fetch models" button).
//!
//! The second phase will implement the `AiProvider` trait based on this configuration
//! to route Agent multi-agent calls to local providers.

pub mod active_ai;
pub mod attachment_caps;
pub mod chat_stream;
pub mod llm_id;
pub mod models_dev;
pub mod oneshot;
pub mod openai_compatible;
pub mod prompt_renderer;
pub mod reasoning;
pub mod secrets;
pub mod tools;
pub mod user_context;

#[cfg(test)]
mod cache_stability_tests;

// Current external usage points:
// - `fetch_openai_compatible_models`: FetchAgentProviderModels handler in ai_page.rs
// - `AgentProviderSecrets`: multiple handlers in ai_page.rs and registration point in lib.rs
// Remaining symbols (`OpenAiCompatibleError`/`OpenAiCompatibleModel`/`AgentProviderSecretsEvent`)
// are still accessible via full paths like `crate::ai::agent_providers::openai_compatible::*`,
// not re-exported here to avoid `unused_imports` warnings.
pub use openai_compatible::fetch_openai_compatible_models;
pub use secrets::AgentProviderSecrets;

// ---------------------------------------------------------------------------
// LLMInfo Synthesis: convert agent_providers configured in settings into picker-ready form
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use settings::Setting;
use warpui::{AppContext, SingletonEntity};

use crate::ai::llms::{
    AvailableLLMs, DisableReason, LLMContextWindow, LLMInfo, LLMProvider, LLMUsageMetadata,
    ModelsByFeature,
};
use crate::settings::{AISettings, AgentProvider};

/// Synthesize a list of LLMInfo for all valid (provider, model) pairs for a given provider.
///
/// "Valid" = provider has non-empty base_url + at least 1 model.
/// **API key is optional**: local unauthenticated providers (ollama / lm-studio / vllm, etc.)
/// can leave it empty; models are still exposed to the picker when key is missing; requests
/// are still sent at runtime, just without `Authorization`. Invalid providers (missing base_url
/// or no models) are entirely skipped; their models don't appear in the picker, so users clearly
/// see "which providers are incomplete → not shown".
fn build_byop_llm_infos(app: &AppContext) -> Vec<LLMInfo> {
    let providers = AISettings::as_ref(app).agent_providers.value().clone();
    let mut out = Vec::new();

    for provider in providers {
        if provider.base_url.trim().is_empty() {
            continue;
        }
        if provider.models.is_empty() {
            continue;
        }

        let provider_label = if provider.name.trim().is_empty() {
            provider.id.clone()
        } else {
            provider.name.clone()
        };

        for model in &provider.models {
            if model.id.trim().is_empty() {
                continue;
            }
            let display_name = if model.name.trim().is_empty() {
                model.id.clone()
            } else {
                model.name.clone()
            };
            // Three-level priority for resolving final capabilities: user-forced tri-state chip toggle in settings →
            // models.dev catalog inference → substring fallback.
            // This same function is used by chat_stream when deciding to insert ContentPart::Binary;
            // UI display and runtime behavior are always consistent.
            let resolved_caps =
                attachment_caps::resolve_for_model(&provider.id, provider.api_type, model);
            let vision_supported = resolved_caps.images;
            out.push(LLMInfo {
                display_name: format!("{provider_label} / {display_name}"),
                base_model_name: format!("{provider_label} / {display_name}"),
                id: llm_id::encode(&provider.id, &model.id),
                reasoning_level: None,
                usage_metadata: LLMUsageMetadata {
                    request_multiplier: 1,
                    credit_multiplier: None,
                },
                description: None,
                disable_reason: None,
                vision_supported,
                spec: None,
                provider: LLMProvider::Unknown,
                host_configs: HashMap::new(),
                discount_percentage: None,
                context_window: LLMContextWindow::default(),
            });
        }
    }

    out
}

/// Placeholder entry: when the user has not configured any valid provider, the picker
/// must have at least 1 entry (`AvailableLLMs::new` rejects empty lists). This entry is
/// grayed out with `DisableReason::Unavailable`, unselectable, and prompts the user to configure it in settings.
fn placeholder_llm_info() -> LLMInfo {
    LLMInfo {
        display_name: "No custom providers configured — add one in Settings → AI".to_owned(),
        base_model_name: "Unconfigured".to_owned(),
        id: ai::LLMId::from("byop-placeholder"),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: Some(DisableReason::Unavailable),
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

/// Construct a `ModelsByFeature` filled entirely with BYOP models.
/// All 4 features (agent_mode / coding / cli_agent / computer_use) use the same model set —
/// custom providers don't distinguish by capability; all models can be used for any feature.
pub fn build_byop_models_by_feature(app: &AppContext) -> ModelsByFeature {
    let mut choices = build_byop_llm_infos(app);
    if choices.is_empty() {
        choices.push(placeholder_llm_info());
    }

    let default_id = choices[0].id.clone();
    let make = || {
        AvailableLLMs::new(default_id.clone(), choices.clone(), None)
            .expect("choices is non-empty by construction")
    };

    ModelsByFeature {
        agent_mode: make(),
        coding: make(),
        cli_agent: Some(make()),
        computer_use: Some(make()),
    }
}

/// Given a BYOP `LLMId`, look up `(provider, api_key, model_id)` from `AISettings` and secrets.
/// Returns `None` if any information is missing (the controller caller should map this to an `InvalidApiKey` error).
pub fn lookup_byop(app: &AppContext, id: &ai::LLMId) -> Option<(AgentProvider, String, String)> {
    let (provider_id, model_id) = llm_id::decode(id)?;
    let providers = AISettings::as_ref(app).agent_providers.value().clone();
    let provider = providers.into_iter().find(|p| p.id == provider_id)?;
    // API key is optional: when absent, return empty string, which downstream build_client
    // passes to genai as `AuthData::from_single("")` — no `Authorization` header, adapting
    // to local unauthenticated services like ollama.
    let api_key = AgentProviderSecrets::as_ref(app)
        .get(&provider_id)
        .map(str::to_owned)
        .unwrap_or_default();
    Some((provider, api_key, model_id))
}
