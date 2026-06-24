//! View model for the "Candidates" area — flattens the result from `warp_ssh_manager::load_candidates()`
//! (plus imported alias set and collapse state) into a UI-friendly [`CandidateRow`]
//! list.
//!
//! Design key points (corresponding to `specs/gh-110-ssh-config-import/{PRODUCT,TECH}.md`):
//!
//! - `rows()` is a **pure function**: depends only on the current view-model fields, touches no IO or runtime.
//!   Unit tests can directly construct a `CandidatesViewModel` and assert the output. This is exactly the point
//!   required in TDD discussions — PR 2's render-layer warpui testing is too costly, extracting the "which rows
//!   should display" logic for unit tests suffices to cover key decisions.
//! - `refresh()` synchronously calls `warp_ssh_manager::load_candidates()` (<10KB file,
//!   see TECH.md §3.1 trade-offs), stores the result in `state`.
//! - `on_tree_changed()` is called by the panel after subscribing to `SshTreeChangedNotifier` — collects
//!   all `host` fields from servers in the tree into a `HashSet`, used as the criterion for the "Added"
//!   badge (PRODUCT.md decision E).
//! - "Already imported" is determined by `host == alias`. Import logic on the panel side sets `server.host`
//!   to the candidate alias (PRODUCT.md decision I), so comparison semantics here align with import semantics.
//!
//! All fields are `pub(crate)`, exposed only to `panel.rs`; `CandidatesViewModel` itself
//! is exposed to `mod.rs` via `pub` re-export.

use std::collections::HashSet;

use settings::Setting;
use warp_ssh_manager::{LoadOutcome, LoadResult, SshConfigCandidate, load_candidates};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::settings::SshSettings;

/// View showing the source and status of one candidate server line from `~/.ssh/config` in the UI.
pub struct CandidatesViewModel {
    /// Latest load result. `None` means the model was just created and no refresh has been triggered yet.
    state: Option<LoadResult>,
    /// Set of all `host` fields from servers in the tree. `rows()` uses it to determine `added`.
    added_aliases: HashSet<String>,
    /// Section collapse state (PRODUCT.md UX table "Many candidates"). Expanded by default.
    expanded: bool,
}

impl Default for CandidatesViewModel {
    fn default() -> Self {
        Self::new()
    }
}

impl CandidatesViewModel {
    /// Empty constructor — used when the model is first added to the App via `add_model`. `refresh()` must be
    /// triggered by the caller at an appropriate time (can be called once in the panel's `new` method).
    pub fn new() -> Self {
        Self {
            state: None,
            added_aliases: HashSet::new(),
            expanded: true,
        }
    }

    /// Test constructor: explicitly sets internal state, bypassing runtime / IO, to directly drive
    /// various branches of `rows()`.
    #[cfg(test)]
    pub fn with_state(
        state: Option<LoadResult>,
        added_aliases: HashSet<String>,
        expanded: bool,
    ) -> Self {
        Self {
            state,
            added_aliases,
            expanded,
        }
    }

