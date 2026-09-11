//! One-time compatibility cleanup for secrets written by retired AI features.

use warpui::AppContext;
use warpui_extras::secure_storage::{self, AppContextExt};

const RETIRED_SECRET_KEYS: [&str; 2] = ["AgentProviderSecrets", "AiApiKeys"];

pub(crate) fn remove_retired_provider_secrets(ctx: &AppContext) {
    remove_retired_provider_secrets_with(|key| ctx.secure_storage().remove_value(key));
}

fn remove_retired_provider_secrets_with(
    mut remove_value: impl FnMut(&str) -> Result<(), secure_storage::Error>,
) {
    for key in RETIRED_SECRET_KEYS {
        if let Err(error) = remove_value(key) {
            if !matches!(error, secure_storage::Error::NotFound) {
                log::warn!("Failed to remove retired AI provider secrets: {error:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_all_retired_provider_secret_keys() {
        let mut removed_keys = Vec::new();

        remove_retired_provider_secrets_with(|key| {
            removed_keys.push(key.to_string());
            Ok(())
        });

        assert_eq!(
            removed_keys,
            RETIRED_SECRET_KEYS.map(str::to_string).to_vec()
        );
    }

    #[test]
    fn missing_retired_provider_secrets_are_already_clean() {
        let mut calls = 0;
        remove_retired_provider_secrets_with(|_| {
            calls += 1;
            Err(secure_storage::Error::NotFound)
        });
        assert_eq!(calls, RETIRED_SECRET_KEYS.len());
    }

    #[test]
    fn provider_secret_cleanup_continues_after_a_storage_error() {
        let mut attempted_keys = Vec::new();

        remove_retired_provider_secrets_with(|key| {
            attempted_keys.push(key.to_string());
            if key == RETIRED_SECRET_KEYS[0] {
                return Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                    "injected storage failure"
                )));
            }
            Ok(())
        });

        assert_eq!(
            attempted_keys,
            RETIRED_SECRET_KEYS.map(str::to_string).to_vec()
        );
    }

    #[test]
    fn retired_provider_secret_cleanup_is_wired_after_storage_registration() {
        let startup = include_str!("../lib.rs");
        let cleanup = startup
            .find("legacy_secret_cleanup::remove_retired_provider_secrets(ctx)")
            .expect("startup must invoke retired provider secret cleanup");

        for registration in [
            "secure_storage::register_noop",
            "secure_storage::register_with_fallback",
            "secure_storage::register_with_dir",
            "secure_storage::register(&data_domain",
        ] {
            let registration = startup
                .find(registration)
                .expect("every platform must register secure storage");
            assert!(registration < cleanup);
        }
    }
}
