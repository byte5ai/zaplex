//! SSH Manager data layer — persisted server/folder tree + OS keychain credential storage +
//! command construction. UI and PTY injection logic are in `app/src/ssh_manager/` and `secret_injector`
//! modules; this module stays pure Rust with no warpui dependency and can run `cargo test` independently.

pub mod credential_lifecycle;
pub mod db;
pub mod repository;
pub mod secrets;
pub mod ssh_command;
pub mod ssh_config_parser;
pub mod sync_provider;
pub mod types;
pub mod validation;

pub use credential_lifecycle::{
    CredentialOperationError, DeleteHostExpectation, DeleteNodeExpectation, SaveServerRequest,
    clone_server_with_secrets, delete_node_and_secrets, delete_onekey_credential_and_secrets,
    prepare_delete_node, save_onekey_credential_with_secret, save_server_with_secrets,
};
pub use db::{set_database_path, with_conn};
pub use repository::{SshRepository, SshRepositoryError, SyncMetaRepository};
pub use secrets::{KeychainSecretStore, SecretKind, SshSecretStore, SshSecretStoreError};
#[cfg(unix)]
pub use ssh_command::persist_confirmed_host_key;
pub use ssh_command::{
    ConnectionTestResult, DefaultWorkspaceCommandFactory, HostKeyPreflight,
    InvalidMultiplexerTarget, MultiplexerAttachMode, UnknownHostKey, WorkspaceCommandFactory,
    build_multiplexer_ssh_command_line, build_ssh_args, build_ssh_command_line, preflight_host_key,
    preflight_host_key_with_factory, test_connection, test_connection_confirm_host_key,
    test_connection_with_factory,
};
pub use ssh_config_parser::{
    LoadOutcome, LoadResult, SshConfigCandidate, default_ssh_config_path, load_candidates,
    load_candidates_from, parse_ssh_config,
};
pub use sync_provider::{
    DbVersionStore, SshSyncData, SshSyncProvider, SyncNode, SyncOneKeyCredential, SyncServer,
};
pub use types::ConnectionStatus;
pub use types::{
    AuthType, NodeKind, OneKeyCredentialKind, ResolvedSshAuth, SessionResilience, SshNode,
    SshOneKeyCredential, SshServerInfo,
};
pub use validation::{
    EndpointUse, SshEndpointValidationError, ValidatedSshEndpoint, validate_ssh_endpoint,
};
