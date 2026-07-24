use diesel::{Connection, result::Error as DieselError, sqlite::SqliteConnection};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{
    AuthType, NodeKind, SecretKind, SshNode, SshOneKeyCredential, SshRepository,
    SshRepositoryError, SshSecretStore, SshSecretStoreError, SshServerInfo,
};

const SERVER_SECRET_KINDS: [SecretKind; 3] = [
    SecretKind::Password,
    SecretKind::Passphrase,
    SecretKind::RootPassword,
];

#[derive(Debug, Error)]
pub enum CredentialOperationError {
    #[error(transparent)]
    Repository(#[from] SshRepositoryError),
    #[error("keychain {operation} failed for {owner_id}/{kind:?}: {source}")]
    Secret {
        operation: &'static str,
        owner_id: String,
        kind: SecretKind,
        source: SshSecretStoreError,
    },
    #[error("{operation} failed: {failure}. Compensation failures: {compensation_failures:?}")]
    Compensation {
        operation: &'static str,
        failure: String,
        compensation_failures: Vec<String>,
    },
}

impl From<DieselError> for CredentialOperationError {
    fn from(error: DieselError) -> Self {
        Self::Repository(SshRepositoryError::Db(error))
    }
}

pub fn delete_node_and_secrets(
    conn: &mut SqliteConnection,
    store: &dyn SshSecretStore,
    node_id: &str,
) -> Result<(), CredentialOperationError> {
    let nodes = SshRepository::list_nodes(conn)?;
    if !nodes.iter().any(|node| node.id == node_id) {
        return Ok(());
    }
    let owner_ids = descendant_server_ids(&nodes, node_id);
    let snapshots = snapshot_secrets(store, &owner_ids)?;
    let mut deleted = Vec::new();
    for snapshot in snapshots.iter().filter(|snapshot| snapshot.value.is_some()) {
        deleted.push(snapshot.clone());
        if let Err(source) = store.delete(&snapshot.owner_id, snapshot.kind) {
            return Err(compensation_error(
                "delete SSH node",
                format!(
                    "keychain delete failed for {}/{:?}: {source}",
                    snapshot.owner_id, snapshot.kind
                ),
                restore_secrets(store, &deleted),
            ));
        }
    }
    if let Err(error) = SshRepository::delete_node(conn, node_id) {
        return Err(compensation_error(
            "delete SSH node",
            error.to_string(),
            restore_secrets(store, &deleted),
        ));
    }
    Ok(())
}

pub fn clone_server_with_secrets(
    conn: &mut SqliteConnection,
    store: &dyn SshSecretStore,
    source_id: &str,
    parent_id: Option<&str>,
    name: &str,
    cloned_info: &SshServerInfo,
) -> Result<SshNode, CredentialOperationError> {
    let source_secrets = snapshot_secrets(store, &[source_id.to_string()])?;
    let mut created_owner_id = None;
    let result = conn.transaction::<SshNode, CredentialOperationError, _>(|conn| {
        let node = SshRepository::create_server(conn, parent_id, name, cloned_info)?;
        created_owner_id = Some(node.id.clone());
        for snapshot in source_secrets
            .iter()
            .filter(|snapshot| snapshot.value.is_some())
        {
            let value = snapshot
                .value
                .as_ref()
                .expect("filtered to present secrets");
            store
                .set(&node.id, snapshot.kind, value)
                .map_err(|source| CredentialOperationError::Secret {
                    operation: "copy",
                    owner_id: node.id.clone(),
                    kind: snapshot.kind,
                    source,
                })?;
        }
        Ok(node)
    });
    match result {
        Ok(node) => Ok(node),
        Err(error) => {
            let compensation = created_owner_id
                .as_deref()
                .map(|owner_id| delete_secrets(store, owner_id, &SERVER_SECRET_KINDS))
                .unwrap_or_default();
            Err(compensation_error(
                "clone SSH server",
                error.to_string(),
                compensation,
            ))
        }
    }
}

pub struct SaveServerRequest<'a> {
    pub name: &'a str,
    pub server: &'a SshServerInfo,
    pub move_to_parent: bool,
    pub parent_id: Option<&'a str>,
    pub password_or_passphrase: Option<&'a str>,
    pub root_password: Option<&'a str>,
}

