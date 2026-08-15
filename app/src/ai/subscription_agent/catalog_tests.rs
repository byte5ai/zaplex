use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelEffort, SubscriptionAgent,
};
use std::path::PathBuf;

fn installation(version: &str) -> InstallationIdentity {
    InstallationIdentity {
        agent: SubscriptionAgent::ClaudeCode,
        host: HostIdentity {
            id: "local".to_string(),
            display_name: "Local".to_string(),
        },
        account: AccountIdentity {
            id: "account-1".to_string(),
            display_name: "developer@example.com".to_string(),
            config_dir: Some(PathBuf::from("/accounts/claude-1")),
        },
        executable: PathBuf::from("/usr/bin/claude"),
        version: version.to_string(),
    }
}

fn model(id: &str, description: &str) -> ModelCapability {
    ModelCapability {
        id: id.to_string(),
        display_name: id.to_string(),
        description: Some(description.to_string()),
        resolved_model: Some(format!("{id}-resolved")),
        is_default: false,
        supported_efforts: vec![ModelEffort {
            id: "high".to_string(),
            display_name: "High".to_string(),
        }],
        default_effort: Some("high".to_string()),
        context_window: None,
    }
}

#[test]
fn refresh_retains_exact_model_and_replaces_its_metadata() {
    let installation = installation("2.1.0");
    let mut catalog = CapabilityCatalog::default();
    let refresh = catalog.replace(
        AgentCapability {
            installation: installation.clone(),
            models: vec![model("sonnet", "new metadata")],
        },
        Some("sonnet"),
    );

    assert_eq!(refresh.selection_invalidated, false);
    assert_eq!(
        refresh.selected_model.and_then(|model| model.description),
        Some("new metadata".to_string())
    );
    assert_eq!(
        catalog.get(&installation).map(|entry| entry.models.len()),
        Some(1)
    );
}

#[test]
fn refresh_invalidates_missing_model_without_fallback() {
    let mut catalog = CapabilityCatalog::default();
    let refresh = catalog.replace(
        AgentCapability {
            installation: installation("2.1.0"),
            models: vec![model("sonnet", "available")],
        },
        Some("opus"),
    );

    assert_eq!(refresh.selected_model, None);
    assert_eq!(refresh.selection_invalidated, true);
}

#[test]
fn cli_version_is_part_of_cache_identity() {
    let mut catalog = CapabilityCatalog::default();
    for version in ["2.1.0", "2.2.0"] {
        catalog.replace(
            AgentCapability {
                installation: installation(version),
                models: vec![model(version, "versioned")],
            },
            None,
        );
    }

    assert_eq!(catalog.all().count(), 2);
}
