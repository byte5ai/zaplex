//! Diesel CRUD over `ssh_nodes` + `ssh_servers`. All return types are plain data structures
//! from `crate::types`, keeping ORM details confined to crate boundary.
//!
//! All write operations default sort_order to the current maximum + 1 within the same parent;
//! UI can append directly when order doesn't matter. Callers of move_node are responsible for passing the new sort_order.

use chrono::Utc;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sqlite::SqliteConnection;
use thiserror::Error;
use uuid::Uuid;

use crate::secrets::SecretKind;
use crate::types::{
    AuthType, NodeKind, OneKeyCredentialKind, ResolvedSshAuth, SessionResilience, SshNode,
    SshOneKeyCredential, SshServerInfo,
};
use persistence::model::{
    NewSshNode, NewSshOneKeyCredential, NewSshServer, NewSyncMeta, SshNodeRow,
    SshOneKeyCredentialRow, SshServerRow, SyncMetaRow,
};
use persistence::schema::{ssh_nodes, ssh_onekey_credentials, ssh_servers, sync_meta};

#[derive(Debug, Error)]
pub enum SshRepositoryError {
    #[error("database error: {0}")]
    Db(#[from] DieselError),
    #[error("node not found: {0}")]
    NotFound(String),
    #[error("OneKey credential {credential_id} is still referenced by {reference_count} server(s)")]
    CredentialInUse {
        credential_id: String,
        reference_count: i64,
    },
    #[error("invalid SSH sync version: {0}")]
    InvalidSyncVersion(String),
    #[error("invalid value in db column `{column}`: {value}")]
    InvalidEnum { column: &'static str, value: String },
}

/// Data access layer. Each method accepts `&mut SqliteConnection`, giving callers ownership
/// so transaction boundaries are determined by callers (typical UI model layer opens a new transaction per operation).
pub struct SshRepository;

impl SshRepository {
    /// List all nodes (folder + server) without details. Callers arrange them into a tree.
    pub fn list_nodes(conn: &mut SqliteConnection) -> Result<Vec<SshNode>, SshRepositoryError> {
        let rows: Vec<SshNodeRow> = ssh_nodes::table
            .order((ssh_nodes::parent_id.asc(), ssh_nodes::sort_order.asc()))
            .load(conn)?;
        rows.into_iter().map(node_from_row).collect()
    }

    pub fn get_server(
        conn: &mut SqliteConnection,
        node_id: &str,
    ) -> Result<Option<SshServerInfo>, SshRepositoryError> {
        let row: Option<SshServerRow> = ssh_servers::table.find(node_id).first(conn).optional()?;
        row.map(server_from_row).transpose()
    }

    pub fn create_folder(
        conn: &mut SqliteConnection,
        parent_id: Option<&str>,
        name: &str,
    ) -> Result<SshNode, SshRepositoryError> {
        let id = new_uuid();
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let sort = next_sort_order(conn, parent_id)?;
            diesel::insert_into(ssh_nodes::table)
                .values(NewSshNode {
                    id: &id,
                    parent_id,
                    kind: NodeKind::Folder.as_db_str(),
                    name,
                    sort_order: sort,
                })
                .execute(conn)?;
            Self::increment_sync_version(conn)?;
            Self::get_node(conn, &id)
        })
    }

