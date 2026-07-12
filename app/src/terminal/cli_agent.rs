//! CLI agent detection and configuration.
//!
//! This module provides types for detecting and working with CLI-based AI agents
//! like Claude Code, Gemini CLI, Codex, Amp, and Droid.

use ai::skills::SkillProvider;
use enum_iterator::Sequence;
use markdown_parser::parse_markdown;
use pathfinder_color::ColorU;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(unix)]
use std::collections::HashSet;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use warp_editor::content::{buffer::Buffer, markdown::MarkdownStyle};

use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::agent::{AgentReviewCommentBatch, DiffSetHunk};
use crate::ai::blocklist::CLAUDE_ORANGE;
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::AttachedReviewCommentTarget;
use crate::server::telemetry::CLIAgentType;
use crate::ui_components::icons::Icon;
use crate::workspaces::user_workspaces::UserWorkspaces;
use warp_completer::parsers::simple::top_level_command;
use warp_util::path::EscapeChar;

/// UID for the Uber team.
/// See https://warp.metabaseapp.com/dashboard/1454?team_id=46347
const UBER_TEAM_UID: &str = "BdVbYjy9LRZcZrYBemSfAF";

/// Gemini brand blue color
pub(crate) const GEMINI_BLUE: ColorU = ColorU {
    r: 66,
    g: 133,
    b: 244,
    a: 255,
};

/// OpenAI brand color (dark gray/black)
const OPENAI_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};

/// Amp brand color (#F34E3F)
const AMP_COLOR: ColorU = ColorU {
    r: 243,
    g: 78,
    b: 63,
    a: 255,
};

/// Droid brand color (white)
const DROID_COLOR: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// OpenCode brand color (gray, used for contrast calculation only)
const OPENCODE_COLOR: ColorU = ColorU {
    r: 128,
    g: 128,
    b: 128,
    a: 255,
};

/// Copilot brand color (Copilot purple selected from https://brand.github.com/brand-identity/copilot)
const COPILOT_COLOR: ColorU = ColorU {
    r: 133,
    g: 52,
    b: 243,
    a: 255,
};

