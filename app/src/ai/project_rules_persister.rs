//! `ProjectRulesPersister` — bridge for project rules (WARP.md / AGENTS.md) persistence.
//!
//! This thin singleton model has two responsibilities:
//!
//! 1. Subscribe to [`ProjectContextModel`]'s [`KnownRulesChanged`] event, convert
//!    `discovered_rules` / `deleted_rules` to [`ModelEvent::UpsertProjectRules`] /
//!    [`ModelEvent::DeleteProjectRules`] and write to SQLite `project_rules` table.
//! 2. Subscribe to [`DetectedRepositories`]' `DetectedGitRepo` event; when user enters
//!    a new git repo, trigger [`ProjectContextModel::index_and_store_rules`] to scan
//!    WARP.md / AGENTS.md.
//!
//! Historically, these two bits of logic lived in `PersistedWorkspace::new`, tightly
//! coupled to LSP enable persistence and "visited git repo history". After LSP + workspace
//! history went offline, this bridge must live independently; otherwise project rules
//! stop writing to disk / stop auto-scanning with cd.

use std::sync::mpsc::SyncSender;

use ai::project_context::model::{ProjectContextModel, ProjectContextModelEvent};
use repo_metadata::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::ModelEvent;

/// See module-level documentation for details.
pub struct ProjectRulesPersister {
    /// Channel for writing to SQLite; `None` means current build doesn't enable persistence.
    persistence_tx: Option<SyncSender<ModelEvent>>,
}

impl Entity for ProjectRulesPersister {
    type Event = ();
}

impl SingletonEntity for ProjectRulesPersister {}

impl ProjectRulesPersister {
    /// Register two subscriptions:
    /// - `ProjectContextModel` → convert rule delta to SQLite ModelEvent;
    /// - `DetectedRepositories` → trigger rule scan when entering git repo.
    pub fn new(
        persistence_tx: Option<SyncSender<ModelEvent>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&ProjectContextModel::handle(ctx), |me, event, _ctx| {
            let ProjectContextModelEvent::KnownRulesChanged(delta) = event else {
                return;
            };

            let mut events = vec![];

            if !delta.discovered_rules.is_empty() {
                events.push(ModelEvent::UpsertProjectRules {
                    project_rule_paths: delta.discovered_rules.clone(),
                });
            }

            if !delta.deleted_rules.is_empty() {
                events.push(ModelEvent::DeleteProjectRules {
                    path: delta.deleted_rules.clone(),
                });
            }

            if events.is_empty() {
                return;
            }

            let Some(tx) = me.persistence_tx.as_ref() else {
                return;
            };

            for event in events {
                if let Err(err) = tx.send(event) {
                    log::warn!("ProjectRulesPersister: failed to write SQLite: {err}");
                }
            }
        });

        ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |_me, event, ctx| {
            let DetectedRepositoriesEvent::DetectedGitRepo { repository, .. } = event;
            let repo_path = repository.as_ref(ctx).root_dir().to_local_path_lossy();

            ProjectContextModel::handle(ctx).update(ctx, |model, ctx| {
                let _ = model.index_and_store_rules(repo_path, ctx);
            });
        });

        Self { persistence_tx }
    }

    /// For testing only: no persistence channel binding, no model subscriptions.
    #[cfg(test)]
    pub fn new_for_test(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            persistence_tx: None,
        }
    }
}
