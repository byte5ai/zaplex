use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
#[cfg(feature = "local_fs")]
use crate::ai::agent::AIAgentActionResultType;
use crate::ai::skills::{SkillManager, SkillTelemetryEvent};
#[cfg(feature = "local_fs")]
use crate::ai::skills::extract_skill_parent_directory;
use crate::send_telemetry_from_ctx;
use ai::agent::action_result::AnyFileContent;
use ai::skills::SkillReference;
#[cfg(feature = "local_fs")]
use ai::skills::parse_skill;
use std::path::Path;
use warpui::{ModelContext, SingletonEntity};

use crate::ai::agent::AIAgentActionType;
use crate::ai::agent::ReadSkillRequest;
use crate::ai::agent::ReadSkillResult;
use ai::agent::action_result::FileContext;
use futures::future::{BoxFuture, FutureExt};
use warpui::Entity;

pub struct ReadSkillExecutor;

impl ReadSkillExecutor {
    pub fn new() -> Self {
        Self
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        // User-created skills are readable on demand.
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::ReadSkill(ReadSkillRequest { skill: skill_ref }) = &action.action
        else {
            return ActionExecution::InvalidAction;
        };

        let manager = SkillManager::as_ref(ctx);

        // Cache hit: proto's `SkillReference::Path(p)` hits here only if p is exactly
        // the real SKILL.md absolute path in the index.
        if let Some(skill) = manager.skill_by_reference(skill_ref) {
            send_telemetry_from_ctx!(
                SkillTelemetryEvent::Read {
                    reference: skill_ref.clone(),
                    name: Some(skill.name.clone()),
                    scope: Some(skill.scope),
                    provider: Some(skill.provider),
                    error: false,
                },
                ctx
            );
            return success_execution(skill);
        }

        // BYOP `read_skill` tool argument is a skill **name**, placed into
        // `SkillReference::SkillPath(name)` slot by `from_args` (avoids proto schema change).
        // On cache miss, reverse-lookup the real SKILL.md path by name, covering all skills
        // visible to the Skill manager (file skills + bundled skills).
        if let SkillReference::Path(p) = skill_ref {
            if let Some(candidate_name) = name_candidate(p) {
                if let Some(skill) = manager.find_skill_by_name(candidate_name) {
                    send_telemetry_from_ctx!(
                        SkillTelemetryEvent::Read {
                            reference: skill_ref.clone(),
                            name: Some(skill.name.clone()),
                            scope: Some(skill.scope),
                            provider: Some(skill.provider),
                            error: false,
                        },
                        ctx
                    );
                    return success_execution(skill);
                }
            }
        }

        // Cache miss fallback: for `SkillReference::Path` references,
        // if the path shape is a valid skill file
        // (`.../<provider>/skills/<name>/SKILL.md` or under warp-managed skill directory),
        // read directly from disk and parse, fixing the "skill exists but cache not warm" scenario in issue #99.
        //
        // Design trade-offs:
        // - Don't actively warm SkillManager cache. Cache is maintained unidirectionally by SkillWatcher;
        //   writing here would break the data flow. Repeated read_skill on same path causes repeated disk reads,
        //   but SKILL.md is usually small, negligible.
        // - `extract_skill_parent_directory` only validates path shape, same security level as returned
        //   path on cache hit — both don't restrict to home directory prefix. Intentional:
        //   project-internal skills (`/some/repo/.agents/skills/...`) must also be readable.
        // - On Windows, regex uses backslash separators; Linux-style `/home/<u>/...` paths are rejected;
        //   means this fallback doesn't work for "Windows main process + WSL session", a known
        //   limitation of issue #99 (see PR description).
        // Cache miss fallback only available in builds with local filesystem;
        // in WASM and other no-fs builds, `extract_skill_parent_directory` / `parse_skill`
        // don't exist, so reading from disk is impossible.
        #[cfg(feature = "local_fs")]
        if let SkillReference::Path(path) = skill_ref {
            if extract_skill_parent_directory(path).is_ok() {
                let path = path.clone();
                let skill_ref_for_async = skill_ref.clone();
                return ActionExecution::new_async(
                    async move { parse_skill(&path) },
                    move |parsed, _app| match parsed {
                        Ok(skill) => AIAgentActionResultType::ReadSkill(
                            ReadSkillResult::Success {
                                content: FileContext::new(
                                    skill.path.to_string_lossy().into_owned(),
                                    AnyFileContent::StringContent(skill.content.clone()),
                                    skill.line_range.clone(),
                                    None,
                                ),
                            },
                        ),
                        Err(err) => AIAgentActionResultType::ReadSkill(
                            ReadSkillResult::Error(format!(
                                "Skill not found: {skill_ref_for_async:?} ({err})"
                            )),
                        ),
                    },
                );
            }
        }

        send_telemetry_from_ctx!(
            SkillTelemetryEvent::Read {
                reference: skill_ref.clone(),
                name: None,
                scope: None,
                provider: None,
                error: true,
            },
            ctx
        );
        ActionExecution::Sync(
            ReadSkillResult::Error(format!("Skill not found: {:?}", skill_ref)).into(),
        )
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

/// Build a sync success execution from a parsed skill.
///
/// Extract helper so that generic `T` in `ActionExecution<T>` infers to the same type
/// in both `success_execution` and `new_async` paths (otherwise Rust requires explicit return type declaration).
fn success_execution(skill: &ai::skills::ParsedSkill) -> ActionExecution<anyhow::Result<ai::skills::ParsedSkill>> {
    let content = FileContext::new(
        skill.path.to_string_lossy().into_owned(),
        AnyFileContent::StringContent(skill.content.clone()),
        skill.line_range.clone(),
        None,
    );
    ActionExecution::Sync(ReadSkillResult::Success { content }.into())
}

/// Determine whether the value in `SkillReference::Path` should be treated as a skill **name** for reverse-lookup.
///
/// Real SKILL.md paths contain path separators (`/` or `\`) or are absolute paths, while BYOP
/// tool names (like `"build-feature"`) are pure strings. Distinguish these two cases
/// to avoid misinterpreting `/home/.../SKILL.md` as a name and missing the filesystem fallback.
fn name_candidate(p: &Path) -> Option<&str> {
    if p.is_absolute() {
        return None;
    }
    let s = p.to_str()?;
    if s.is_empty() || s.contains('/') || s.contains('\\') {
        return None;
    }
    Some(s)
}

impl Entity for ReadSkillExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "read_skill_tests.rs"]
mod tests;
