//! Active AI (active AI) subchain BYOP adaptation.
//!
//! Covers three categories:
//! - `prompt_suggestions`: after command completion, provide "ask Agent" suggestion (Simple/Coding)
//! - `nld_predict`: real-time completion as user types in Agent input box
//! - `relevant_files`: filter relevant subset from given file list based on query
//!
//! Common pattern:
//! 1. Caller invokes `dispatch::*` helper series before spawn (with `&AppContext`),
//!    extracts `OneshotConfig` + rendered system/user prompt → `RenderedRequest`
//! 2. Within spawn closure, calls `run_*(req)` to send request + parse, returns response type for each subchain
//! 3. UI callback directly consumes returned response, completely equivalent to original `ServerApi` path
//!
//! No BYOP configuration (`active_ai_model` decode fails) → `dispatch::*` returns `None`,
//! caller silently no-ops (Zaplex has removed cloud, no longer fallback to ServerApi).

use minijinja::{context, Environment};
use serde::Serialize;
use std::sync::OnceLock;

use super::oneshot::{
    byop_oneshot_completion, resolve_active_ai_oneshot, resolve_next_command_oneshot,
    OneshotConfig, OneshotOptions,
};
use crate::ai::predict::generate_am_query_suggestions::GenerateAMQuerySuggestionsResponse;

pub mod parsing;

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

static ENV: OnceLock<Environment<'static>> = OnceLock::new();

fn build_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template(
        "prompt_suggestions_system.j2",
        include_str!("../prompts/active_ai/prompt_suggestions_system.j2"),
    )
    .expect("prompt_suggestions_system parses");
    env.add_template(
        "prompt_suggestions_user.j2",
        include_str!("../prompts/active_ai/prompt_suggestions_user.j2"),
    )
    .expect("prompt_suggestions_user parses");
    env.add_template(
        "nld_predict_system.j2",
        include_str!("../prompts/active_ai/nld_predict_system.j2"),
    )
    .expect("nld_predict_system parses");
    env.add_template(
        "nld_predict_user.j2",
        include_str!("../prompts/active_ai/nld_predict_user.j2"),
    )
    .expect("nld_predict_user parses");
    env.add_template(
        "relevant_files_system.j2",
        include_str!("../prompts/active_ai/relevant_files_system.j2"),
    )
    .expect("relevant_files_system parses");
    env.add_template(
        "relevant_files_user.j2",
        include_str!("../prompts/active_ai/relevant_files_user.j2"),
    )
    .expect("relevant_files_user parses");
    env.add_template(
        "next_command_system.j2",
        include_str!("../prompts/active_ai/next_command_system.j2"),
    )
    .expect("next_command_system parses");
    env.add_template(
        "next_command_user.j2",
        include_str!("../prompts/active_ai/next_command_user.j2"),
    )
    .expect("next_command_user parses");
    env.add_template(
        "workflow_metadata_system.j2",
        include_str!("../prompts/active_ai/workflow_metadata_system.j2"),
    )
    .expect("workflow_metadata_system parses");
    env.add_template(
        "workflow_metadata_user.j2",
        include_str!("../prompts/active_ai/workflow_metadata_user.j2"),
    )
    .expect("workflow_metadata_user parses");
    env
}

fn env() -> &'static Environment<'static> {
    ENV.get_or_init(build_env)
}

fn render(template: &str, ctx: minijinja::Value) -> String {
    env()
        .get_template(template)
        .and_then(|t| t.render(ctx))
        .unwrap_or_else(|e| {
            log::warn!("[active_ai] render {template} failed: {e}");
            String::new()
        })
}

// ---------------------------------------------------------------------------
// Shared context snippets
// ---------------------------------------------------------------------------

/// Compact context of a single completed command block (consumed by prompt_suggestions / nld_predict).
#[derive(Debug, Clone, Serialize, Default)]
pub struct BlockSnippet {
    pub command: String,
    pub output_summary: String,
    pub exit_code: i32,
    pub pwd: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LastBlockSnippet {
    pub command: String,
    pub exit_code: i32,
    pub pwd: String,
}

/// Request with prompt rendered and OneshotConfig parsed — passed across spawn boundary.
pub struct RenderedRequest {
    pub cfg: OneshotConfig,
    pub system: String,
    pub user: String,
    pub opts: OneshotOptions,
}

// ---------------------------------------------------------------------------
// prompt_suggestions
// ---------------------------------------------------------------------------

pub mod prompt_suggestions {
    use super::*;
    use warpui::{AppContext, EntityId};