    pub fn create_server(
        conn: &mut SqliteConnection,
        parent_id: Option<&str>,
        name: &str,
        info: &SshServerInfo,
    ) -> Result<SshNode, SshRepositoryError> {
        let id = new_uuid();
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let sort = next_sort_order(conn, parent_id)?;
            diesel::insert_into(ssh_nodes::table)
                .values(NewSshNode {
                    id: &id,
                    parent_id,
                    kind: NodeKind::Server.as_db_str(),
                    name,
                    sort_order: sort,
                })
                .execute(conn)?;
            diesel::insert_into(ssh_servers::table)
                .values(NewSshServer {
                    node_id: &id,
                    host: &info.host,
                    port: info.port as i32,
                    username: &info.username,
                    auth_type: info.auth_type.as_db_str(),
                    key_path: info.key_path.as_deref(),
                    startup_command: info.startup_command.as_deref(),
                    notes: info.notes.as_deref(),
                    credential_id: info.credential_id.as_deref(),
                    session_resilience: info.session_resilience.as_db_str(),
                    ring_ceiling_mb: info.ring_ceiling_mb as i32,
                })
                .execute(conn)?;
            Self::increment_sync_version(conn)?;
            Self::get_node(conn, &id)
        })
    }

    pub fn rename_node(
        conn: &mut SqliteConnection,
        node_id: &str,
        new_name: &str,
    ) -> Result<(), SshRepositoryError> {
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let n = diesel::update(ssh_nodes::table.find(node_id))
                .set((
                    ssh_nodes::name.eq(new_name),
                    ssh_nodes::updated_at.eq(Utc::now().naive_utc()),
                ))
                .execute(conn)?;
            if n == 0 {
                return Err(SshRepositoryError::NotFound(node_id.to_string()));
            }
            Self::increment_sync_version(conn)?;
            Ok(())
        })
    }

    pub fn update_server(
        conn: &mut SqliteConnection,
        info: &SshServerInfo,
    ) -> Result<(), SshRepositoryError> {
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let n = diesel::update(ssh_servers::table.find(&info.node_id))
                .set((
                    ssh_servers::host.eq(&info.host),
                    ssh_servers::port.eq(info.port as i32),
                    ssh_servers::username.eq(&info.username),
                    ssh_servers::auth_type.eq(info.auth_type.as_db_str()),
                    ssh_servers::key_path.eq(info.key_path.as_deref()),
                    ssh_servers::startup_command.eq(info.startup_command.as_deref()),
                    ssh_servers::notes.eq(info.notes.as_deref()),
                    ssh_servers::credential_id.eq(info.credential_id.as_deref()),
                    ssh_servers::session_resilience.eq(info.session_resilience.as_db_str()),
                    ssh_servers::ring_ceiling_mb.eq(info.ring_ceiling_mb as i32),
                ))
                .execute(conn)?;
            if n == 0 {
                return Err(SshRepositoryError::NotFound(info.node_id.clone()));
            }
            diesel::update(ssh_nodes::table.find(&info.node_id))
                .set(ssh_nodes::updated_at.eq(Utc::now().naive_utc()))
                .execute(conn)?;
            Self::increment_sync_version(conn)?;
            Ok(())
        })
    }

    /// Delete node; ON DELETE CASCADE syncs deletion of children + ssh_servers rows.
    /// Callers are responsible for clearing the corresponding secret from keychain.
    pub fn delete_node(
        conn: &mut SqliteConnection,
        node_id: &str,
    ) -> Result<(), SshRepositoryError> {
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let n = diesel::delete(ssh_nodes::table.find(node_id)).execute(conn)?;
            if n == 0 {
                return Err(SshRepositoryError::NotFound(node_id.to_string()));
            }
            Self::increment_sync_version(conn)?;
            Ok(())
        })
    }

    /// Support changing parent and order simultaneously. new_parent_id=None means move to root.
    pub fn move_node(
        conn: &mut SqliteConnection,
        node_id: &str,
        new_parent_id: Option<&str>,
        new_sort_order: i32,
    ) -> Result<(), SshRepositoryError> {
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let n = diesel::update(ssh_nodes::table.find(node_id))
                .set((
                    ssh_nodes::parent_id.eq(new_parent_id),
                    ssh_nodes::sort_order.eq(new_sort_order),
                    ssh_nodes::updated_at.eq(Utc::now().naive_utc()),
                ))
                .execute(conn)?;
            if n == 0 {
                return Err(SshRepositoryError::NotFound(node_id.to_string()));
            }
            Self::increment_sync_version(conn)?;
            Ok(())
        })
    }

    /// Move node to the end of target parent (new_parent_id=None means move to root).
    /// Auto-calculate sort_order as current max + 1 under target parent, excluding self to avoid gaps when moving within same parent.
    pub fn move_node_to_end(
        conn: &mut SqliteConnection,
        node_id: &str,
        new_parent_id: Option<&str>,
    ) -> Result<(), SshRepositoryError> {
        let sort = next_sort_order_excluding(conn, new_parent_id, node_id)?;
        Self::move_node(conn, node_id, new_parent_id, sort)
    }

    pub fn touch_last_connected(
        conn: &mut SqliteConnection,
        node_id: &str,
    ) -> Result<(), SshRepositoryError> {
        diesel::update(ssh_servers::table.find(node_id))
            .set(ssh_servers::last_connected_at.eq(Some(Utc::now().naive_utc())))
            .execute(conn)?;
        Ok(())
    }

    pub fn list_onekey_credentials(
        conn: &mut SqliteConnection,
    ) -> Result<Vec<SshOneKeyCredential>, SshRepositoryError> {
        let rows: Vec<SshOneKeyCredentialRow> = ssh_onekey_credentials::table
            .order(ssh_onekey_credentials::label.asc())
            .load(conn)?;
        rows.into_iter().map(onekey_from_row).collect()
    }

    pub fn get_onekey_credential(
        conn: &mut SqliteConnection,
        credential_id: &str,
    ) -> Result<Option<SshOneKeyCredential>, SshRepositoryError> {
        let row: Option<SshOneKeyCredentialRow> = ssh_onekey_credentials::table
            .find(credential_id)
            .first(conn)
            .optional()?;
        row.map(onekey_from_row).transpose()
    }

    pub fn create_onekey_credential(
        conn: &mut SqliteConnection,
        label: &str,
        username: &str,
        kind: OneKeyCredentialKind,
        key_path: Option<&str>,
    ) -> Result<SshOneKeyCredential, SshRepositoryError> {
        let id = new_uuid();
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            diesel::insert_into(ssh_onekey_credentials::table)
                .values(NewSshOneKeyCredential {
                    id: &id,
                    label,
                    username,
                    kind: kind.as_db_str(),
                    key_path,
                })
                .execute(conn)?;
            Self::increment_sync_version(conn)?;
            Self::get_onekey_credential(conn, &id)?
                .ok_or_else(|| SshRepositoryError::NotFound(id.clone()))
        })
    }

    pub fn update_onekey_credential(
        conn: &mut SqliteConnection,
        credential: &SshOneKeyCredential,
    ) -> Result<(), SshRepositoryError> {
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let n = diesel::update(ssh_onekey_credentials::table.find(&credential.id))
                .set((
                    ssh_onekey_credentials::label.eq(&credential.label),
                    ssh_onekey_credentials::username.eq(&credential.username),
                    ssh_onekey_credentials::kind.eq(credential.kind.as_db_str()),
                    ssh_onekey_credentials::key_path.eq(credential.key_path.as_deref()),
                    ssh_onekey_credentials::updated_at.eq(Utc::now().naive_utc()),
                ))
                .execute(conn)?;
            if n == 0 {
                return Err(SshRepositoryError::NotFound(credential.id.clone()));
            }
            Self::increment_sync_version(conn)?;
            Ok(())
        })
    }

    pub fn delete_onekey_credential(
        conn: &mut SqliteConnection,
        credential_id: &str,
    ) -> Result<(), SshRepositoryError> {
        conn.transaction::<_, SshRepositoryError, _>(|conn| {
            let reference_count = Self::onekey_credential_reference_count(conn, credential_id)?;
            if reference_count > 0 {
                return Err(SshRepositoryError::CredentialInUse {
                    credential_id: credential_id.to_string(),
                    reference_count,
                });
            }
            let n =
                diesel::delete(ssh_onekey_credentials::table.find(credential_id)).execute(conn)?;
            if n == 0 {
                return Err(SshRepositoryError::NotFound(credential_id.to_string()));
            }
            Self::increment_sync_version(conn)?;
            Ok(())
        })
    }

    pub fn onekey_credential_reference_count(
        conn: &mut SqliteConnection,
        credential_id: &str,
    ) -> Result<i64, SshRepositoryError> {
        Ok(ssh_servers::table
            .filter(ssh_servers::credential_id.eq(credential_id))
            .count()
            .get_result(conn)?)
    }

    pub fn resolve_server_auth(
        conn: &mut SqliteConnection,
        server: &SshServerInfo,
    ) -> Result<ResolvedSshAuth, SshRepositoryError> {
        match server.auth_type {
            AuthType::Password => Ok(ResolvedSshAuth {
                username: server.username.clone(),
                auth_type: AuthType::Password,
                key_path: None,
                secret_lookup_id: server.node_id.clone(),
                secret_kind: SecretKind::Password,
            }),
            AuthType::Key => Ok(ResolvedSshAuth {
                username: server.username.clone(),
                auth_type: AuthType::Key,
                key_path: server.key_path.clone(),
                secret_lookup_id: server.node_id.clone(),
                secret_kind: SecretKind::Passphrase,
            }),
            AuthType::OneKey => {
                let Some(credential_id) = server.credential_id.as_deref() else {
                    return Err(SshRepositoryError::NotFound(
                        "onekey credential".to_string(),
                    ));
                };
                let Some(credential) = Self::get_onekey_credential(conn, credential_id)? else {
                    return Err(SshRepositoryError::NotFound(credential_id.to_string()));
                };
                Ok(ResolvedSshAuth {
                    username: credential.username,
                    auth_type: match credential.kind {
                        OneKeyCredentialKind::Password => AuthType::Password,
                        OneKeyCredentialKind::Key => AuthType::Key,
                    },
                    key_path: credential.key_path,
                    secret_lookup_id: credential_id.to_string(),
                    secret_kind: match credential.kind {
                        OneKeyCredentialKind::Password => SecretKind::OneKeyPassword,
                        OneKeyCredentialKind::Key => SecretKind::Passphrase,
                    },
                })
            }
        }
    }

    /// Update collapsed state for a single folder. Server nodes can also be set (though UI doesn't use it)
    /// to simplify caller logic.
    pub fn set_collapsed(
        conn: &mut SqliteConnection,
        node_id: &str,
        collapsed: bool,
    ) -> Result<(), SshRepositoryError> {
        let n = diesel::update(ssh_nodes::table.find(node_id))
            .set((
                ssh_nodes::is_collapsed.eq(collapsed),
                ssh_nodes::updated_at.eq(Utc::now().naive_utc()),
            ))
            .execute(conn)?;
        if n == 0 {
            return Err(SshRepositoryError::NotFound(node_id.to_string()));
        }
        Ok(())
    }

    /// Increment sync version number.
    pub fn increment_sync_version(conn: &mut SqliteConnection) -> Result<i64, SshRepositoryError> {
        SyncMetaRepository::increment_sync_version(conn)
    }

    /// Set `is_collapsed` to the given value for all folder nodes in one operation.
    pub fn set_all_folders_collapsed(
        conn: &mut SqliteConnection,
        collapsed: bool,
    ) -> Result<(), SshRepositoryError> {
        diesel::update(ssh_nodes::table.filter(ssh_nodes::kind.eq(NodeKind::Folder.as_db_str())))
            .set((
                ssh_nodes::is_collapsed.eq(collapsed),
                ssh_nodes::updated_at.eq(Utc::now().naive_utc()),
            ))
            .execute(conn)?;
        Ok(())
    }

    fn get_node(conn: &mut SqliteConnection, node_id: &str) -> Result<SshNode, SshRepositoryError> {
        let row: SshNodeRow = ssh_nodes::table
            .find(node_id)
            .first(conn)
            .map_err(|e| match e {
                DieselError::NotFound => SshRepositoryError::NotFound(node_id.to_string()),
                other => SshRepositoryError::Db(other),
            })?;
        node_from_row(row)
    }
}