/// Pi brand color (white, monochrome logo)
const PI_COLOR: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Auggie brand color (white, monochrome logo)
const AUGGIE_COLOR: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Cursor brand color (#26251E, from official brand assets)
const CURSOR_COLOR: ColorU = ColorU {
    r: 38,
    g: 37,
    b: 30,
    a: 255,
};

/// Antigravity brand color (#7C3AED, purple from official banner accent)
const ANTIGRAVITY_PURPLE: ColorU = ColorU {
    r: 0x7C,
    g: 0x3A,
    b: 0xED,
    a: 255,
};

/// Goose brand color (#101010, from Block's official Goose logo)
const DEEPSEEK_COLOR: ColorU = ColorU {
    r: 53,
    g: 120,
    b: 229,
    a: 255,
};

const GOOSE_COLOR: ColorU = ColorU {
    r: 16,
    g: 16,
    b: 16,
    a: 255,
};

/// Represents a CLI agent (e.g., Claude Code, Gemini CLI, Codex, Amp, Droid, OpenCode, Copilot, Pi, Auggie, Cursor, Goose)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Sequence, Serialize, Deserialize)]
pub enum CLIAgent {
    Claude,
    Gemini,
    Codex,
    Amp,
    Droid,
    OpenCode,
    Copilot,
    Pi,
    Auggie,
    CursorCli,
    Goose,
    DeepSeek,
    Antigravity,
    /// Represents an unknown/custom CLI agent matched by user-configured regex patterns.
    Unknown,
}

impl CLIAgent {
    /// The command prefix used to invoke this CLI agent.
    pub fn command_prefix(&self) -> &'static str {
        match self {
            CLIAgent::Claude => "claude",
            CLIAgent::Gemini => "gemini",
            CLIAgent::Codex => "codex",
            CLIAgent::Amp => "amp",
            CLIAgent::Droid => "droid",
            CLIAgent::OpenCode => "opencode",
            CLIAgent::Copilot => "copilot",
            CLIAgent::Pi => "pi",
            CLIAgent::Auggie => "auggie",
            CLIAgent::CursorCli => "agent",
            CLIAgent::Goose => "goose",
            CLIAgent::DeepSeek => "deepseek",
            CLIAgent::Antigravity => "agy",
            CLIAgent::Unknown => "",
        }
    }

    fn command_prefix_aliases(&self) -> &'static [&'static str] {
        match self {
            CLIAgent::DeepSeek => &["deepseek-tui"],
            _ => &[],
        }
    }

    fn matches_command_prefix(&self, command: &str) -> bool {
        command == self.command_prefix() || self.command_prefix_aliases().contains(&command)
    }

    /// The shell command that forks an existing conversation into a **new**
    /// session — same history, divergent future; the original session stays
    /// untouched (fork/worktree design §2).
    ///
    /// Verified against the current CLIs (2026-07-03):
    /// `claude --resume <id> --fork-session` and `codex fork <id>`.
    /// `None` = this agent has no known fork mechanism; surfaces stay
    /// visibly disabled rather than guessing (no fake fork).
    pub fn fork_command(&self, session_id: &str) -> Option<String> {
        // Ids come from the providers' own registries (UUIDs), but they end
        // up on a shell command line — quote defensively.
        let id = shell_words::quote(session_id).into_owned();
        match self {
            CLIAgent::Claude => Some(format!("claude --resume {id} --fork-session")),
            CLIAgent::Codex => Some(format!("codex fork {id}")),
            _ => None,
        }
    }

    /// [`Self::fork_command`] with account pinning: a non-default account's
    /// config dir is prepended as an inline env assignment
    /// (`CLAUDE_CONFIG_DIR=… claude …` / `CODEX_HOME=… codex …`), so the fork
    /// runs on the same subscription as the source session. Inline env is used
    /// (not per-launch env injection) so the same string works verbatim in
    /// local tabs, worktree tab-configs, and daemon `startup_command`s — and
    /// the pinning stays visible in the block.
    pub fn fork_command_pinned(
        &self,
        session_id: &str,
        config_dir: Option<&Path>,
    ) -> Option<String> {
        Some(self.pin_config_dir(self.fork_command(session_id)?, config_dir))
    }

    /// The shell command that **resumes an existing conversation in place** —
    /// same session, same history, continues where it left off (no fork, no new
    /// session). This is how an idle CLI session surfaced by the cockpit is
    /// *adopted* into a live pane: "open = focus/adopt" (audit (b)#13, (c)#4).
    ///
    /// Verified against the current CLIs (2026-07-05):
    /// `claude --resume <id>` and `codex resume <id>`.
    /// `None` = this agent has no known resume mechanism; surfaces stay
    /// visibly disabled rather than guessing.
    pub fn resume_command(&self, session_id: &str) -> Option<String> {
        // Ids come from the providers' own registries (UUIDs) but land on a
        // shell command line — quote defensively.
        let id = shell_words::quote(session_id).into_owned();
        match self {
            CLIAgent::Claude => Some(format!("claude --resume {id}")),
            CLIAgent::Codex => Some(format!("codex resume {id}")),
            _ => None,
        }
    }

    /// [`Self::resume_command`] with account pinning (see
    /// [`Self::fork_command_pinned`] for the inline-env rationale), so the
    /// adopted session resumes on its original subscription.
    pub fn resume_command_pinned(
        &self,
        session_id: &str,
        config_dir: Option<&Path>,
    ) -> Option<String> {
        Some(self.pin_config_dir(self.resume_command(session_id)?, config_dir))
    }

    /// Prepend an account's config dir as an inline env assignment
    /// (`CLAUDE_CONFIG_DIR=… <cmd>` / `CODEX_HOME=… <cmd>`) so `<cmd>` runs on a
    /// specific subscription. Inline env (not per-launch injection) keeps the
    /// same string working verbatim in local tabs, worktree tab-configs, and
    /// daemon `startup_command`s — and keeps the pin visible in the block.
    /// Shared by fork/resume pinning. No-op when `config_dir` is `None` or the
    /// agent has no config-dir model.
    fn pin_config_dir(&self, cmd: String, config_dir: Option<&Path>) -> String {
        let Some(dir) = config_dir else {
            return cmd;
        };
        let var = match self {
            CLIAgent::Claude => "CLAUDE_CONFIG_DIR",
            CLIAgent::Codex => "CODEX_HOME",
            _ => return cmd,
        };
        let dir = shell_words::quote(&dir.to_string_lossy()).into_owned();
        format!("{var}={dir} {cmd}")
    }

    /// The **subscription-routed launch command** (C4): start this agent *fresh*,
    /// authenticated via a *subscription* rather than a pay-per-token API key —
    /// the plexing model. Two parts, both inline so the string works verbatim in
    /// local tabs, worktree tab-configs, and daemon `startup_command`s:
    /// 1. **API-key scrub** — `unset` any inherited key env, so the config-dir /
    ///    default login wins (a set `ANTHROPIC_API_KEY` would otherwise override
    ///    the pin, silently defeating account routing *and* subscription billing).
    /// 2. **Account pin** — `VAR=<config_dir> …` when a specific account is chosen
    ///    (`None` = the provider's default login, still scrubbed).
    ///
    /// Agents without a subscription/config-dir model launch bare (no scrub, no pin).
    pub fn launch_command_routed(&self, config_dir: Option<&Path>) -> String {
        self.launch_command_routed_with(config_dir, None, None)
    }

    /// [`Self::launch_command_routed`] extended with an explicit **model** and
    /// **thinking-effort** chosen at launch (the Spawn-Karte core: a launch must
    /// be unmistakable about *which* model + effort it starts — Haiku/Low vs a
    /// top model/Extra-High is a huge difference).
    ///
    /// Flags are appended to the same scrub+pin prefix, so the string still works
    /// verbatim in local tabs, worktree tab-configs, and daemon `startup_command`s.
    /// Provider-correct injection (verified against the current CLIs, 2026-07-06):
    /// - **Claude Code:** `--model <model>` (e.g. `opus`/`sonnet`/`haiku`). Claude
    ///   Code has **no** CLI flag for reasoning effort, so `effort` is intentionally
    ///   **not** placed on the command line here (it is recorded for the Conductor
    ///   via the launch registry, not faked as a CLI arg).
    /// - **Codex:** `--model <model>` plus reasoning effort as a config override
    ///   `-c model_reasoning_effort="<low|medium|high>"` (Codex's documented way to
    ///   set effort non-interactively; it has no dedicated `--effort` flag).
    ///
    /// `model`/`effort` `None` = today's behavior verbatim (bare routed launch).
    /// Agents without a subscription/config-dir model launch bare (no scrub, no
    /// pin, no flags), unchanged.
    pub fn launch_command_routed_with(
        &self,
        config_dir: Option<&Path>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> String {
        let cmd = self.command_prefix();
        let (dir_var, key_vars): (&str, &[&str]) = match self {
            CLIAgent::Claude => (
                "CLAUDE_CONFIG_DIR",
                &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"],
            ),
            CLIAgent::Codex => ("CODEX_HOME", &["OPENAI_API_KEY"]),
            // No known subscription/config-dir routing for other agents.
            _ => return cmd.to_string(),
        };
        // 1. Scrub inherited API-key env so the subscription authenticates.
        let mut prefix = format!("unset {}; ", key_vars.join(" "));
        // 2. Pin the chosen account's config dir, if any.
        if let Some(dir) = config_dir {
            let dir = shell_words::quote(&dir.to_string_lossy()).into_owned();
            prefix.push_str(&format!("{dir_var}={dir} "));
        }
        // 3. Append the provider-correct model/effort flags, if chosen.
        let flags = self.model_effort_flags(model, effort);
        format!("{prefix}{cmd}{flags}")
    }

    /// The provider-correct model + effort CLI flags (with a leading space each),
    /// or an empty string when neither applies. Split out so it is unit-testable
    /// in isolation and reused by any launch path. See
    /// [`Self::launch_command_routed_with`] for the per-provider rationale.
    fn model_effort_flags(&self, model: Option<&str>, effort: Option<&str>) -> String {
        let mut flags = String::new();
        match self {
            CLIAgent::Claude => {
                if let Some(model) = model {
                    let model = shell_words::quote(model).into_owned();
                    flags.push_str(&format!(" --model {model}"));
                }
                // Claude Code has no reasoning-effort CLI flag: effort is tracked,
                // not placed on the command line (no fake flag).
            }
            CLIAgent::Codex => {
                if let Some(model) = model {
                    let model = shell_words::quote(model).into_owned();
                    flags.push_str(&format!(" --model {model}"));
                }
                if let Some(effort) = effort {
                    // Codex sets reasoning effort via a config override, not a
                    // dedicated flag. `-c key=value` is parsed as TOML, so the
                    // value must itself be a TOML-quoted string (a bare `high`
                    // is not valid TOML) — hence `model_reasoning_effort="high"`.
                    // The outer `shell_words::quote` then wraps the whole
                    // `key="value"` token in shell quoting so it survives as a
                    // single argv token verbatim.
                    let kv = shell_words::quote(&format!("model_reasoning_effort=\"{effort}\""))
                        .into_owned();
                    flags.push_str(&format!(" -c {kv}"));
                }
            }
            _ => {}
        }
        flags
    }

    /// Serialized version of the CLIAgent name (e.g. "Claude", "Gemini"). Used for the
    /// session-sharing protocol's opaque `cli_agent` string field.
    pub fn to_serialized_name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// Inverse of `to_serialized_name`. Falls back to `Unknown`.
    pub fn from_serialized_name(name: &str) -> CLIAgent {
        serde_json::from_value(name.into()).unwrap_or(CLIAgent::Unknown)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            CLIAgent::Claude => "Claude Code",
            CLIAgent::Gemini => "Gemini",
            CLIAgent::Codex => "Codex",
            CLIAgent::Amp => "Amp",
            CLIAgent::Droid => "Droid",
            CLIAgent::OpenCode => "OpenCode",
            CLIAgent::Copilot => "Copilot",
            CLIAgent::Pi => "Pi",
            CLIAgent::Auggie => "Auggie",
            CLIAgent::CursorCli => "Cursor",
            CLIAgent::Goose => "Goose",
            CLIAgent::DeepSeek => "DeepSeek",
            CLIAgent::Antigravity => "Antigravity",
            CLIAgent::Unknown => "CLI Agent",
        }
    }

    /// Returns the Icon for this CLI agent, or `None` for unknown/custom agents.
    pub fn icon(&self) -> Option<Icon> {
        match self {
            CLIAgent::Claude => Some(Icon::ClaudeLogo),
            CLIAgent::Gemini => Some(Icon::GeminiLogo),
            CLIAgent::Codex => Some(Icon::OpenAILogo),
            CLIAgent::Amp => Some(Icon::AmpLogo),
            CLIAgent::Droid => Some(Icon::DroidLogo),
            CLIAgent::OpenCode => Some(Icon::OpenCodeLogo),
            CLIAgent::Copilot => Some(Icon::CopilotLogo),
            CLIAgent::Pi => Some(Icon::PiLogo),
            CLIAgent::Auggie => Some(Icon::AuggieLogo),
            CLIAgent::CursorCli => Some(Icon::CursorLogo),
            CLIAgent::Goose => Some(Icon::GooseLogo),
            CLIAgent::DeepSeek => Some(Icon::DeepSeekLogo),
            CLIAgent::Antigravity => Some(Icon::AntigravityLogo),
            CLIAgent::Unknown => None,
        }
    }

    /// Returns the skill providers whose skills this CLI agent can natively interpret.
    /// When the CLI agent rich input is open, only skills from these providers are shown
    /// in the slash menu. Returns an empty slice for agents with no known skills support.
    pub fn supported_skill_providers(&self) -> &'static [SkillProvider] {
        match self {
            CLIAgent::Claude => &[SkillProvider::Claude],
            CLIAgent::Codex => &[
                SkillProvider::Agents,
                SkillProvider::Claude,
                SkillProvider::Codex,
            ],
            CLIAgent::OpenCode => &[
                SkillProvider::OpenCode,
                SkillProvider::Agents,
                SkillProvider::Claude,
            ],
            CLIAgent::Gemini => &[SkillProvider::Agents, SkillProvider::Gemini],
            CLIAgent::Amp => &[SkillProvider::Agents],
            CLIAgent::Copilot => &[SkillProvider::Agents, SkillProvider::Copilot],
            CLIAgent::Droid => &[SkillProvider::Droid, SkillProvider::Agents],
            CLIAgent::Pi => &[SkillProvider::Agents],
            CLIAgent::Auggie => &[SkillProvider::Agents],
            CLIAgent::CursorCli => &[SkillProvider::Agents],
            CLIAgent::Goose => &[SkillProvider::Agents],
            CLIAgent::DeepSeek => &[SkillProvider::Agents],
            CLIAgent::Antigravity => &[SkillProvider::Agents],
            CLIAgent::Unknown => &[],
        }
    }

    /// Returns the prefix character used for skill invocations by this CLI agent.
    /// Most agents use `/` (e.g. `/skill-name`), but Codex uses `$` (e.g. `$skill-name`).
    pub fn skill_command_prefix(&self) -> &'static str {
        match self {
            CLIAgent::Codex => "$",
            _ => "/",
        }
    }

    /// Whether this CLI agent supports the `!` bash mode prefix in the rich input.
    /// When `true`, typing `!` in the CLI agent rich input activates shell mode with
    /// decorations, completions, and error underlining.
    ///
    /// TODO(advait): Check whether Gemini, Amp, Droid, and Copilot support `!` bash
    /// mode and enable them here if so.
    pub fn supports_bash_mode(&self) -> bool {
        matches!(
            self,
            CLIAgent::Claude | CLIAgent::Codex | CLIAgent::OpenCode | CLIAgent::DeepSeek
        )
    }

    /// Returns the brand color for this CLI agent, or `None` for unknown/custom agents.
    pub fn brand_color(&self) -> Option<ColorU> {
        match self {
            CLIAgent::Claude => Some(CLAUDE_ORANGE),
            CLIAgent::Gemini => Some(GEMINI_BLUE),
            CLIAgent::Codex => Some(OPENAI_COLOR),
            CLIAgent::Amp => Some(AMP_COLOR),
            CLIAgent::Droid => Some(DROID_COLOR),
            CLIAgent::OpenCode => Some(OPENCODE_COLOR),
            CLIAgent::Copilot => Some(COPILOT_COLOR),
            CLIAgent::Pi => Some(PI_COLOR),
            CLIAgent::Auggie => Some(AUGGIE_COLOR),
            CLIAgent::CursorCli => Some(CURSOR_COLOR),
            CLIAgent::Goose => Some(GOOSE_COLOR),
            CLIAgent::DeepSeek => Some(DEEPSEEK_COLOR),
            CLIAgent::Antigravity => Some(ANTIGRAVITY_PURPLE),
            CLIAgent::Unknown => None,
        }
    }

    /// Returns the icon color to use when rendered on the brand-colored circle background.
    /// Agents with light brand colors use a dark icon for contrast.
    pub fn brand_icon_color(&self) -> ColorU {
        match self {
            CLIAgent::Pi | CLIAgent::Auggie | CLIAgent::Droid => ColorU::new(0, 0, 0, 255),
            _ => ColorU::white(),
        }
    }

    /// Extracts the first meaningful command token from a command string.
    ///
    /// When `escape_char` is provided, uses shell parsing to skip leading
    /// env-var assignments (e.g. `FOO=1 claude` → `claude`).
    /// Otherwise falls back to a simple whitespace split.
    fn extract_first_command(command: &str, escape_char: Option<EscapeChar>) -> Option<String> {
        match escape_char {
            Some(esc) => top_level_command(command, esc),
            None => command.split_whitespace().next().map(String::from),
        }
    }

    /// Detects the CLI agent from a command string.
    ///
    /// When `escape_char` is provided, full shell parsing is used to skip leading
    /// env-var assignments (e.g. `FOO=1 claude`). Otherwise falls back to a simple
    /// whitespace split.
    ///
    /// If `aliases` is provided, the first word of the command will be looked up
    /// in the alias map. If found, the alias value replaces the first word to
    /// produce the resolved command used for detection.
    ///
    /// Returns `Some(CLIAgent)` if the command matches a known CLI agent, `None` otherwise.
    pub fn detect(
        command: &str,
        escape_char: Option<EscapeChar>,
        aliases: Option<&HashMap<SmolStr, String>>,
        ctx: &AppContext,
    ) -> Option<CLIAgent> {
        let trimmed = command.trim_start();
        let first_word = Self::extract_first_command(trimmed, escape_char)?;

        // Resolve the full command through aliases. If the first word matches an
        // alias, replace it with the alias value to produce the resolved command.
        let resolved_command: Cow<'_, str> = aliases
            .and_then(|a| a.get(first_word.as_str()))
            .map(|alias_value| {
                let rest = trimmed
                    .find(first_word.as_str())
                    .map(|pos| &trimmed[pos + first_word.len()..])
                    .unwrap_or("");
                Cow::Owned(format!("{}{}", alias_value.trim(), rest))
            })
            .unwrap_or(Cow::Borrowed(trimmed));

        let resolved_first_word = Self::extract_first_command(&resolved_command, escape_char)?;

        // Check if resolved command matches any known CLI agent.
        // Also matches `aifx agent run claude` as Claude for Uber employees.
        enum_iterator::all::<CLIAgent>()
            .filter(|agent| !matches!(agent, CLIAgent::Unknown))
            .find(|agent| {
                agent.matches_command_prefix(&resolved_first_word)
                    || (matches!(agent, CLIAgent::Claude)
                        && Self::is_aifx_agent_run_claude(&resolved_command, ctx))
            })
    }

    /// Returns true if the resolved command is `aifx agent run claude` (Uber's
    /// internal wrapper around Claude) and the user is on the Uber team.
    /// We special-case this so Uber employees get the toolbar without needing
    /// to configure anything.
    fn is_aifx_agent_run_claude(resolved_command: &str, ctx: &AppContext) -> bool {
        resolved_command.starts_with("aifx agent run claude")
            && Self::is_on_uber_team(UserWorkspaces::as_ref(ctx))
    }

    fn is_on_uber_team(user_workspaces: &UserWorkspaces) -> bool {
        user_workspaces
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.teams.iter())
            .any(|team| team.uid.uid() == UBER_TEAM_UID)
    }
}

