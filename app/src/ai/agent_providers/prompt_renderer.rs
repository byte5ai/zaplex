//! BYOP system prompt template rendering.
//!
//! Renders the `AIAgentContext` already collected by the warp client (env / git / skills / project_rules / current_time)
//! into the `system` message string for an OpenAI-compatible endpoint.
//!
//! ## Workflow
//!
//! 1. Extract the most recent `UserQuery.context: Arc<[AIAgentContext]>` from `params.input`
//!    (warp's `convert_to.rs::convert_input` reads the same thing)
//! 2. `collect_prompt_context` flattens each enum variant into a flat `PromptContext` struct
//! 3. `pick_template` selects `system/{anthropic,gpt,beast,codex,
//!    gemini,kimi,trinity,default}.j2` by substring-matching the model id (mirrors opencode's
//!    `packages/opencode/src/session/system.ts::provider`)
//! 4. minijinja rendering
//!
//! ## Template loading
//!
//! All templates are compiled into the binary via `include_str!` (zero runtime IO); changing a template requires a recompile.

use std::sync::OnceLock;

use ai::LLMId;
use chrono::Local;
use minijinja::{Environment, Value};
use serde::Serialize;

use crate::ai::agent::AIAgentContext;

// ---------------------------------------------------------------------------
// Template environment
// ---------------------------------------------------------------------------

static ENV: OnceLock<Environment<'static>> = OnceLock::new();

fn build_env() -> Environment<'static> {
    let mut env = Environment::new();

    // Partials
    env.add_template("partials/env.j2", include_str!("prompts/partials/env.j2"))
        .expect("env partial parses");
    env.add_template(
        "partials/skills.j2",
        include_str!("prompts/partials/skills.j2"),
    )
    .expect("skills partial parses");
    env.add_template(
        "partials/project_rules.j2",
        include_str!("prompts/partials/project_rules.j2"),
    )
    .expect("project_rules partial parses");
    env.add_template(
        "partials/user_rules.j2",
        include_str!("prompts/partials/user_rules.j2"),
    )
    .expect("user_rules partial parses");
    env.add_template(
        "partials/tool_aliases.j2",
        include_str!("prompts/partials/tool_aliases.j2"),
    )
    .expect("tool_aliases partial parses");
    env.add_template(
        "partials/footer.j2",
        include_str!("prompts/partials/footer.j2"),
    )
    .expect("footer partial parses");
    env.add_template(
        "partials/thinking_language.j2",
        include_str!("prompts/partials/thinking_language.j2"),
    )
    .expect("thinking_language partial parses");
    env.add_template(
        "partials/plan_mode.j2",
        include_str!("prompts/partials/plan_mode.j2"),
    )
    .expect("plan_mode partial parses");
    env.add_template(
        "commands/init_project.j2",
        include_str!("prompts/commands/init_project.j2"),
    )
    .expect("init_project command template parses");

    // Dispatch the system prompt by substring-matching the model id (mirrors opencode's
    // `packages/opencode/src/session/system.ts::provider`). OpenRouter paths such as
    // `anthropic/claude-3.5-sonnet` / `google/gemini-2.5-flash` / `openai/gpt-4o`
    // also match correctly. If no family is recognized it falls back to default.j2, so custom model ids are safe.
    for (name, src) in [
        (
            "system/default.j2",
            include_str!("prompts/system/default.j2") as &str,
        ),
        (
            "system/anthropic.j2",
            include_str!("prompts/system/anthropic.j2"),
        ),
        ("system/gpt.j2", include_str!("prompts/system/gpt.j2")),
        ("system/beast.j2", include_str!("prompts/system/beast.j2")),
        ("system/codex.j2", include_str!("prompts/system/codex.j2")),
        ("system/gemini.j2", include_str!("prompts/system/gemini.j2")),
        ("system/kimi.j2", include_str!("prompts/system/kimi.j2")),
        (
            "system/trinity.j2",
            include_str!("prompts/system/trinity.j2"),
        ),
    ] {
        env.add_template(name, src)
            .unwrap_or_else(|e| panic!("template {name} parses: {e}"));
    }

    env
}

