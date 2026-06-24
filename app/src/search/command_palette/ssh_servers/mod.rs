//! Command palette data source: SSH servers (openWarp-specific).
//!
//! In Ctrl+Shift+P, users fuzzy-match by server name / host, select → emit
//! `WorkspaceAction::OpenSshTerminal` to open a new tab connection (via SecretInjector for automatic
//! password injection, completely equivalent to right-click "Connect" from SSH manager).

pub mod data_source;
pub mod search_item;

pub use data_source::SshServersDataSource;
pub use search_item::SshServerSearchItem;