/// Builds a prompt string from a batch of code review comments suitable for
/// writing to a CLI agent's PTY.
///
/// # Location format
/// Locations use `L<line>` notation (1-indexed).
/// Line ranges are written `L<start>-L<end>` where both ends are **inclusive**.
/// Instructs the agent to run `git diff` for deleted-line context rather than
/// inlining the full diff.
pub fn build_review_prompt(review: &AgentReviewCommentBatch) -> String {
    let mut text = String::from(
        "Please address the following code review comments. \
         Run `git diff` (or `git diff HEAD`) to see the full context of any changes, \
         especially for deleted lines.\n",
    );

    for comment in &review.comments {
        if comment.outdated {
            continue;
        }
        let body = export_review_comment_for_cli_prompt(&comment.content);
        let location = match &comment.target {
            AttachedReviewCommentTarget::Line {
                absolute_file_path,
                line,
                ..
            } => {
                let path = absolute_file_path.display();
                match line {
                    EditorLineLocation::Current { line_number, .. } => {
                        let n = line_number.as_usize() + 1;
                        format!("{path} L{n}")
                    }
                    EditorLineLocation::Removed { line_number, .. } => {
                        let n = line_number.as_usize() + 1;
                        format!("{path} (deleted, was L{n} — see `git diff`)")
                    }
                    EditorLineLocation::Collapsed { line_range } => {
                        // line_range is [start, end) 0-indexed; convert to L<start>-L<end>
                        // where both start and end are 1-indexed inclusive.
                        let start = line_range.start.as_usize() + 1;
                        let end = line_range.end.as_usize();
                        format!("{path} (collapsed hunk, L{start}-L{end} — see `git diff`)")
                    }
                }
            }
            AttachedReviewCommentTarget::File { absolute_file_path } => {
                let path = absolute_file_path.display();
                let abs_str = absolute_file_path.to_string_lossy();
                let is_deleted = review.diff_set.iter().any(|(file_key, hunks)| {
                    abs_str.ends_with(file_key.as_str())
                        && !hunks.is_empty()
                        && hunks
                            .iter()
                            .all(|h| h.lines_added == 0 && h.lines_removed > 0)
                });
                if is_deleted {
                    format!("{path} (deleted file — see `git diff`)")
                } else {
                    format!("{path}")
                }
            }
            AttachedReviewCommentTarget::General => "General".to_string(),
        };
        text.push_str(&format!("\n- {location}: {body}"));
    }

    text
}