fn env() -> &'static Environment<'static> {
    ENV.get_or_init(build_env)
}

// ---------------------------------------------------------------------------
// Template selection
// ---------------------------------------------------------------------------

/// Selects a template by substring-matching the model id (mirrors opencode's
/// `packages/opencode/src/session/system.ts::provider`).
///
/// Matching rules (order-sensitive, first match wins):
/// - `gpt-4` / `o1` / `o3` / `o4` → beast (strong autonomy + sequential thinking)
/// - other `gpt` containing `codex` → codex (apply_file_diffs + strict final answer formatting)
/// - other `gpt` → gpt (pragmatic engineer + commentary/final dual channel)
/// - `gemini-` → gemini (Core Mandates + Workflows + many examples)
/// - `claude` / `sonnet` / `opus` / `haiku` → anthropic (Claude Code style)
/// - `trinity` → trinity (one-tool-per-message style)
/// - `kimi` → kimi (SAME language + AGENTS.md)
/// - everything else → default.j2 (fallback)
///
/// Everything is matched after lowercasing, so user casing like `GPT-4o` / `OPENAI/gpt-4o` / `Anthropic/Claude-3.5`
/// is handled. The OpenRouter `provider/model` form also matches correctly.
pub fn pick_template(model_id: &str) -> &'static str {
    let id = model_id.to_ascii_lowercase();

    if id.contains("gpt-4") || id.contains("o1") || id.contains("o3") || id.contains("o4") {
        return "system/beast.j2";
    }
    if id.contains("gpt") {
        if id.contains("codex") {
            return "system/codex.j2";
        }
        return "system/gpt.j2";
    }
    if id.contains("gemini-") {
        return "system/gemini.j2";
    }
    if id.contains("claude") || id.contains("sonnet") || id.contains("opus") || id.contains("haiku")
    {
        return "system/anthropic.j2";
    }
    if id.contains("trinity") {
        return "system/trinity.j2";
    }
    if id.contains("kimi") {
        return "system/kimi.j2";
    }
    "system/default.j2"
}

/// Extracts the model id string from an `LLMId`. For BYOP encoding it takes the model part,
/// otherwise it returns the value as-is (in theory the BYOP path only passes BYOP ids, but this is a safety fallback).
fn model_id_from_llm_id(id: &LLMId) -> String {
    if let Some((_pid, mid)) = super::llm_id::decode(id) {
        mid
    } else {
        id.as_str().to_owned()
    }
}

