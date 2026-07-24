//! SSH manager UI (left Tool Panel). Currently skeleton, content pending Commit 2b implementation:
//! tree-style folder/server list + right-side detail form.
//!
//! Data layer in separate crate `warp_ssh_manager` (`crates/warp_ssh_manager/`).

pub mod candidates;
pub mod notifier;
pub mod onekey;
pub mod panel;
pub mod password_prompt;
pub mod secret_injector;
pub mod server_view;
pub mod shell_prompt;
pub mod startup_command_injector;
pub mod su_password_injector;

// `CandidatesViewModel` currently only referenced by `panel.rs`; `CandidateRow` just intermediate representation
// for panel internal layout, no need to export. Add re-export when needed by external consumers.
#[allow(unused_imports)]
pub use candidates::CandidatesViewModel;
pub use notifier::{SshTreeChangedEvent, SshTreeChangedNotifier};
pub use panel::SshManagerPanel;
// Re-exports for downstream UI consumers (Commit 2b).
#[allow(unused_imports)]
pub use panel::{SshManagerPanelAction, SshManagerPanelEvent};

fn credential_operation_message(error: &anyhow::Error) -> String {
    use warp_ssh_manager::CredentialOperationError;

    let Some(error) = error.downcast_ref::<CredentialOperationError>() else {
        return crate::t!("common-error");
    };
    match error {
        CredentialOperationError::Repository(_) => crate::t!("common-error"),
        CredentialOperationError::Secret { source, .. } => {
            crate::t!("ssh-keychain-error", err = source.to_string())
        }
        CredentialOperationError::Compensation {
            compensation_failures,
            ..
        } => {
            let marker = if compensation_failures.is_empty() {
                "↩ ✓"
            } else {
                "↩ ✕"
            };
            let keychain = crate::t!("ssh-keychain-error", err = marker);
            let failed = crate::t!("common-failed");
            if compensation_failures.is_empty() {
                format!("{failed} · DB ↔ {keychain} · {}", crate::t!("common-retry"))
            } else {
                format!("{failed} · DB ↔ {keychain}")
            }
        }
    }
}

fn endpoint_validation_message(error: warp_ssh_manager::SshEndpointValidationError) -> String {
    use warp_ssh_manager::SshEndpointValidationError;

    match error {
        SshEndpointValidationError::EmptyHost | SshEndpointValidationError::InvalidHost => {
            crate::t!("workspace-left-panel-ssh-manager-error-host-required")
        }
        SshEndpointValidationError::InvalidPort => {
            crate::t!("workspace-left-panel-ssh-manager-error-port-invalid")
        }
    }
}
