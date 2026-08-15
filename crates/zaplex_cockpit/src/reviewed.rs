//! The user's **"I have looked at this"** marks (spec v3 §2 F7).
//!
//! A private note to yourself, and nothing more: it does not tell the agent
//! anything, does not approve a change, and does not touch the working tree. It
//! answers one question while you work through a fleet of agents — *did I
//! already read this one?*
//!
//! **Keyed by `session_id` alone**, deliberately. The Conductor's UI state is
//! keyed by `conductor::host_key`, which scopes an id by the host's identity —
//! right for hover handles and expansion, because a *project* id is a path
//! (`/home/me/proj`) that several hosts can share. It is the wrong key here for
//! two reasons:
//!
//! - A daemon's `host_id` is a fresh UUID **per daemon start**
//!   (`server_model.rs`), so it identifies a *process*, not a machine. It is
//!   stable across reconciles — which is all the hover maps need — but a mark
//!   stored under it would be orphaned by the next daemon restart, and the user
//!   would simply find their marks gone with no explanation.
//! - A session id is the conversation's own durable identity — the very thing
//!   `--resume <id>` takes — and it survives restarts on both sides.
//!
//! The honest bound on that second point: an id is a provider-issued UUID in
//! practice, but neither provider *guarantees* one. Both fall back to deriving
//! it from a transcript's file name when the file is not shaped as expected
//! (`codex_sessions::session_id_from_path`, and Claude's transcript stem), and
//! nothing validates the result as a UUID. Two hosts each holding an oddly-named
//! transcript that derives the same string would share one mark. That is a wrong
//! tick on a private note, in a case that needs the same malformed file twice —
//! set against marks that would *certainly* vanish on every daemon restart. The
//! trade is deliberate.
//!
//! Bounded by count rather than by age: a mark is only worth keeping while its
//! session can still be found, but a long-running session may have been marked
//! long ago, so pruning by age would quietly drop exactly the marks a careful
//! user set first. The cap can in principle drop a mark that is still on screen
//! — but only past [`REVIEWED_LIMIT`] of them, which is orders of magnitude
//! beyond any working set a person reads through.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How many marks are kept. Far above any realistic working set — the cap is a
/// backstop against a file that grows forever, not a policy about how many
/// sessions you may review.
pub const REVIEWED_LIMIT: usize = 1000;

/// The set of sessions the user has marked as read, with when they did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedSessions {
    /// `session_id` → when it was marked. The timestamp is what the prune keeps
    /// order by; nothing displays it (yet).
    #[serde(default)]
    marks: BTreeMap<String, DateTime<Utc>>,
}

impl ReviewedSessions {
    /// Has the user marked this session as read?
    pub fn contains(&self, session_id: &str) -> bool {
        self.marks.contains_key(session_id)
    }

    /// Flip the mark. Returns the state afterwards (`true` = now marked).
    ///
    /// Pruning happens here rather than on save so that the in-memory set and
    /// the file can never disagree about what was kept.
    pub fn toggle(&mut self, session_id: &str, now: DateTime<Utc>) -> bool {
        if self.marks.remove(session_id).is_some() {
            return false;
        }
        self.marks.insert(session_id.to_string(), now);
        self.prune(REVIEWED_LIMIT);
        true
    }

    /// Drop the oldest marks beyond `limit`.
    pub fn prune(&mut self, limit: usize) {
        if self.marks.len() <= limit {
            return;
        }
        let mut by_age: Vec<(DateTime<Utc>, String)> = self
            .marks
            .iter()
            .map(|(id, at)| (*at, id.clone()))
            .collect();
        // Most recent first, then drop the tail.
        by_age.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, id) in by_age.into_iter().skip(limit) {
            self.marks.remove(&id);
        }
    }

    /// How many sessions are marked.
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

#[cfg(test)]
#[path = "reviewed_tests.rs"]
mod tests;
