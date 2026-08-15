//! Compatibility singleton for the retired custom-provider secret store.
//!
//! BYOP is no longer a supported execution path. The singleton remains temporarily so old
//! settings code can deserialize without a startup panic, but it never returns or persists a key.

use std::collections::HashMap;

use warpui::{Entity, ModelContext, SingletonEntity};
use warpui_extras::secure_storage::{self, AppContextExt};

const SECURE_STORAGE_KEY: &str = "AgentProviderSecrets";

/// Emitted when any Provider's API key changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentProviderSecretsEvent {
    KeysUpdated,
}

/// Compatibility singleton that keeps the retired provider-secret store empty.
pub struct AgentProviderSecrets {
    keys: HashMap<String, String>,
}

impl AgentProviderSecrets {
    /// Delete the retired value on startup and expose an empty compatibility store.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        if let Err(error) = ctx.secure_storage().remove_value(SECURE_STORAGE_KEY) {
            if !matches!(error, secure_storage::Error::NotFound) {
                log::warn!("Failed to remove retired agent-provider secrets: {error:#}");
            }
        }
        Self {
            keys: HashMap::new(),
        }
    }

    /// Get the API key for a specified Provider; returns `None` if not configured.
    pub fn get(&self, provider_id: &str) -> Option<&str> {
        self.keys.get(provider_id).map(String::as_str)
    }

    /// Ignore writes from stale settings surfaces and keep the retired key deleted.
    pub fn set(&mut self, provider_id: &str, api_key: String, ctx: &mut ModelContext<Self>) {
        drop(api_key);
        self.keys.remove(provider_id);
        ctx.emit(AgentProviderSecretsEvent::KeysUpdated);
        Self::remove_retired_value(ctx);
    }

    /// Delete a Provider (along with its secret).
    pub fn remove(&mut self, provider_id: &str, ctx: &mut ModelContext<Self>) {
        if self.keys.remove(provider_id).is_some() {
            ctx.emit(AgentProviderSecretsEvent::KeysUpdated);
        }
        Self::remove_retired_value(ctx);
    }

    fn remove_retired_value(ctx: &mut ModelContext<Self>) {
        if let Err(error) = ctx.secure_storage().remove_value(SECURE_STORAGE_KEY) {
            if !matches!(error, secure_storage::Error::NotFound) {
                log::warn!("Failed to remove retired agent-provider secrets: {error:#}");
            }
        }
    }
}

impl Entity for AgentProviderSecrets {
    type Event = AgentProviderSecretsEvent;
}

impl SingletonEntity for AgentProviderSecrets {}