fn export_review_comment_for_cli_prompt(comment: &str) -> String {
    let mut result = parse_markdown(comment)
        .map(|parsed| {
            Buffer::export_to_markdown(
                parsed,
                None,
                MarkdownStyle::Export {
                    app_context: None,
                    should_not_escape_markdown_punctuation: true,
                },
            )
        })
        .unwrap_or_else(|_| comment.to_string());
    result.truncate(result.trim_end().len());
    result
}

/// Builds a prompt string for a single diff hunk location suitable for writing
/// to a CLI agent's PTY. Includes change stats (+N -N) and instructs the agent
/// to run `git diff` for full context.
///
/// # Location format
/// `<path> L<start>-L<end>` where `start` and `end` are 1-indexed and both
/// ends are **inclusive**.
pub fn build_diff_hunk_prompt(
    file_path: &Path,
    start_line: usize,
    end_line: usize,
    lines_added: u32,
    lines_removed: u32,
) -> String {
    let path = file_path.display();
    format!(
        "{path} L{start_line}-L{end_line} (+{lines_added} -{lines_removed}) \
         -- run `git diff` to see the full context."
    )
}

/// Builds a prompt string for a set of diff file context hunks suitable for
/// writing to a CLI agent's PTY.
///
/// # Location format
/// Each line is `<path> L<start>-L<end> (+N -N)` where `start` and `end` are
/// 1-indexed and both ends are **inclusive**.
pub fn build_diff_context_prompt(file_diffs: &HashMap<String, Vec<DiffSetHunk>>) -> String {
    let mut text = String::new();
    let mut sorted_keys: Vec<&String> = file_diffs.keys().collect();
    sorted_keys.sort();
    for file_key in sorted_keys {
        let hunks = &file_diffs[file_key];
        for hunk in hunks {
            // hunk.line_range is [start, end) 0-indexed; convert to L<start>-L<end>
            // where both start and end are 1-indexed inclusive.
            let start = hunk.line_range.start.as_usize() + 1;
            let end = hunk.line_range.end.as_usize();
            text.push_str(&format!(
                "{file_key} L{start}-L{end} (+{} -{})",
                hunk.lines_added, hunk.lines_removed,
            ));
            text.push('\n');
        }
    }
    // Remove trailing newline.
    text.truncate(text.trim_end().len());
    text
}

