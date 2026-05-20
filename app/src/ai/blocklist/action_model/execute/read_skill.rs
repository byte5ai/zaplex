use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionResultType;
use crate::ai::skills::{extract_skill_parent_directory, SkillManager, SkillTelemetryEvent};
use crate::send_telemetry_from_ctx;
use ai::agent::action_result::AnyFileContent;
use ai::skills::{parse_skill, SkillReference};
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

        match SkillManager::as_ref(ctx).skill_by_reference(skill_ref) {
            Some(skill) => {
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
                let content = FileContext::new(
                    skill.path.to_string_lossy().into_owned(),
                    AnyFileContent::StringContent(skill.content.clone()),
                    skill.line_range.clone(),
                    None,
                );
                ActionExecution::Sync(ReadSkillResult::Success { content }.into())
            }
            None => {
                // Cache miss 兜底:对于 `SkillReference::Path` 形式的引用,
                // 如果路径形状是合法的 skill 文件
                // (`.../<provider>/skills/<name>/SKILL.md` 或 warp managed skill 目录下),
                // 直接读盘解析,修复 issue #99 中描述的「skill 已存在但 cache 未热」场景。
                //
                // 设计取舍:
                // - 不主动 warm SkillManager cache。Cache 由 SkillWatcher 单向维护,
                //   在这里写入会破坏数据流。重复 read_skill 同一路径会重复读盘,
                //   但 SKILL.md 通常很小,可忽略。
                // - `extract_skill_parent_directory` 只校验路径形状,与 cache hit 时
                //   返回的 path 安全等级一致 —— 都不限定家目录前缀。这是有意的:
                //   project 内 skill (`/some/repo/.agents/skills/...`) 也需要能读。
                // - Windows 下正则用反斜杠分隔,Linux 风格 `/home/<u>/...` 路径会被
                //   拒绝;这意味着本兜底对 "Windows 主进程 + WSL session" 不生效,
                //   是 issue #99 的已知限制(见 PR 描述)。
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
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for ReadSkillExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "read_skill_tests.rs"]
mod tests;