pub fn save_server_with_secrets(
    conn: &mut SqliteConnection,
    store: &dyn SshSecretStore,
    request: SaveServerRequest<'_>,
) -> Result<(), CredentialOperationError> {
    let mut changes = Vec::new();
    let previous_server = SshRepository::get_server(conn, &request.server.node_id)?
        .ok_or_else(|| SshRepositoryError::NotFound(request.server.node_id.clone()))?;
    if previous_server.auth_type != request.server.auth_type {
        let obsolete_kinds: &[SecretKind] = match request.server.auth_type {
            AuthType::Password => &[SecretKind::Passphrase],
            AuthType::Key => &[SecretKind::Password],
            AuthType::OneKey => &[SecretKind::Password, SecretKind::Passphrase],
        };
        for kind in obsolete_kinds {
            changes.push(delete_secret_change(store, &request.server.node_id, *kind)?);
        }
    }
    if let Some(secret) = request
        .password_or_passphrase
        .filter(|secret| !secret.is_empty())
    {
        let kind = match request.server.auth_type {
            AuthType::Password => Some(SecretKind::Password),
            AuthType::Key => Some(SecretKind::Passphrase),
            AuthType::OneKey => None,
        };
        if let Some(kind) = kind {
            changes.push(secret_change(store, &request.server.node_id, kind, secret)?);
        }
    }
    if let Some(secret) = request.root_password.filter(|secret| !secret.is_empty()) {
        changes.push(secret_change(
            store,
            &request.server.node_id,
            SecretKind::RootPassword,
            secret,
        )?);
    }

    let mut applied = Vec::new();
    for change in &changes {
        applied.push(change.snapshot.clone());
        let result = match &change.value {
            Some(value) => store.set(&change.snapshot.owner_id, change.snapshot.kind, value),
            None => store.delete(&change.snapshot.owner_id, change.snapshot.kind),
        };
        if let Err(error) = result {
            return Err(compensation_error(
                "save SSH server",
                format!(
                    "keychain mutation failed for {}/{:?}: {error}",
                    change.snapshot.owner_id, change.snapshot.kind
                ),
                restore_secrets(store, &applied),
            ));
        }
    }

    let db_result = conn.transaction::<(), SshRepositoryError, _>(|conn| {
        SshRepository::rename_node(conn, &request.server.node_id, request.name)?;
        SshRepository::update_server(conn, request.server)?;
        if request.move_to_parent {
            SshRepository::move_node_to_end(conn, &request.server.node_id, request.parent_id)?;
        }
        Ok(())
    });
    if let Err(error) = db_result {
        return Err(compensation_error(
            "save SSH server",
            error.to_string(),
            restore_secrets(store, &applied),
        ));
    }
    Ok(())
}

pub fn delete_onekey_credential_and_secrets(
    conn: &mut SqliteConnection,
    store: &dyn SshSecretStore,
    credential_id: &str,
) -> Result<(), CredentialOperationError> {
    if SshRepository::get_onekey_credential(conn, credential_id)?.is_none() {
        return Ok(());
    }
    let reference_count = SshRepository::onekey_credential_reference_count(conn, credential_id)?;
    if reference_count > 0 {
        return Err(SshRepositoryError::CredentialInUse {
            credential_id: credential_id.to_string(),
            reference_count,
        }
        .into());
    }
    let snapshots = [SecretKind::OneKeyPassword, SecretKind::Passphrase]
        .into_iter()
        .map(|kind| {
            let value = store.get(credential_id, kind).map_err(|source| {
                CredentialOperationError::Secret {
                    operation: "read before delete",
                    owner_id: credential_id.to_string(),
                    kind,
                    source,
                }
            })?;
            Ok(SecretSnapshot {
                owner_id: credential_id.to_string(),
                kind,
                value,
            })
        })
        .collect::<Result<Vec<_>, CredentialOperationError>>()?;
    let mut deleted = Vec::new();
    for snapshot in snapshots.iter().filter(|snapshot| snapshot.value.is_some()) {
        deleted.push(snapshot.clone());
        if let Err(error) = store.delete(credential_id, snapshot.kind) {
            return Err(compensation_error(
                "delete OneKey credential",
                format!("keychain delete failed for {:?}: {error}", snapshot.kind),
                restore_secrets(store, &deleted),
            ));
        }
    }
    if let Err(error) = SshRepository::delete_onekey_credential(conn, credential_id) {
        return Err(compensation_error(
            "delete OneKey credential",
            error.to_string(),
            restore_secrets(store, &deleted),
        ));
    }
    Ok(())
}