/// Builds a prompt for a single-line text selection suitable for writing to a CLI agent's PTY.
/// Prefixes the literal text with its file path and line number for context.
///
/// # Format
/// `<path> L<line>: <text>` where `line` is 1-indexed.
pub fn build_selection_substring_prompt(file_path: &str, line: usize, text: &str) -> String {
    format!("{file_path} L{line}: {text}")
}

/// Builds a prompt for a multi-line selection suitable for writing to a CLI agent's PTY.
/// For single-line selections, use [`build_selection_substring_prompt`] instead.
///
/// # Location format
/// `<path> L<start>-L<end>` where line numbers are 1-indexed and both ends are inclusive.
pub fn build_selection_line_range_prompt(
    file_path: &str,
    start_line: usize,
    end_line: usize,
) -> String {
    format!("{file_path} L{start_line}-L{end_line}")
}

impl From<CLIAgent> for CLIAgentType {
    fn from(agent: CLIAgent) -> Self {
        match agent {
            CLIAgent::Claude => CLIAgentType::Claude,
            CLIAgent::Gemini => CLIAgentType::Gemini,
            CLIAgent::Codex => CLIAgentType::Codex,
            CLIAgent::Amp => CLIAgentType::Amp,
            CLIAgent::Droid => CLIAgentType::Droid,
            CLIAgent::OpenCode => CLIAgentType::OpenCode,
            CLIAgent::Copilot => CLIAgentType::Copilot,
            CLIAgent::Pi => CLIAgentType::Pi,
            CLIAgent::Auggie => CLIAgentType::Auggie,
            CLIAgent::CursorCli => CLIAgentType::Cursor,
            CLIAgent::Goose => CLIAgentType::Goose,
            CLIAgent::DeepSeek => CLIAgentType::DeepSeek,
            CLIAgent::Antigravity => CLIAgentType::Antigravity,
            CLIAgent::Unknown => CLIAgentType::Unknown,
        }
    }
}