// ---------------------------------------------------------------------------
// AIAgentContext → flat template context
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
struct ShellCtx {
    name: String,
    version: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct OsCtx {
    platform: String,
    distribution: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct GitCtx {
    head: String,
    branch: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkillCtx {
    name: String,
    description: String,
    /// Absolute path to SKILL.md for filesystem skills; `None` for bundled skills.
    /// Bundled skills are loaded via `AIAgentInput::InvokeSkill`, not `read_skill`,
    /// so exposing `@warp-skill:<id>` here would mislead the model into calling a
    /// path that always fails the BYOP `skill_by_reference` lookup.
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProjectRuleCtx {
    path: String,
    content: String,
}

/// Zap BYOP fix for Issue #116: a flat view of the global Rules (created by the user under
/// Settings → Agents → Rules), fed to `partials/user_rules.j2` to be rendered into the system prompt.
#[derive(Debug, Serialize)]
struct UserRuleCtx {
    name: Option<String>,
    content: String,
}

#[derive(Debug, Default, Serialize)]
struct InitProjectCommandContext {
    arguments: String,
}

#[derive(Debug, Default, Serialize)]
struct PromptContext {
    cwd: Option<String>,
    shell: Option<ShellCtx>,
    os: Option<OsCtx>,
    git: Option<GitCtx>,
    skills: Vec<SkillCtx>,
    project_rules: Vec<ProjectRuleCtx>,
    /// Zap BYOP fix for Issue #116: injected by the caller (`render_system`) from
    /// `RequestParams.user_rules` and rendered via `partials/user_rules.j2`.
    user_rules: Vec<UserRuleCtx>,
    current_time: String,
    model_id: String,
    /// The list of tool names actually fed to the upstream model this turn (computed by
    /// `chat_stream::available_tool_names`, including the post-gating built-in tools and the current MCP tools).
    /// The template renders the whitelist dynamically from this instead of hardcoding it.
    available_tools: Vec<String>,
    /// Whether this turn is in the `/plan`-triggered Plan Mode (read-only research mode).
    /// Computed by `chat_stream::is_plan_mode_turn`; based on this the template includes
    /// `partials/plan_mode.j2` to inject the read-only constraints + plan-output guidance.
    plan_mode: bool,
}

fn collect_prompt_context(model_id: &str, ctx: &[AIAgentContext]) -> PromptContext {
    let mut out = PromptContext {
        // P0-1 prompt cache optimization: `current_time` is kept only at calendar-day granularity,
        // no longer down to the second. Reasons:
        // - Anything in the system prompt that changes on every request makes the hash written at
        //   Anthropic's first system breakpoint unique → invalidated as soon as it's written, never hit.
        //   The same applies to OpenAI's first-256-token routing hash, which gets scattered across machines.
        // - The model really only needs to know "what day it is today", so the one miss when crossing
        //   into a new calendar day is an acceptable cost (one day × all active conversations × system tokens).
        // - Crossing a year boundary costs the same as crossing a day, so no extra handling is needed.
        // Later we could go further and move "current time" to the end of the user message (P0-1 option C),
        // making the system segment 100% stable; for this step we take the lower-risk option B.
        current_time: Local::now().format("%Y-%m-%d").to_string(),
        model_id: model_id.to_owned(),
        ..Default::default()
    };

    for c in ctx {
        match c {
            AIAgentContext::Directory { pwd, .. } => {
                if out.cwd.is_none() {
                    out.cwd = pwd.clone();
                }
            }
            AIAgentContext::ExecutionEnvironment(exec) => {
                out.shell = Some(ShellCtx {
                    name: exec.shell_name.clone(),
                    version: exec.shell_version.clone(),
                });
                let has_os = exec.os.category.is_some() || exec.os.distribution.is_some();
                if has_os {
                    out.os = Some(OsCtx {
                        platform: exec.os.category.clone().unwrap_or_default(),
                        distribution: exec.os.distribution.clone(),
                    });
                }
            }
            AIAgentContext::CurrentTime { current_time } => {
                // P0-1: consistent with the default value, keep only calendar-day granularity.
                // Upstream Zap may pass a second-precision timestamp, so we normalize it down to "current date" here.
                out.current_time = current_time.format("%Y-%m-%d").to_string();
            }
            // Code indexing is not implemented, so Codebase context does not go into the system prompt.
            AIAgentContext::Codebase { .. } => {}
            // P1-7 prompt cache note: `Git { head, branch }` depends on the current repository state,
            // so switching branches changes the rendered system segment, invalidating the
            // system+messages cache of every upstream provider (Anthropic / OpenAI / DeepSeek).
            // This is **expected behavior**:
            //   - on a new branch the instruction model must not assume the old git context;
            //   - as a tradeoff, the user's first request on a new branch is a 100% miss that writes a new
            //     cache, after which that branch reuses it. Developers who jump between branches frequently see the most misses.
            // Alternative considered: moving the git state to the end of the user message (same as P0-1 option C),
            // but that would make the system segment lose the contextual meaning of "the model can tell the current branch at a glance",
            // degrading models that need to rely on it for reasoning. This patch keeps the status quo.
            AIAgentContext::Git { head, branch } => {
                out.git = Some(GitCtx {
                    head: head.clone(),
                    branch: branch.clone(),
                });
            }
            AIAgentContext::Skills { skills } => {
                for s in skills {
                    let path = match &s.reference {
                        ai::skills::SkillReference::Path(p) => {
                            Some(p.to_string_lossy().into_owned())
                        }
                        // Bundled skills load via InvokeSkill, not read_skill.
                        // Omit skill_path to avoid guiding the model toward a
                        // value that will always fail BYOP's skill_by_reference.
                        ai::skills::SkillReference::BundledSkillId(_) => None,
                    };
                    out.skills.push(SkillCtx {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        path,
                    });
                }
            }
            AIAgentContext::ProjectRules {
                root_path,
                active_rules,
                ..
            } => {
                use ai::agent::action_result::AnyFileContent;
                for rule in active_rules {
                    let content = match &rule.content {
                        AnyFileContent::StringContent(s) => s.clone(),
                        AnyFileContent::BinaryContent(_) => continue,
                    };
                    let path = if rule.file_name.starts_with('/') {
                        rule.file_name.clone()
                    } else {
                        format!("{root_path}/{}", rule.file_name)
                    };
                    out.project_rules.push(ProjectRuleCtx { path, content });
                }
            }
            // User attachment context (File / Image / SelectedText / Block) does not go into the system prompt;
            // it is injected into the current turn's user message by `user_context::render_user_attachments`
            // in chat_stream's UserQuery branch. This aligns with the semantics of warp's own path, which splits into two kinds:
            // - environment type → InputContext.{directory,shell,git,...} → backend injects into the system area
            // - attachment type → InputContext.{executed_shell_commands,selected_text,files,images}
            //            → backend injects into the user area
            AIAgentContext::File(_)
            | AIAgentContext::Image(_)
            | AIAgentContext::SelectedText(_)
            | AIAgentContext::Block(_) => {}
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn render_init_project_command(arguments: Option<&str>) -> String {
    let arguments = arguments
        .map(str::trim)
        .filter(|arguments| !arguments.is_empty())
        .unwrap_or("(none)")
        .to_owned();
    let ctx = InitProjectCommandContext { arguments };
    let env = env();
    let template_name = "commands/init_project.j2";
    let tmpl = match env.get_template(template_name) {
        Ok(t) => t,
        Err(e) => {
            log::error!("[byop prompt] failed to get template {template_name}: {e}");
            return fallback_init_project_command(&ctx.arguments);
        }
    };
    match tmpl.render(Value::from_serialize(&ctx)) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[byop prompt] render {template_name} failed: {e}");
            fallback_init_project_command(&ctx.arguments)
        }
    }
}

/// Renders the final system message string sent to the upstream model.
///
/// `ctx` usually comes from the most recent `AIAgentInput::UserQuery.context` in `params.input`.
/// Not getting any context (an empty array) is fine too — the template renders with default placeholders.
///
/// `available_tools` is computed by `chat_stream::available_tool_names`: the list of tool names actually
/// exposed to the upstream LLM this turn (built-in + MCP, with gating applied). The template renders the
/// whitelist dynamically from this; do not hardcode an "unavailable tools" blacklist anymore — the model
/// naturally won't call tools it can't see, whereas a textual blacklist makes the model afraid to call even genuinely available tools.
pub fn render_system(
    model: &LLMId,
    ctx: &[AIAgentContext],
    available_tools: &[String],
    plan_mode: bool,
    user_rules: &[(Option<String>, String)],
) -> String {
    let model_id = model_id_from_llm_id(model);
    let template_name = pick_template(&model_id);
    let mut prompt_ctx = collect_prompt_context(&model_id, ctx);
    prompt_ctx.available_tools = available_tools.to_vec();
    prompt_ctx.plan_mode = plan_mode;
    prompt_ctx.user_rules = user_rules
        .iter()
        .map(|(name, content)| UserRuleCtx {
            name: name.clone(),
            content: content.clone(),
        })
        .collect();

    let env = env();
    let tmpl = match env.get_template(template_name) {
        Ok(t) => t,
        Err(e) => {
            log::error!("[byop prompt] failed to get template {template_name}: {e}");
            return fallback_system(&model_id);
        }
    };
    match tmpl.render(Value::from_serialize(&prompt_ctx)) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[byop prompt] render {template_name} failed: {e}");
            fallback_system(&model_id)
        }
    }
}

fn fallback_init_project_command(arguments: &str) -> String {
    format!(
        "Create or update `AGENTS.md` for this repository.\n\nUser-provided focus or constraints (honor these):\n{arguments}"
    )
}

/// Renders the fallback system prompt (used only when template loading/rendering fails; should not be triggered on the normal path).
fn fallback_system(model_id: &str) -> String {
    format!(
        "You are the AI coding agent inside Zap, an AI Development Environment (ADE). \
         Model: {model_id}. \
         Use the registered tools (run_shell_command / read_files / apply_file_diffs / grep / file_glob / ...) \
         to take actions on the user's behalf. Be concise."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent::AIAgentContext;
    use crate::ai_assistant::execution_context::{WarpAiExecutionContext, WarpAiOsContext};

    #[test]
    fn render_init_project_command_uses_command_template_arguments() {
        let out = render_init_project_command(Some("focus on test commands"));
        assert!(out.contains("Create or update `AGENTS.md`"), "{out}");
        assert!(out.contains("focus on test commands"), "{out}");
        assert!(out.contains("## Writing rules"), "{out}");
    }

    #[test]
    fn pick_template_dispatches_by_model_family() {
        // Direct-connection form
        for (id, want) in [
            ("claude-sonnet-4-5", "system/anthropic.j2"),
            ("claude-opus-4-1", "system/anthropic.j2"),
            ("haiku-3-5", "system/anthropic.j2"),
            ("gpt-4o", "system/beast.j2"),
            ("gpt-4-turbo", "system/beast.j2"),
            ("o1-preview", "system/beast.j2"),
            ("o3-mini", "system/beast.j2"),
            ("o4-mini", "system/beast.j2"),
            ("gpt-5-codex", "system/codex.j2"),
            ("gpt-3.5-turbo", "system/gpt.j2"),
            ("gemini-2.0-flash", "system/gemini.j2"),
            ("gemini-2.5-pro", "system/gemini.j2"),
            ("kimi-k2", "system/kimi.j2"),
            ("trinity-v1", "system/trinity.j2"),
            // Fallback
            ("deepseek-chat", "system/default.j2"),
            ("qwen2.5-coder", "system/default.j2"),
            ("glm-4", "system/default.j2"),
            ("my-custom-model", "system/default.j2"),
            ("", "system/default.j2"),
        ] {
            assert_eq!(pick_template(id), want, "id={id}");
        }
    }

    #[test]
    fn pick_template_handles_openrouter_path_form() {
        // OpenRouter `provider/model` form; substring matching still hits the correct family
        for (id, want) in [
            ("anthropic/claude-3.5-sonnet", "system/anthropic.j2"),
            ("anthropic/claude-opus-4", "system/anthropic.j2"),
            ("openai/gpt-4o", "system/beast.j2"),
            ("openai/gpt-5-codex", "system/codex.j2"),
            ("openai/o1-preview", "system/beast.j2"),
            ("google/gemini-2.5-flash", "system/gemini.j2"),
            ("moonshot/kimi-k2", "system/kimi.j2"),
        ] {
            assert_eq!(pick_template(id), want, "id={id}");
        }
    }

    #[test]
    fn pick_template_is_case_insensitive() {
        for (id, want) in [
            ("Claude-Sonnet-4", "system/anthropic.j2"),
            ("GPT-4o", "system/beast.j2"),
            ("Gemini-2.5-Pro", "system/gemini.j2"),
            ("KIMI-K2", "system/kimi.j2"),
            ("Anthropic/Claude-3.5", "system/anthropic.j2"),
        ] {
            assert_eq!(pick_template(id), want, "id={id}");
        }
    }

    #[test]
    fn render_includes_env_block_with_cwd_and_shell() {
        let ctx = vec![
            AIAgentContext::Directory {
                pwd: Some("/home/user/project".into()),
                home_dir: Some("/home/user".into()),
                are_file_symbols_indexed: false,
            },
            AIAgentContext::ExecutionEnvironment(WarpAiExecutionContext {
                os: WarpAiOsContext {
                    category: Some("linux".into()),
                    distribution: Some("Ubuntu 22.04".into()),
                },
                shell_name: "bash".into(),
                shell_version: Some("5.1".into()),
            }),
        ];
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &ctx, &[], false, &[]);
        assert!(
            out.contains("Working directory: /home/user/project"),
            "{out}"
        );
        assert!(out.contains("Shell: bash 5.1"), "{out}");
        assert!(out.contains("linux (Ubuntu 22.04)"), "{out}");
        // The home field was dropped to align with opencode and is no longer rendered
        assert!(!out.contains("Home directory:"), "{out}");
    }

    #[test]
    fn render_produces_non_empty_for_all_families() {
        // Any model id should render a non-empty string (containing Zap's self-identification).
        for id in [
            "claude-sonnet-4-5",
            "gpt-4o",
            "gpt-5-codex",
            "gemini-2.5-pro",
            "kimi-k2",
            "trinity-v1",
            "deepseek-chat",
            "weird-model",
        ] {
            let out = render_system(
                &LLMId::from(format!("byop:p:{id}").as_str()),
                &[],
                &[],
                false,
                &[],
            );
            assert!(
                out.contains("Zap"),
                "id={id} should mention Zap, got: {out}"
            );
        }
    }

    #[test]
    fn render_omits_skills_block_when_empty() {
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &[], &[], false, &[]);
        // When there are no skills, the skills block should not appear
        assert!(
            !out.contains("Skills provide specialized instructions"),
            "{out}"
        );
    }

    /// Issue #169 regression: the skill block in the system prompt must contain skill_path (an absolute path),
    /// not just name/description, otherwise the model cannot call the read_skill tool correctly.
    #[test]
    fn render_includes_skill_path_for_read_skill_tool() {
        use crate::ai::skills::SkillDescriptor;
        use ai::skills::{SkillProvider, SkillReference, SkillScope};

        let skill_path = "/home/user/.agents/skills/open-browser-use/SKILL.md";
        let skill = SkillDescriptor {
            reference: SkillReference::Path(skill_path.into()),
            name: "open-browser-use".into(),
            description: "Automates Chrome browser operations.".into(),
            scope: SkillScope::Project,
            provider: SkillProvider::Agents,
            icon_override: None,
        };
        let ctx = vec![AIAgentContext::Skills {
            skills: vec![skill],
        }];
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &ctx, &[], false, &[]);
        assert!(
            out.contains(skill_path),
            "system prompt must expose the skill_path so the model can pass it to read_skill; got: {out}"
        );
    }

    /// Issue #169 follow-up: a bundled skill's BundledSkillId variant cannot be loaded via read_skill
    /// on the BYOP path (it goes through InvokeSkill), so the system prompt should not emit <skill_path>,
    /// to avoid the model using an @warp-skill:{id} value that is guaranteed to fail.
    #[test]
    fn render_omits_skill_path_for_bundled_skill() {
        use crate::ai::skills::SkillDescriptor;
        use ai::skills::{SkillProvider, SkillReference, SkillScope};
        use warp_core::ui::icons::Icon;

        let skill = SkillDescriptor {
            reference: SkillReference::BundledSkillId("find-skills".into()),
            name: "find-skills".into(),
            description: "Help discover and install new agent skills.".into(),
            scope: SkillScope::Bundled,
            provider: SkillProvider::Zap,
            icon_override: Some(Icon::WarpLogoLight),
        };
        let ctx = vec![AIAgentContext::Skills {
            skills: vec![skill],
        }];
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &ctx, &[], false, &[]);
        assert!(
            out.contains("find-skills"),
            "bundled skill name should still appear in prompt: {out}"
        );
        assert!(
            !out.contains("@warp-skill:"),
            "bundled skill must NOT emit <skill_path> to avoid misleading the model: {out}"
        );
        assert!(
            !out.contains("<skill_path>"),
            "no <skill_path> tag should be rendered for bundled skills: {out}"
        );
    }

