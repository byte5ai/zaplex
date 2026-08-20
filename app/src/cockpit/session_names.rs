//! Persistent authoritative Cockpit names for exact provider sessions.
//!
//! Providers that cannot rename an existing conversation natively may declare
//! this versioned overlay as their Cockpit naming source. The key contains the
//! complete stable provider/account/host route and excludes volatile PIDs,
//! terminal ids, and host labels. Corrupt input is protected from overwrite.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zaplex_cockpit::{CockpitSnapshot, FleetTree, Provider, SessionSnapshot};

use crate::cockpit::session_lifecycle::{SessionAccountRoute, SessionHostRoute, SessionRoute};

const NAME_STORE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredHost {
    Local,
    Remote { host_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredAccount {
    Local {
        config_dir: Option<PathBuf>,
        account_email: Option<String>,
    },
    Remote {
        account_id: String,
        account_email: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SessionNameKey {
    provider: Provider,
    session_id: String,
    host: StoredHost,
    account: StoredAccount,
}

impl From<&SessionRoute> for SessionNameKey {
    fn from(route: &SessionRoute) -> Self {
        let host = match &route.host {
            SessionHostRoute::Local => StoredHost::Local,
            SessionHostRoute::Remote { host_id, .. } => StoredHost::Remote {
                host_id: host_id.clone(),
            },
        };
        let account = match &route.account {
            SessionAccountRoute::Local {
                config_dir,
                account_email,
            } => StoredAccount::Local {
                config_dir: config_dir.clone(),
                account_email: account_email.clone(),
            },
            SessionAccountRoute::Remote {
                account_id,
                account_email,
            } => StoredAccount::Remote {
                account_id: account_id.clone(),
                account_email: account_email.clone(),
            },
        };
        Self {
            provider: route.provider,
            session_id: route.session_id.clone(),
            host,
            account,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SessionNameEntry {
    key: SessionNameKey,
    name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedSessionNames {
    version: u32,
    entries: Vec<SessionNameEntry>,
}

impl Default for PersistedSessionNames {
    fn default() -> Self {
        Self {
            version: NAME_STORE_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameFileState {
    Missing,
    Loaded,
    Protected,
}

pub(crate) struct SessionNameStore {
    persisted: PersistedSessionNames,
    state: NameFileState,
    #[cfg(not(target_family = "wasm"))]
    file: Option<PathBuf>,
}

impl SessionNameStore {
    pub(crate) fn load() -> Self {
        #[cfg(not(target_family = "wasm"))]
        {
            return Self::load_from(session_names_file());
        }
        #[cfg(target_family = "wasm")]
        Self {
            persisted: PersistedSessionNames::default(),
            state: NameFileState::Missing,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from(file: PathBuf) -> Self {
        let (persisted, state) = match std::fs::read_to_string(&file) {
            Ok(contents) => match serde_json::from_str::<PersistedSessionNames>(&contents) {
                Ok(persisted) if persisted.version == NAME_STORE_VERSION => {
                    (persisted, NameFileState::Loaded)
                }
                Ok(_) | Err(_) => (PersistedSessionNames::default(), NameFileState::Protected),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (PersistedSessionNames::default(), NameFileState::Missing)
            }
            Err(_) => (PersistedSessionNames::default(), NameFileState::Protected),
        };
        Self {
            persisted,
            state,
            file: Some(file),
        }
    }

    /// Providers explicitly covered by this authoritative naming store.
    pub(crate) fn supports(provider: Provider) -> bool {
        matches!(provider, Provider::Claude | Provider::Codex)
    }

    pub(crate) fn name(&self, route: &SessionRoute) -> Option<&str> {
        let key = SessionNameKey::from(route);
        self.persisted
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.name.as_str())
    }

    fn apply_to_session(
        &self,
        session: &mut SessionSnapshot,
        is_local: bool,
        host_id: Option<&str>,
        node_id: Option<&str>,
    ) {
        let Ok(route) = SessionRoute::from_snapshot(session, is_local, host_id, node_id) else {
            return;
        };
        if let Some(name) = self.name(&route) {
            session.name = name.to_string();
        }
    }

    pub(crate) fn apply_to_local_snapshot(&self, snapshot: &mut CockpitSnapshot) {
        for account in &mut snapshot.accounts {
            for session in account
                .sessions
                .iter_mut()
                .chain(account.idle_sessions.iter_mut())
            {
                self.apply_to_session(session, true, None, None);
            }
        }
    }

    pub(crate) fn apply_to_inventory(&self, inventory: &mut FleetTree) {
        for host in &mut inventory.hosts {
            for session in host
                .projects
                .iter_mut()
                .flat_map(|project| project.sessions.iter_mut())
            {
                self.apply_to_session(
                    session,
                    host.is_local,
                    host.host_id.as_deref(),
                    host.registry_node_id.as_deref(),
                );
            }
        }
    }

    /// Apply an already validated name to exactly one route and persist it
    /// before reporting success.
    pub(crate) fn set_name(&mut self, route: &SessionRoute, name: String) -> anyhow::Result<()> {
        anyhow::ensure!(
            Self::supports(route.provider),
            "provider naming is unsupported"
        );
        anyhow::ensure!(!name.trim().is_empty(), "session name must not be empty");
        let previous = self.persisted.clone();
        let key = SessionNameKey::from(route);
        if let Some(existing) = self
            .persisted
            .entries
            .iter_mut()
            .find(|entry| entry.key == key)
        {
            existing.name = name;
        } else {
            self.persisted.entries.push(SessionNameEntry { key, name });
        }
        if let Err(error) = self.save() {
            self.persisted = previous;
            return Err(error);
        }
        self.state = NameFileState::Loaded;
        Ok(())
    }

    #[cfg(not(target_family = "wasm"))]
    fn save(&self) -> anyhow::Result<()> {
        use anyhow::{bail, Context as _};
        use std::io::Write as _;

        if self.state == NameFileState::Protected {
            bail!("refusing to overwrite an unreadable session-name store");
        }
        let path = self
            .file
            .as_deref()
            .context("session-name store has no path")?;
        let parent = path.parent().context("session-name store has no parent")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec_pretty(&self.persisted)?)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(path)
            .map_err(|error| anyhow::Error::from(error.error))?;
        Ok(())
    }

    #[cfg(target_family = "wasm")]
    fn save(&self) -> anyhow::Result<()> {
        anyhow::bail!("session-name persistence is unavailable on wasm")
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_names_file() -> PathBuf {
    warp_core::paths::data_dir().join("cockpit-session-names.json")
}

/// Persist a validated name before the UI reports success. A subsequent model
/// rescan applies the same store to local account details and the fleet tree.
pub(crate) fn persist_session_name(route: &SessionRoute, name: String) -> anyhow::Result<()> {
    SessionNameStore::load().set_name(route, name)
}

#[cfg(test)]
#[path = "session_names_tests.rs"]
mod tests;