    pub struct Input {
        pub recent_blocks: Vec<BlockSnippet>,
        pub system_context: Option<String>,
        pub last_exit_code: i32,
    }

    /// Before spawn: decode BYOP configuration + render prompt. `None` ⇒ silent no-op.
    pub fn dispatch(
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
        input: Input,
    ) -> Option<RenderedRequest> {
        let cfg = resolve_active_ai_oneshot(app, terminal_view_id)?;
        let system = render("prompt_suggestions_system.j2", context! {});
        let user = render(
            "prompt_suggestions_user.j2",
            context! {
                recent_blocks => input.recent_blocks,
                system_context => input.system_context,
                last_exit_code => input.last_exit_code,
            },
        );
        Some(RenderedRequest {
            cfg,
            system,
            user,
            opts: OneshotOptions {
                response_format_json: true,
                max_chars: Some(6000),
                ..Default::default()
            },
        })
    }

    /// Within spawn: send request + parse. Failure → `None` (caller maps to Error).
    pub async fn run(req: RenderedRequest) -> Option<GenerateAMQuerySuggestionsResponse> {
        let raw = match byop_oneshot_completion(&req.cfg, &req.system, &req.user, &req.opts).await {
            Ok(s) => s,
            Err(e) => {
                log::debug!("[active_ai] prompt_suggestions oneshot failed: {e:#}");
                return None;
            }
        };
        log::debug!(
            "[active_ai] prompt_suggestions raw response ({} chars): {raw}",
            raw.len()
        );
        parsing::parse_suggestion(&raw)
    }
}

// ---------------------------------------------------------------------------
// nld_predict
// ---------------------------------------------------------------------------

pub mod nld_predict {
    use super::*;
    use warpui::{AppContext, EntityId};

    pub struct Input {
        pub partial_query: String,
        pub last_block: Option<LastBlockSnippet>,
        pub system_context: Option<String>,
    }

    pub fn dispatch(
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
        input: Input,
    ) -> Option<RenderedRequest> {
        let cfg = resolve_active_ai_oneshot(app, terminal_view_id)?;
        let system = render("nld_predict_system.j2", context! {});
        let user = render(
            "nld_predict_user.j2",
            context! {
                partial_query => input.partial_query,
                last_block => input.last_block,
                system_context => input.system_context,
            },
        );
        Some(RenderedRequest {
            cfg,
            system,
            user,
            opts: OneshotOptions {
                response_format_json: false,
                max_chars: Some(4000),
                ..Default::default()
            },
        })
    }

    pub async fn run(req: RenderedRequest) -> Option<String> {
        let raw = match byop_oneshot_completion(&req.cfg, &req.system, &req.user, &req.opts).await {
            Ok(s) => s,
            Err(e) => {
                log::debug!("[active_ai] nld_predict oneshot failed: {e:#}");
                return None;
            }
        };
        parsing::sanitize_predict(&raw)
    }
}

// ---------------------------------------------------------------------------
// relevant_files
// ---------------------------------------------------------------------------

pub mod relevant_files {
    use super::*;
    use warpui::{AppContext, EntityId};

    #[derive(Debug, Clone, Serialize)]
    pub struct FileEntry {
        pub path: String,
        pub symbols: String,
    }

    pub struct Input {
        pub query: String,
        pub files: Vec<FileEntry>,
    }

    pub struct Prepared {
        pub req: RenderedRequest,
        pub input_paths: Vec<String>,
    }

    pub fn dispatch(
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
        input: Input,
    ) -> Option<Prepared> {
        let cfg = resolve_active_ai_oneshot(app, terminal_view_id)?;
        let input_paths: Vec<String> = input.files.iter().map(|f| f.path.clone()).collect();
        let system = render("relevant_files_system.j2", context! {});
        let user = render(
            "relevant_files_user.j2",
            context! {
                query => input.query,
                files => input.files,
            },
        );
        Some(Prepared {
            req: RenderedRequest {
                cfg,
                system,
                user,
                opts: OneshotOptions {
                    response_format_json: true,
                    max_chars: Some(12000),
                    ..Default::default()
                },
            },
            input_paths,
        })
    }

    pub async fn run(prepared: Prepared) -> Vec<String> {
        let raw = match byop_oneshot_completion(
            &prepared.req.cfg,
            &prepared.req.system,
            &prepared.req.user,
            &prepared.req.opts,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                log::debug!("[active_ai] relevant_files oneshot failed: {e:#}");
                return Vec::new();
            }
        };
        parsing::parse_relevant_files(&raw, &prepared.input_paths)
    }
}