pub fn save_onekey_credential_with_secret(
    conn: &mut SqliteConnection,
    store: &dyn SshSecretStore,
    existing: Option<&SshOneKeyCredential>,
    credential: &SshOneKeyCredential,
    secret: Option<&str>,
) -> Result<SshOneKeyCredential, CredentialOperationError> {
    let secret = secret.filter(|secret| !secret.is_empty());
    let secret_kind = match credential.kind {
        crate::OneKeyCredentialKind::Password => SecretKind::OneKeyPassword,
        crate::OneKeyCredentialKind::Key => SecretKind::Passphrase,
    };
    let previous = if let Some(existing) = existing {
        [SecretKind::OneKeyPassword, SecretKind::Passphrase]
            .into_iter()
            .map(|kind| {
                let value = store.get(&existing.id, kind).map_err(|source| {
                    CredentialOperationError::Secret {
                        operation: "read before write",
                        owner_id: existing.id.clone(),
                        kind,
                        source,
                    }
                })?;
                Ok(SecretSnapshot {
                    owner_id: existing.id.clone(),
                    kind,
                    value,
                })
            })
            .collect::<Result<Vec<_>, CredentialOperationError>>()?
    } else {
        Vec::new()
    };
    let mut saved_owner_id = None;
    let result = conn.transaction::<SshOneKeyCredential, CredentialOperationError, _>(|conn| {
        let saved = match existing {
            Some(_) => {
                SshRepository::update_onekey_credential(conn, credential)?;
                credential.clone()
            }
            None => SshRepository::create_onekey_credential(
                conn,
                &credential.label,
                &credential.username,
                credential.kind,
                credential.key_path.as_deref(),
            )?,
        };
        saved_owner_id = Some(saved.id.clone());
        if let Some(secret) = secret {
            store
                .set(&saved.id, secret_kind, secret)
                .map_err(|source| CredentialOperationError::Secret {
                    operation: "write",
                    owner_id: saved.id.clone(),
                    kind: secret_kind,
                    source,
                })?;
        }
        if existing.is_some() {
            let obsolete_kind = match secret_kind {
                SecretKind::OneKeyPassword => SecretKind::Passphrase,
                SecretKind::Passphrase => SecretKind::OneKeyPassword,
                SecretKind::Password | SecretKind::RootPassword => {
                    unreachable!("OneKey credentials only use password or passphrase slots")
                }
            };
            store.delete(&saved.id, obsolete_kind).map_err(|source| {
                CredentialOperationError::Secret {
                    operation: "delete obsolete",
                    owner_id: saved.id.clone(),
                    kind: obsolete_kind,
                    source,
                }
            })?;
        }
        Ok(saved)
    });
    match result {
        Ok(saved) => Ok(saved),
        Err(error) => {
            let compensation = if previous.is_empty() {
                saved_owner_id
                    .as_deref()
                    .map(|owner_id| {
                        delete_secrets(
                            store,
                            owner_id,
                            &[SecretKind::OneKeyPassword, SecretKind::Passphrase],
                        )
                    })
                    .unwrap_or_default()
            } else {
                restore_secrets(store, &previous)
            };
            Err(compensation_error(
                "save OneKey credential",
                error.to_string(),
                compensation,
            ))
        }
    }
}

