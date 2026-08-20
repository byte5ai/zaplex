//! Persisted, host-scoped launch-directory history for the Spawn Card.
//!
//! History is steering state, not a suggestion cache: the selected entry is the
//! path that will be launched. Local and remote histories use disjoint stable
//! keys, validation is never persisted, and only successful launches enter the
//! MRU.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(super) const MAX_FOLDERS_PER_HOST: usize = 20;
const HISTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum FolderHistoryHost {
    Local,
    Remote { node_id: String },
}

impl FolderHistoryHost {
    pub(crate) fn remote(node_id: impl Into<String>) -> Option<Self> {
        let node_id = node_id.into();
        (!node_id.trim().is_empty()).then_some(Self::Remote { node_id })
    }

    fn storage_key(&self) -> String {
        match self {
            Self::Local => "local".to_string(),
            Self::Remote { node_id } => format!("remote:{node_id}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FolderHistoryEntry {
    pub(super) path: PathBuf,
    pub(super) last_success: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedFolderHistory {
    version: u32,
    hosts: BTreeMap<String, Vec<FolderHistoryEntry>>,
}

impl Default for PersistedFolderHistory {
    fn default() -> Self {
        Self {
            version: HISTORY_SCHEMA_VERSION,
            hosts: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryFileState {
    Missing,
    Loaded,
    Protected,
}

/// The persistent MRU. A corrupt source is protected from overwrite so an
/// empty fallback can never destroy the user's last recoverable history.
pub(super) struct FolderHistory {
    persisted: PersistedFolderHistory,
    file_state: HistoryFileState,
    #[cfg(not(target_family = "wasm"))]
    file: Option<PathBuf>,
}

impl FolderHistory {
    pub(super) fn load() -> Self {
        let (persisted, file_state) = load_history();
        Self {
            persisted,
            file_state,
            #[cfg(not(target_family = "wasm"))]
            file: Some(history_file()),
        }
    }

    #[cfg(test)]
    fn empty() -> Self {
        Self {
            persisted: PersistedFolderHistory::default(),
            file_state: HistoryFileState::Missing,
            #[cfg(not(target_family = "wasm"))]
            file: None,
        }
    }

    #[cfg(all(test, not(target_family = "wasm")))]
    fn with_file(file: PathBuf) -> Self {
        let (persisted, file_state) = load_history_from(&file);
        Self {
            persisted,
            file_state,
            file: Some(file),
        }
    }

    pub(super) fn entries(&self, host: &FolderHistoryHost) -> &[FolderHistoryEntry] {
        self.persisted
            .hosts
            .get(&host.storage_key())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn search(&self, host: &FolderHistoryHost, query: &str) -> Vec<&FolderHistoryEntry> {
        let query = query.trim().to_lowercase();
        self.entries(host)
            .iter()
            .filter(|entry| {
                query.is_empty() || entry.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .collect()
    }

    /// Record only an acknowledged launch. Deduplication uses the normalized
    /// path and moves an existing entry to the front.
    pub(super) fn record_success(
        &mut self,
        host: &FolderHistoryHost,
        path: &Path,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let path = normalize_path(host, path)?;
        let previous = self.persisted.clone();
        let key = host.storage_key();
        let entries = self.persisted.hosts.entry(key).or_default();
        entries.retain(|entry| entry.path != path);
        entries.insert(
            0,
            FolderHistoryEntry {
                path,
                last_success: at,
            },
        );
        entries.truncate(MAX_FOLDERS_PER_HOST);
        #[cfg(not(target_family = "wasm"))]
        if let Some(path) = self.file.as_deref() {
            if let Err(error) = save_history_to(path, &self.persisted, self.file_state) {
                self.persisted = previous;
                return Err(error);
            }
        }
        #[cfg(target_family = "wasm")]
        if let Err(error) = save_history(&self.persisted, self.file_state) {
            self.persisted = previous;
            return Err(error);
        }
        self.file_state = HistoryFileState::Loaded;
        Ok(())
    }
}

/// One open card's browser-style navigation. This is intentionally separate
/// from MRU ordering: going Back must not reorder persisted history before a
/// launch succeeds.
#[derive(Clone, Debug, Default)]
pub(super) struct FolderNavigation {
    paths: Vec<PathBuf>,
    cursor: Option<usize>,
}

impl FolderNavigation {
    pub(super) fn reset(&mut self, selected: Option<PathBuf>) {
        self.paths.clear();
        self.cursor = None;
        if let Some(path) = selected {
            self.paths.push(path);
            self.cursor = Some(0);
        }
    }

    pub(super) fn select(&mut self, path: PathBuf) {
        if self.current().is_some_and(|current| current == path) {
            return;
        }
        if let Some(cursor) = self.cursor {
            self.paths.truncate(cursor + 1);
        } else {
            self.paths.clear();
        }
        self.paths.push(path);
        self.cursor = Some(self.paths.len() - 1);
    }

    pub(super) fn back(&mut self) -> Option<&Path> {
        let cursor = self.cursor?;
        if cursor == 0 {
            return None;
        }
        self.cursor = Some(cursor - 1);
        self.current()
    }

    pub(super) fn forward(&mut self) -> Option<&Path> {
        let cursor = self.cursor?;
        if cursor + 1 >= self.paths.len() {
            return None;
        }
        self.cursor = Some(cursor + 1);
        self.current()
    }

    pub(super) fn can_back(&self) -> bool {
        self.cursor.is_some_and(|cursor| cursor > 0)
    }

    pub(super) fn can_forward(&self) -> bool {
        self.cursor
            .is_some_and(|cursor| cursor + 1 < self.paths.len())
    }

    pub(super) fn current(&self) -> Option<&Path> {
        self.cursor
            .and_then(|cursor| self.paths.get(cursor))
            .map(PathBuf::as_path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryValidation {
    Unknown,
    Checking,
    Valid,
    Stale,
    Unverifiable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryValidationRequest {
    pub(crate) generation: u64,
    pub(crate) host: FolderHistoryHost,
    pub(crate) path: PathBuf,
}

/// Generation-bound validation prevents a late remote answer for the previous
/// host/path from enabling Confirm.
#[derive(Clone, Debug)]
pub(super) struct DirectoryValidationState {
    generation: u64,
    request: Option<DirectoryValidationRequest>,
    status: DirectoryValidation,
}

impl Default for DirectoryValidationState {
    fn default() -> Self {
        Self {
            generation: 0,
            request: None,
            status: DirectoryValidation::Unknown,
        }
    }
}

impl DirectoryValidationState {
    pub(super) fn begin(
        &mut self,
        host: FolderHistoryHost,
        path: PathBuf,
    ) -> DirectoryValidationRequest {
        self.generation = self.generation.wrapping_add(1);
        let request = DirectoryValidationRequest {
            generation: self.generation,
            host,
            path,
        };
        self.request = Some(request.clone());
        self.status = DirectoryValidation::Checking;
        request
    }

    pub(super) fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.request = None;
        self.status = DirectoryValidation::Unknown;
    }

    pub(super) fn apply(
        &mut self,
        request: &DirectoryValidationRequest,
        result: DirectoryValidation,
    ) -> bool {
        if self.request.as_ref() != Some(request) {
            return false;
        }
        self.status = result;
        true
    }

    pub(super) fn status(&self) -> DirectoryValidation {
        self.status
    }

    pub(super) fn is_valid(&self) -> bool {
        self.status == DirectoryValidation::Valid
    }
}

fn normalize_path(host: &FolderHistoryHost, path: &Path) -> anyhow::Result<PathBuf> {
    let raw = path.as_os_str().to_string_lossy();
    anyhow::ensure!(
        !raw.chars().any(char::is_control),
        "launch directory contains control characters"
    );
    if matches!(host, FolderHistoryHost::Remote { .. }) {
        anyhow::ensure!(
            raw.starts_with('/'),
            "remote launch directory must be a POSIX absolute path"
        );
        let mut parts = Vec::new();
        for part in raw.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    anyhow::ensure!(parts.pop().is_some(), "launch directory escapes its root");
                }
                part => parts.push(part),
            }
        }
        return Ok(PathBuf::from(format!("/{}", parts.join("/"))));
    }
    anyhow::ensure!(path.is_absolute(), "launch directory must be absolute");

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::ensure!(normalized.pop(), "launch directory escapes its root");
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    anyhow::ensure!(
        normalized.is_absolute(),
        "launch directory must remain absolute"
    );
    Ok(normalized)
}

#[cfg(not(target_family = "wasm"))]
fn history_file() -> PathBuf {
    warp_core::paths::data_dir().join("spawn-folder-history.json")
}

#[cfg(not(target_family = "wasm"))]
fn load_history_from(path: &Path) -> (PersistedFolderHistory, HistoryFileState) {
    match std::fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<PersistedFolderHistory>(&contents) {
            Ok(history) if history.version == HISTORY_SCHEMA_VERSION => {
                (history, HistoryFileState::Loaded)
            }
            Ok(_) => {
                log::warn!(
                    "spawn-folder history {} has an unsupported schema; protecting it",
                    path.display()
                );
                (
                    PersistedFolderHistory::default(),
                    HistoryFileState::Protected,
                )
            }
            Err(error) => {
                log::warn!(
                    "spawn-folder history {} failed to parse ({error}); protecting it",
                    path.display()
                );
                (
                    PersistedFolderHistory::default(),
                    HistoryFileState::Protected,
                )
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (PersistedFolderHistory::default(), HistoryFileState::Missing)
        }
        Err(error) => {
            log::warn!(
                "spawn-folder history {} could not be read ({error}); protecting it",
                path.display()
            );
            (
                PersistedFolderHistory::default(),
                HistoryFileState::Protected,
            )
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn load_history() -> (PersistedFolderHistory, HistoryFileState) {
    load_history_from(&history_file())
}

#[cfg(not(target_family = "wasm"))]
fn save_history_to(
    path: &Path,
    history: &PersistedFolderHistory,
    state: HistoryFileState,
) -> anyhow::Result<()> {
    use anyhow::{bail, Context as _};
    use std::io::Write as _;

    if state == HistoryFileState::Protected {
        bail!(
            "refusing to overwrite unreadable spawn-folder history {}",
            path.display()
        );
    }
    let parent = path
        .parent()
        .with_context(|| format!("history path {} has no parent", path.display()))?;
    std::fs::create_dir_all(parent)?;
    let json = serde_json::to_vec_pretty(history)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&json)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| anyhow::Error::from(error.error))?;
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn save_history(history: &PersistedFolderHistory, state: HistoryFileState) -> anyhow::Result<()> {
    save_history_to(&history_file(), history, state)
}

#[cfg(target_family = "wasm")]
fn load_history() -> (PersistedFolderHistory, HistoryFileState) {
    (PersistedFolderHistory::default(), HistoryFileState::Missing)
}

#[cfg(target_family = "wasm")]
fn save_history(_history: &PersistedFolderHistory, _state: HistoryFileState) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