// ── CLI Agent installation status singleton model ──
// Aligned with AntivirusInfo pattern: ctx.spawn async scan → callback emit event → subscribers auto-refresh UI

/// CLI agent installation scan complete event.
pub enum CLIAgentInstallEvent {
    /// Background scan complete; installation status cache is ready.
    ScanComplete,
}

/// Singleton model tracking CLI agent installation status.
///
/// On construction, starts a background PATH scan via `ctx.spawn`, then emits
/// [`CLIAgentInstallEvent::ScanComplete`] and auto-syncs per-agent settings when done.
///
/// All UI code needing to query installation status should read via `CLIAgentInstallModel::as_ref(ctx)`
/// and subscribe to events to trigger redraws when scan completes.
pub struct CLIAgentInstallModel {
    /// None = scan not yet complete; Some = results ready.
    cache: Option<HashMap<CLIAgent, bool>>,
}

impl CLIAgentInstallModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.spawn(
            async move { scan_cli_agent_installations() },
            Self::on_scan_complete,
        );
        Self { cache: None }
    }

    fn on_scan_complete(&mut self, results: HashMap<CLIAgent, bool>, ctx: &mut ModelContext<Self>) {
        let any_installed = results.values().any(|&v| v);
        log::info!(
            "cli-agent scan complete: any_installed={any_installed}, results={results:?}"
        );
        self.cache = Some(results.clone());

        // Auto-sync to per-agent settings — but never *prune* per-agent settings
        // from a scan that found nothing. An all-false first scan is far more
        // likely a transient/env fluke than the truth, and pruning on it would
        // hide a genuinely-installed agent (S0 hardening).
        if any_installed {
            crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.sync_per_agent_from_scan(&results, ctx);
            });
        } else {
            log::warn!(
                "cli-agent scan found no agents installed; skipping per-agent settings prune"
            );
        }

        ctx.emit(CLIAgentInstallEvent::ScanComplete);
    }

    /// Query if an agent is installed. Returns false if scan not yet complete.
    pub fn is_cli_agent_installed(&self, agent: CLIAgent) -> bool {
        self.cache
            .as_ref()
            .map(|m| m.get(&agent).copied().unwrap_or(false))
            .unwrap_or(false)
    }

    /// Check if scan is complete.
    pub fn is_scan_complete(&self) -> bool {
        self.cache.is_some()
    }

    /// Get installation status snapshot. Returns None if scan not yet complete.
    pub fn snapshot(&self) -> Option<HashMap<CLIAgent, bool>> {
        self.cache.clone()
    }
}

