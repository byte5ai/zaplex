//! `ProxyCredentials`: store proxy Basic Auth password in OS keychain (see Issue #72).
//!
//! Only stores password; non-sensitive fields like username, URL remain in `NetworkSettings`'s settings.toml.
//! Design mirrors `crate::ai::agent_providers::AgentProviderSecrets`: based on
//! `warpui_extras::secure_storage` (macOS Keychain / Windows DPAPI / Linux Keyring).
//!
//! Note: proxy has only one global password, so storage has one key and value is the raw password
//! string (no longer uses JSON map).

use warpui::{Entity, ModelContext, SingletonEntity};
use warpui_extras::secure_storage::{self, AppContextExt};

const SECURE_STORAGE_KEY: &str = "ProxyPassword";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyCredentialsEvent {
    /// Password value changed (may be empty).
    PasswordChanged,
}

/// Singleton: manages the global HTTP proxy's Basic Auth password.
pub struct ProxyCredentials {
    password: String,
}

impl ProxyCredentials {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        Self {
            password: Self::load_from_storage(ctx),
        }
    }

    /// Read current password; returns empty string if no value.
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Set / update password. Passing empty string is equivalent to deletion.
    pub fn set_password(&mut self, password: String, ctx: &mut ModelContext<Self>) {
        if self.password == password {
            return;
        }
        self.password = password;
        self.persist(ctx);
        ctx.emit(ProxyCredentialsEvent::PasswordChanged);
    }

    fn load_from_storage(ctx: &mut ModelContext<Self>) -> String {
        match ctx.secure_storage().read_value(SECURE_STORAGE_KEY) {
            Ok(value) => value,
            Err(secure_storage::Error::NotFound) => String::new(),
            Err(e) => {
                log::error!("Failed to read proxy password: {e:#}");
                String::new()
            }
        }
    }

    fn persist(&self, ctx: &mut ModelContext<Self>) {
        if self.password.is_empty() {
            // Empty string semantics: "no password"; accept delete failure, only log.
            // Avoid let-chain (app crate is Rust 2021), split into two checks.
            if let Err(e) = ctx.secure_storage().remove_value(SECURE_STORAGE_KEY) {
                if !matches!(e, secure_storage::Error::NotFound) {
                    log::error!("Failed to remove proxy password: {e:#}");
                }
            }
            return;
        }
        if let Err(e) = ctx
            .secure_storage()
            .write_value(SECURE_STORAGE_KEY, &self.password)
        {
            log::error!("Failed to write proxy password: {e:#}");
        }
    }
}

impl Entity for ProxyCredentials {
    type Event = ProxyCredentialsEvent;
}

impl SingletonEntity for ProxyCredentials {}