fn next_sort_order(
    conn: &mut SqliteConnection,
    parent_id: Option<&str>,
) -> Result<i32, SshRepositoryError> {
    let max: Option<i32> = match parent_id {
        Some(p) => ssh_nodes::table
            .filter(ssh_nodes::parent_id.eq(p))
            .select(diesel::dsl::max(ssh_nodes::sort_order))
            .first(conn)?,
        None => ssh_nodes::table
            .filter(ssh_nodes::parent_id.is_null())
            .select(diesel::dsl::max(ssh_nodes::sort_order))
            .first(conn)?,
    };
    Ok(max.unwrap_or(-1) + 1)
}

/// Calculate the next sort_order under target parent, excluding the specified node (to avoid gaps when moving within same parent).
fn next_sort_order_excluding(
    conn: &mut SqliteConnection,
    parent_id: Option<&str>,
    exclude_node_id: &str,
) -> Result<i32, SshRepositoryError> {
    let max: Option<i32> = match parent_id {
        Some(p) => ssh_nodes::table
            .filter(ssh_nodes::parent_id.eq(p))
            .filter(ssh_nodes::id.ne(exclude_node_id))
            .select(diesel::dsl::max(ssh_nodes::sort_order))
            .first(conn)?,
        None => ssh_nodes::table
            .filter(ssh_nodes::parent_id.is_null())
            .filter(ssh_nodes::id.ne(exclude_node_id))
            .select(diesel::dsl::max(ssh_nodes::sort_order))
            .first(conn)?,
    };
    Ok(max.unwrap_or(-1) + 1)
}

fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

fn node_from_row(r: SshNodeRow) -> Result<SshNode, SshRepositoryError> {
    let kind = NodeKind::parse(&r.kind).ok_or_else(|| SshRepositoryError::InvalidEnum {
        column: "ssh_nodes.kind",
        value: r.kind.clone(),
    })?;
    Ok(SshNode {
        id: r.id,
        parent_id: r.parent_id,
        kind,
        name: r.name,
        sort_order: r.sort_order,
        created_at: r.created_at,
        updated_at: r.updated_at,
        is_collapsed: r.is_collapsed,
    })
}

fn server_from_row(r: SshServerRow) -> Result<SshServerInfo, SshRepositoryError> {
    let auth = AuthType::parse(&r.auth_type).ok_or_else(|| SshRepositoryError::InvalidEnum {
        column: "ssh_servers.auth_type",
        value: r.auth_type.clone(),
    })?;
    Ok(SshServerInfo {
        node_id: r.node_id,
        host: r.host,
        port: r.port as u16,
        username: r.username,
        auth_type: auth,
        key_path: r.key_path,
        startup_command: r.startup_command,
        notes: r.notes,
        last_connected_at: r.last_connected_at,
        credential_id: r.credential_id,
        // Lenient on purpose: an unknown value (e.g. written by a newer client)
        // degrades to `Off` rather than making the whole server unloadable.
        // Explicit `Off` (not `unwrap_or_default`, whose default is now `PersistOnly`
        // for *new* hosts) so a corrupt stored value never silently upgrades a
        // saved host to persistent.
        session_resilience: SessionResilience::parse(&r.session_resilience)
            .unwrap_or(SessionResilience::Off),
        ring_ceiling_mb: r.ring_ceiling_mb.max(0) as u32,
    })
}