    /// Synchronously re-read `~/.ssh/config`, store the result in `state`.
    ///
    /// By design, does not return errors — `LoadOutcome::Error` already carries the error message string back,
    /// which the UI displays as a red error line (see PRODUCT.md UX table "Parse / IO error").
    ///
    /// When the "Auto-discover SSH hosts" setting is disabled, skips reading and clears the state.
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let auto_discover = *SshSettings::as_ref(ctx).enable_ssh_auto_discovery.value();
        if !auto_discover {
            self.state = None;
            ctx.notify();
            return;
        }
        self.state = Some(load_candidates());
        ctx.notify();
    }

    /// Tree change callback — rebuilds `added_aliases` using the provided server hosts.
    ///
    /// Accepts `impl IntoIterator<Item = String>` instead of `&SshRepository` so tests don't need
    /// to provide a real SQLite connection; the caller (panel) is responsible for collecting the host fields
    /// from `list_nodes` + `get_server` into an iterator before passing it in.
    pub fn on_tree_changed<I>(&mut self, hosts: I, ctx: &mut ModelContext<Self>)
    where
        I: IntoIterator<Item = String>,
    {
        self.added_aliases = hosts.into_iter().collect();
        ctx.notify();
    }

    /// Toggle the "section collapse" state.
    pub fn toggle_expanded(&mut self, ctx: &mut ModelContext<Self>) {
        self.expanded = !self.expanded;
        ctx.notify();
    }

    /// Whether the section is expanded (the panel decides whether to render body rows based on this).
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Find a candidate by alias — used when handling `ImportCandidate { alias }` action,
    /// calls `SshRepository::create_server` after retrieving all fields.
    pub fn find_candidate(&self, alias: &str) -> Option<&SshConfigCandidate> {
        let state = self.state.as_ref()?;
        match &state.outcome {
            LoadOutcome::Loaded(v) => v.iter().find(|c| c.alias == alias),
            LoadOutcome::NotFound | LoadOutcome::Error(_) => None,
        }
    }

    /// Human-readable string of the current `~/.ssh/config` path (for use in `notes = "Imported from {}"`).
    /// `None` means it has not been loaded yet or the home directory is unavailable.
    pub fn path_display(&self) -> Option<String> {
        self.state
            .as_ref()
            .and_then(|s| s.path.as_ref())
            .map(|p| p.display().to_string())
    }

    /// Flatten the current state into a list of rows — see the "pure function" convention in the module docs.
    ///
    /// Output semantics (corresponding to PRODUCT.md §5 UX table):
    /// - No refresh yet: returns empty Vec (panel does not render the section when `state == None`).
    /// - `NotFound`: Header + one `NotFound` row.
    /// - `Error`: Header + one `Error` row (can_refresh=true allows user to retry after fixing config).
    /// - `Loaded(empty)`: Header + one `Empty` row.
    /// - `Loaded(non-empty)`: Header (count = N) + N `Candidate` rows,
    ///   with each row's `added` field determined by `added_aliases.contains(alias)`.
    pub fn rows(&self) -> Vec<CandidateRow> {
        let Some(state) = self.state.as_ref() else {
            return Vec::new();
        };

        let path_display = state
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let mut out = Vec::new();
        let count = match &state.outcome {
            LoadOutcome::Loaded(v) => v.len(),
            LoadOutcome::NotFound | LoadOutcome::Error(_) => 0,
        };
        // Header is always the first row — even if the section is collapsed, the panel still renders the header
        // (that is the toggle entry point). `can_refresh = true` always holds: users can click Refresh to re-read
        // from any state.
        out.push(CandidateRow::Header {
            path_display: path_display.clone(),
            count,
            can_refresh: true,
        });

        // When the section is collapsed, keep only the header; do not render the body.
        if !self.expanded {
            return out;
        }

        match &state.outcome {
            LoadOutcome::NotFound => {
                out.push(CandidateRow::NotFound { path_display });
            }
            LoadOutcome::Error(msg) => {
                out.push(CandidateRow::Error {
                    path_display,
                    message: msg.clone(),
                });
            }
            LoadOutcome::Loaded(v) if v.is_empty() => {
                out.push(CandidateRow::Empty { path_display });
            }
            LoadOutcome::Loaded(v) => {
                for c in v {
                    out.push(CandidateRow::Candidate {
                        alias: c.alias.clone(),
                        hostname: c.hostname.clone(),
                        user: c.user.clone(),
                        port: c.port,
                        identity_file: c.identity_file.as_ref().map(|p| p.display().to_string()),
                        added: self.added_aliases.contains(&c.alias),
                    });
                }
            }
        }

        out
    }
}

/// A UI-friendly row. Header is always at the front, followed by either a single status row
/// (NotFound / Empty / Error) or a sequence of Candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandidateRow {
    Header {
        path_display: String,
        count: usize,
        can_refresh: bool,
    },
    NotFound {
        path_display: String,
    },
    Empty {
        path_display: String,
    },
    Error {
        path_display: String,
        message: String,
    },
    Candidate {
        alias: String,
        hostname: Option<String>,
        user: Option<String>,
        port: Option<u16>,
        identity_file: Option<String>,
        added: bool,
    },
}

impl Entity for CandidatesViewModel {
    type Event = ();
}

#[cfg(test)]
#[path = "candidates_tests.rs"]
mod tests;

// Allow test code to ignore specific PathBuf disk paths — the helper constructs a `LoadResult`
// with a fixed display string. Used by the test module, so placed at outer scope for convenient reuse with #[cfg(test)].
#[cfg(test)]
pub(crate) fn fake_load_result_loaded(path: &str, cands: Vec<SshConfigCandidate>) -> LoadResult {
    LoadResult {
        path: Some(std::path::PathBuf::from(path)),
        outcome: LoadOutcome::Loaded(cands),
    }
}

#[cfg(test)]
pub(crate) fn fake_load_result_not_found(path: &str) -> LoadResult {
    LoadResult {
        path: Some(std::path::PathBuf::from(path)),
        outcome: LoadOutcome::NotFound,
    }
}

#[cfg(test)]
pub(crate) fn fake_load_result_error(path: &str, msg: &str) -> LoadResult {
    LoadResult {
        path: Some(std::path::PathBuf::from(path)),
        outcome: LoadOutcome::Error(msg.to_string()),
    }
}
