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