fn onekey_from_row(r: SshOneKeyCredentialRow) -> Result<SshOneKeyCredential, SshRepositoryError> {
    let kind =
        OneKeyCredentialKind::parse(&r.kind).ok_or_else(|| SshRepositoryError::InvalidEnum {
            column: "ssh_onekey_credentials.kind",
            value: r.kind.clone(),
        })?;
    Ok(SshOneKeyCredential {
        id: r.id,
        label: r.label,
        username: r.username,
        kind,
        key_path: r.key_path,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

/// Sync metadata repository managing sync_version and sync records in the sync_meta table.
pub struct SyncMetaRepository;

impl SyncMetaRepository {
    /// Get sync version number.
    pub fn get_sync_version(conn: &mut SqliteConnection) -> Result<i64, SshRepositoryError> {
        let row: Option<SyncMetaRow> = sync_meta::table
            .find("sync_version")
            .first(conn)
            .optional()?;
        match row {
            Some(row) => row
                .value
                .parse()
                .map_err(|_| SshRepositoryError::InvalidSyncVersion(row.value)),
            None => Ok(0),
        }
    }

    /// Increment sync version number and return the new value.
    pub fn increment_sync_version(conn: &mut SqliteConnection) -> Result<i64, SshRepositoryError> {
        let current = Self::get_sync_version(conn)?;
        let new_version = current
            .checked_add(1)
            .ok_or_else(|| SshRepositoryError::InvalidSyncVersion(current.to_string()))?;
        let val = new_version.to_string();
        diesel::replace_into(sync_meta::table)
            .values(NewSyncMeta {
                key: "sync_version",
                value: &val,
            })
            .execute(conn)?;
        Ok(new_version)
    }

    /// Set sync version number.
    pub fn set_sync_version(
        conn: &mut SqliteConnection,
        version: i64,
    ) -> Result<(), SshRepositoryError> {
        let val = version.to_string();
        diesel::replace_into(sync_meta::table)
            .values(NewSyncMeta {
                key: "sync_version",
                value: &val,
            })
            .execute(conn)?;
        Ok(())
    }

    /// Get last sync time.
    pub fn get_last_sync_time(conn: &mut SqliteConnection) -> Result<String, SshRepositoryError> {
        let row: Option<SyncMetaRow> = sync_meta::table
            .find("last_sync_time")
            .first(conn)
            .optional()?;
        Ok(row.map(|r| r.value).unwrap_or_default())
    }

    /// Get last sync platform.
    pub fn get_last_sync_platform(
        conn: &mut SqliteConnection,
    ) -> Result<String, SshRepositoryError> {
        let row: Option<SyncMetaRow> = sync_meta::table
            .find("last_sync_platform")
            .first(conn)
            .optional()?;
        Ok(row.map(|r| r.value).unwrap_or_default())
    }

    /// Update sync metadata.
    pub fn update_sync_meta(
        conn: &mut SqliteConnection,
        last_time: &str,
        last_platform: &str,
    ) -> Result<(), SshRepositoryError> {
        diesel::replace_into(sync_meta::table)
            .values(&[
                NewSyncMeta {
                    key: "last_sync_time",
                    value: last_time,
                },
                NewSyncMeta {
                    key: "last_sync_platform",
                    value: last_platform,
                },
            ])
            .execute(conn)?;
        Ok(())
    }
}

/// Test helper: run all SSH-related migrations in memory SQLite. Must add include_str! here when adding new migrations.
#[cfg(test)]
pub(crate) fn setup_in_memory() -> SqliteConnection {
    use diesel::connection::SimpleConnection;
    let mut conn = SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute("PRAGMA foreign_keys = ON;").unwrap();
    for up in [
        include_str!(
            "../../persistence/migrations/2026-05-04-120000_add_ssh_manager_tables/up.sql"
        ),
        include_str!(
            "../../persistence/migrations/2026-05-04-130000_add_ssh_nodes_is_collapsed/up.sql"
        ),
        include_str!(
            "../../persistence/migrations/2026-05-23-140000_add_startup_command_and_notes/up.sql"
        ),
        include_str!("../../persistence/migrations/2026-05-24-150000_add_sync_meta/up.sql"),
        include_str!(
            "../../persistence/migrations/2026-06-08-120000_add_ssh_onekey_credentials/up.sql"
        ),
        include_str!(
            "../../persistence/migrations/2026-06-09-160000_add_ssh_onekey_key_type/up.sql"
        ),
        include_str!(
            "../../persistence/migrations/2026-06-27-000000_add_session_resilience/up.sql"
        ),
        include_str!("../../persistence/migrations/2026-06-29-000000_add_ring_ceiling/up.sql"),
    ] {
        conn.batch_execute(up).unwrap();
    }
    conn
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