impl Entity for CLIAgentInstallModel {
    type Event = CLIAgentInstallEvent;
}

impl SingletonEntity for CLIAgentInstallModel {}

/// Synchronous filesystem search detecting which agents are installed. Cheap
/// (a handful of `is_file` probes) — safe to call directly on the main thread as
/// a fallback when the async cache is not yet populated (see `open_spawn_card`).
#[cfg(unix)]
pub(crate) fn scan_cli_agent_installations() -> HashMap<CLIAgent, bool> {
    let search_dirs = cli_agent_search_dirs().collect::<Vec<_>>();
    let map: HashMap<CLIAgent, bool> = enum_iterator::all::<CLIAgent>()
        .filter(|a| !matches!(a, CLIAgent::Unknown))
        .map(|a| (a, cli_agent_is_on_path_with_dirs(a, &search_dirs)))
        .collect();
    // Diagnostic: makes it unambiguous whether the scan runs, what it searched,
    // and what it concluded for the two first-class agents — so a false
    // "not installed" can be traced to either a missing dir or a failed probe.
    log::info!(
        "cli-agent scan: {} search dirs; claude={} codex={}; dirs={:?}",
        search_dirs.len(),
        map.get(&CLIAgent::Claude).copied().unwrap_or(false),
        map.get(&CLIAgent::Codex).copied().unwrap_or(false),
        search_dirs,
    );
    map
}