// ---------------------------------------------------------------------------
// workflow_metadata (Workflow Editor's Autofill button: command → parameterized metadata)
// ---------------------------------------------------------------------------

pub mod workflow_metadata {
    use super::*;
    use warpui::{AppContext, EntityId};

    pub use parsing::WorkflowMetadataDto;

    pub struct Input {
        pub command: String,
    }

    /// Before spawn: decode BYOP configuration + render prompt. `None` ⇒ caller prompts user to configure BYOP.
    pub fn dispatch(
        app: &AppContext,
        terminal_view_id: Option<EntityId>,
        input: Input,
    ) -> Option<RenderedRequest> {
        let cfg = resolve_active_ai_oneshot(app, terminal_view_id)?;
        let system = render("workflow_metadata_system.j2", context! {});
        let user = render(
            "workflow_metadata_user.j2",
            context! {
                command => input.command,
            },
        );
        Some(RenderedRequest {
            cfg,
            system,
            user,
            opts: OneshotOptions {
                response_format_json: true,
                max_chars: Some(4000),
                ..Default::default()
            },
        })
    }

    /// Within spawn: send request + parse. Failure → `None` (caller maps to BadCommand).
    pub async fn run(req: RenderedRequest) -> Option<WorkflowMetadataDto> {
        let raw = match byop_oneshot_completion(&req.cfg, &req.system, &req.user, &req.opts).await {
            Ok(s) => s,
            Err(e) => {
                log::debug!("[active_ai] workflow_metadata oneshot failed: {e:#}");
                return None;
            }
        };
        log::debug!(
            "[active_ai] workflow_metadata raw response ({} chars): {raw}",
            raw.len()
        );
        parsing::parse_workflow_metadata(&raw)
    }
}

// ---------------------------------------------------------------------------
// next_command (gray completion / zero-state suggestion)
// ---------------------------------------------------------------------------

pub mod next_command {
    use super::*;
    use warpui::{AppContext, EntityId};

    #[derive(Debug, Serialize)]
    struct UserRuleCtx {
        name: Option<String>,
        content: String,
    }

    pub struct Input {
        pub recent_blocks: Vec<BlockSnippet>,
        /// Similar command context already selected from history DB on client side (optional).
        pub history_context: String,
        pub system_context: Option<String>,
        /// User-entered prefix (must be used as output prefix).
        pub prefix: Option<String>,
        /// Previously rejected suggestions (avoid repetition).
        pub rejected_suggestions: Vec<String>,
        /// Snapshot of global rules configured by user in Settings → Agents → Rules.
        pub user_rules: Vec<(Option<String>, String)>,
    }

    /// Pre-spawn: decode BYOP configuration (needs `&AppContext`). `None` ⇒ silent no-op.
    pub fn resolve(app: &AppContext, terminal_view_id: Option<EntityId>) -> Option<OneshotConfig> {
        resolve_next_command_oneshot(app, terminal_view_id)
    }

    /// In-spawn: render prompt and send request using cfg + Input.
    /// Template rendering does not depend on AppContext, can be called synchronously within spawn.
    pub async fn run_with(cfg: OneshotConfig, input: Input) -> Option<String> {
        let user_rule_ctxs: Vec<UserRuleCtx> = input
            .user_rules
            .into_iter()
            .map(|(name, content)| UserRuleCtx { name, content })
            .collect();
        let system = render("next_command_system.j2", context! {
            user_rules => user_rule_ctxs,
        });
        let user = render(
            "next_command_user.j2",
            context! {
                recent_blocks => input.recent_blocks,
                history_context => input.history_context,
                system_context => input.system_context,
                prefix => input.prefix,
                rejected_suggestions => input.rejected_suggestions,
            },
        );
        let opts = OneshotOptions {
            response_format_json: false,
            max_chars: Some(8000),
            ..Default::default()
        };
        let raw = match byop_oneshot_completion(&cfg, &system, &user, &opts).await {
            Ok(s) => s,
            Err(e) => {
                log::debug!("[active_ai] next_command oneshot failed: {e:#}");
                return None;
            }
        };
        log::info!(
            "[active_ai] next_command raw response ({} chars): {raw:?}",
            raw.len()
        );
        let sanitized = parsing::sanitize_predict(&raw);
        if sanitized.is_none() && !raw.trim().is_empty() {
            log::warn!("[active_ai] next_command sanitize REJECTED raw response");
        }
        sanitized
    }
}