    #[test]
    fn fallback_does_not_panic() {
        // render_system never panics; on failure it falls back to fallback_system
        let out = render_system(&LLMId::from("byop:p:any"), &[], &[], false, &[]);
        assert!(!out.is_empty());
    }

    #[test]
    fn render_lists_available_tools_dynamically() {
        // The tool names passed in must appear in the system prompt (dynamic whitelist)
        let tools: Vec<String> = vec![
            "run_shell_command".into(),
            "webfetch".into(),
            "websearch".into(),
            "mcp__github__create_issue".into(),
        ];
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &[], &tools, false, &[]);
        for name in &tools {
            assert!(
                out.contains(name),
                "expected `{name}` in prompt, got: {out}"
            );
        }
        // The old blacklist wording should no longer appear
        assert!(
            !out.contains("Do not call unavailable tools"),
            "blacklist section has been removed: {out}"
        );
    }

    #[test]
    fn render_omits_tool_list_when_empty() {
        // tool_names is empty (shouldn't happen in theory; fallback: don't render the whitelist section)
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &[], &[], false, &[]);
        assert!(!out.contains("Available Tools"), "{out}");
    }

    #[test]
    fn plan_mode_off_omits_plan_block() {
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &[], &[], false, &[]);
        assert!(
            !out.contains("Plan Mode (Read-Only)"),
            "plan_mode=false should not contain Plan Mode section: {out}"
        );
    }

    #[test]
    fn plan_mode_on_injects_plan_block_for_all_families() {
        for id in [
            "claude-sonnet-4-5",
            "gpt-4o",
            "gpt-5-codex",
            "gemini-2.5-pro",
            "kimi-k2",
            "trinity-v1",
            "deepseek-chat",
            "weird-model",
        ] {
            let out = render_system(
                &LLMId::from(format!("byop:p:{id}").as_str()),
                &[],
                &[],
                true,
                &[],
            );
            assert!(
                out.contains("Plan Mode (Read-Only)"),
                "id={id} plan_mode=true should contain Plan Mode section: {out}"
            );
            assert!(
                out.contains("Stop and wait"),
                "id={id} plan_mode=true should contain Stop and wait guidance: {out}"
            );
        }
    }

    // Issue #116: the global Rules (created by the user under Settings → Agents → Rules) must be injected into the system prompt.
    // The three test cases below cover the key branches of `partials/user_rules.j2`.

    #[test]
    fn render_omits_user_rules_block_when_empty() {
        let out = render_system(&LLMId::from("byop:p:deepseek-chat"), &[], &[], false, &[]);
        assert!(
            !out.contains("# User rules"),
            "when user_rules is empty, user rules section should not be rendered: {out}"
        );
    }

    #[test]
    fn render_includes_user_rules_when_present() {
        let rules = vec![(
            Some("My rule".to_string()),
            "Always use snake_case in Rust.".to_string(),
        )];
        let out = render_system(
            &LLMId::from("byop:p:deepseek-chat"),
            &[],
            &[],
            false,
            &rules,
        );
        assert!(out.contains("# User rules"), "should render user rules section: {out}");
        assert!(out.contains("## My rule"), "should contain rule name: {out}");
        assert!(
            out.contains("Always use snake_case in Rust."),
            "should contain rule content: {out}"
        );
    }

    #[test]
    fn render_includes_user_rules_across_all_template_families() {
        // user_rules.j2 is injected via footer.j2, and every system template family references footer.
        // This regression test ensures that any of the anthropic / beast / codex / gemini / kimi / trinity /
        // default template families renders user rules, so none of them misses the injection by not pulling in footer.
        let rules = vec![(Some("family coverage".to_string()), "snake_case only.".to_string())];
        for id in [
            "claude-sonnet-4-5",
            "gpt-4o",
            "gpt-5-codex",
            "gemini-2.5-pro",
            "kimi-k2",
            "trinity-v1",
            "deepseek-chat",
            "weird-model",
        ] {
            let out = render_system(
                &LLMId::from(format!("byop:p:{id}").as_str()),
                &[],
                &[],
                false,
                &rules,
            );
            assert!(
                out.contains("snake_case only."),
                "id={id} should contain user rule content: {out}"
            );
        }
    }

    #[test]
    fn render_user_rules_separates_multiple_rules_with_blank_line() {
        // Multiple rules should be separated by a blank line (`{% if not loop.last %}`), with no blank line after the last one.
        let rules = vec![
            (Some("R1".to_string()), "first content".to_string()),
            (Some("R2".to_string()), "second content".to_string()),
            (Some("R3".to_string()), "third content".to_string()),
        ];
        let out = render_system(
            &LLMId::from("byop:p:deepseek-chat"),
            &[],
            &[],
            false,
            &rules,
        );

        // Between two rules there should be at least one "blank line" (two adjacent newlines).
        // We don't hardcode the exact number of newlines, because the count determined by minijinja's
        // default trim_blocks/lstrip_blocks behavior easily changes with minor template tweaks (a reviewer
        // actually observed a 3-newline shape). The contract we want is "a visual blank line + correct order".
        let pos_r1 = out.find("first content").expect("could not find R1 content");
        let pos_r2 = out.find("## R2").expect("could not find R2 heading");
        let pos_r3 = out.find("## R3").expect("could not find R3 heading");
        assert!(pos_r1 < pos_r2 && pos_r2 < pos_r3, "order should be maintained: {out}");
        let between_r1_r2 = &out[pos_r1 + "first content".len()..pos_r2];
        let between_r2_r3 = &out[pos_r2..pos_r3];
        assert!(
            between_r1_r2.contains("\n\n"),
            "there should be a blank line between R1 and R2, actual: {between_r1_r2:?}"
        );
        assert!(
            between_r2_r3.contains("\n\n"),
            "there should be a blank line between R2 and R3, actual: {between_r2_r3:?}"
        );
    }

    #[test]
    fn render_user_rules_handles_no_name() {
        let rules = vec![(None, "Be terse.".to_string())];
        let out = render_system(
            &LLMId::from("byop:p:deepseek-chat"),
            &[],
            &[],
            false,
            &rules,
        );
        assert!(out.contains("# User rules"), "{out}");
        assert!(out.contains("Be terse."), "{out}");
        // When there is no name, an empty `## ` heading line should not be rendered
        assert!(
            !out.contains("## \n"),
            "when there is no name, empty '## ' heading should not be rendered: {out}"
        );
    }

    #[test]
    fn render_includes_thinking_language_across_all_template_families() {
        // thinking_language.j2 is injected via footer.j2, and every system template family references footer.
        // This regression test ensures all 8 template families render thinking_language, so none of them misses
        // the injection by not pulling in footer, which would make the LLM still think in English when the user asks in Chinese.
        // The 8 families correspond to: anthropic / gpt / beast / codex / gemini / kimi / trinity / default
        for id in [
            "claude-sonnet-4-5",
            "gpt-3.5-turbo",
            "gpt-4o",
            "gpt-5-codex",
            "gemini-2.5-pro",
            "kimi-k2",
            "trinity-v1",
            "weird-model",
        ] {
            let out = render_system(
                &LLMId::from(format!("byop:p:{id}").as_str()),
                &[],
                &[],
                false,
                &[],
            );
            assert!(
                out.contains("# Thinking language"),
                "id={id} should render thinking_language section: {out}"
            );
            assert!(
                out.contains("internal reasoning"),
                "id={id} should contain thinking_language anchor: {out}"
            );
        }
    }

    #[test]
    fn render_thinking_language_precedes_tool_aliases() {
        // The meta-rule should come before the tool list and not be overridden by user_rules / project_rules.
        // A non-empty tool list must be passed, otherwise the whole tool_aliases.j2 block is skipped by {% if available_tools %}.
        let tools = vec!["read_files".to_string()];
        let out = render_system(
            &LLMId::from("byop:p:claude-sonnet-4-5"),
            &[],
            &tools,
            false,
            &[],
        );
        let pos_thinking = out
            .find("# Thinking language")
            .expect("should contain thinking_language");
        let pos_tools = out
            .find("# Available Tools")
            .expect("should contain tool_aliases");
        assert!(
            pos_thinking < pos_tools,
            "thinking_language should come before tool_aliases: thinking={pos_thinking}, tools={pos_tools}\n{out}"
        );
    }
}