struct SecretChange {
    snapshot: SecretSnapshot,
    value: Option<Zeroizing<String>>,
}

fn secret_change(
    store: &dyn SshSecretStore,
    owner_id: &str,
    kind: SecretKind,
    value: &str,
) -> Result<SecretChange, CredentialOperationError> {
    let previous =
        store
            .get(owner_id, kind)
            .map_err(|source| CredentialOperationError::Secret {
                operation: "read before write",
                owner_id: owner_id.to_string(),
                kind,
                source,
            })?;
    Ok(SecretChange {
        snapshot: SecretSnapshot {
            owner_id: owner_id.to_string(),
            kind,
            value: previous,
        },
        value: Some(Zeroizing::new(value.to_string())),
    })
}

fn delete_secret_change(
    store: &dyn SshSecretStore,
    owner_id: &str,
    kind: SecretKind,
) -> Result<SecretChange, CredentialOperationError> {
    let previous =
        store
            .get(owner_id, kind)
            .map_err(|source| CredentialOperationError::Secret {
                operation: "read before delete",
                owner_id: owner_id.to_string(),
                kind,
                source,
            })?;
    Ok(SecretChange {
        snapshot: SecretSnapshot {
            owner_id: owner_id.to_string(),
            kind,
            value: previous,
        },
        value: None,
    })
}

#[derive(Clone)]
struct SecretSnapshot {
    owner_id: String,
    kind: SecretKind,
    value: Option<Zeroizing<String>>,
}

fn descendant_server_ids(nodes: &[SshNode], root_id: &str) -> Vec<String> {
    let mut descendants = vec![root_id.to_string()];
    let mut index = 0;
    while index < descendants.len() {
        let parent_id = descendants[index].clone();
        for child in nodes
            .iter()
            .filter(|node| node.parent_id.as_deref() == Some(parent_id.as_str()))
        {
            if !descendants.contains(&child.id) {
                descendants.push(child.id.clone());
            }
        }
        index += 1;
    }
    nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Server && descendants.contains(&node.id))
        .map(|node| node.id.clone())
        .collect()
}

fn snapshot_secrets(
    store: &dyn SshSecretStore,
    owner_ids: &[String],
) -> Result<Vec<SecretSnapshot>, CredentialOperationError> {
    let mut snapshots = Vec::new();
    for owner_id in owner_ids {
        for kind in SERVER_SECRET_KINDS {
            let value =
                store
                    .get(owner_id, kind)
                    .map_err(|source| CredentialOperationError::Secret {
                        operation: "read",
                        owner_id: owner_id.clone(),
                        kind,
                        source,
                    })?;
            snapshots.push(SecretSnapshot {
                owner_id: owner_id.clone(),
                kind,
                value,
            });
        }
    }
    Ok(snapshots)
}

fn restore_secrets(store: &dyn SshSecretStore, snapshots: &[SecretSnapshot]) -> Vec<String> {
    let mut failures = Vec::new();
    for snapshot in snapshots {
        let result = match &snapshot.value {
            Some(value) => store.set(&snapshot.owner_id, snapshot.kind, value),
            None => store.delete(&snapshot.owner_id, snapshot.kind),
        };
        if let Err(error) = result {
            failures.push(format!(
                "restore {}/{:?} failed: {error}",
                snapshot.owner_id, snapshot.kind
            ));
        }
    }
    failures
}

fn delete_secrets(store: &dyn SshSecretStore, owner_id: &str, kinds: &[SecretKind]) -> Vec<String> {
    let mut failures = Vec::new();
    for kind in kinds {
        if let Err(error) = store.delete(owner_id, *kind) {
            failures.push(format!("delete {owner_id}/{kind:?} failed: {error}"));
        }
    }
    failures
}

fn compensation_error(
    operation: &'static str,
    failure: String,
    compensation_failures: Vec<String>,
) -> CredentialOperationError {
    CredentialOperationError::Compensation {
        operation,
        failure,
        compensation_failures,
    }
}

#[cfg(test)]
#[path = "credential_lifecycle_tests.rs"]
mod tests;