/// Synchronous PATH search detecting which agents are installed. Safe to call
/// directly as a fallback when the async cache is not yet populated.
#[cfg(windows)]
pub(crate) fn scan_cli_agent_installations() -> HashMap<CLIAgent, bool> {
    enum_iterator::all::<CLIAgent>()
        .filter(|a| !matches!(a, CLIAgent::Unknown))
        .map(|a| (a, cli_agent_is_on_path(a)))
        .collect()
}

#[cfg(unix)]
fn cli_agent_is_on_path_with_dirs(agent: CLIAgent, search_dirs: &[PathBuf]) -> bool {
    match agent {
        CLIAgent::Unknown => false,
        CLIAgent::CursorCli => is_on_path_in_dirs("cursor-agent", search_dirs),
        CLIAgent::DeepSeek => {
            is_on_path_in_dirs("deepseek", search_dirs)
                || is_on_path_in_dirs("deepseek-tui", search_dirs)
        }
        other => is_on_path_in_dirs(other.command_prefix(), search_dirs),
    }
}

/// Inline PATH search; zero processes, zero window flashing.
#[cfg(unix)]
fn is_on_path_in_dirs(cmd: &str, search_dirs: &[PathBuf]) -> bool {
    search_dirs.iter().any(|dir| dir.join(cmd).is_file())
}

#[cfg(unix)]
fn cli_agent_search_dirs() -> impl Iterator<Item = PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path_var) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }

    extend_common_cli_dirs(&mut dirs);
    dedupe_paths(dirs).into_iter()
}

#[cfg(unix)]
fn extend_common_cli_dirs(dirs: &mut Vec<PathBuf>) {
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
    ]);

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };

    dirs.extend([
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        home.join(".local/bin"),
    ]);

    if let Ok(node_versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        dirs.extend(
            node_versions
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("bin")),
        );
    }
}

#[cfg(unix)]
fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::with_capacity(paths.len());
    let mut deduped = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

#[cfg(windows)]
fn cli_agent_is_on_path(agent: CLIAgent) -> bool {
    match agent {
        CLIAgent::Unknown => false,
        CLIAgent::CursorCli => is_on_path("cursor-agent"),
        CLIAgent::DeepSeek => is_on_path("deepseek") || is_on_path("deepseek-tui"),
        other => is_on_path(other.command_prefix()),
    }
}

#[cfg(windows)]
fn is_on_path(cmd: &str) -> bool {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
    let Ok(path_var) = std::env::var("PATH") else {
        return false;
    };
    let exts: Vec<&str> = pathext.split(';').collect();
    std::env::split_paths(&path_var).any(|dir| {
        exts.iter()
            .any(|ext| dir.join(format!("{}{}", cmd, ext)).is_file())
    })
}

#[cfg(test)]
#[path = "cli_agent_tests.rs"]
mod tests;
