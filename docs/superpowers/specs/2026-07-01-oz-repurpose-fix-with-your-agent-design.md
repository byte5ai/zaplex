# Oz Repurpose — "Fix with <your agent>" Design

> Oz (Warp's built-in MAA agent) is being removed as a *distinct in-app agent* and
> de-branded (done). But its genuinely useful **contextual one-shot actions** —
> "here's an error/some context, fix/explain it" — should be **kept and routed to the
> user's own CLI agent** (Claude Code / Codex / Gemini / …), not to an in-app agent.
> That's the repurpose: zaplex supplies the *context and the button*; the user's
> chosen agent does the work. See [[self-contained-audit-findings]],
> [[branding-warpify-to-zaplexify]], [[filemanager-pane-mode-design]] (same
> "pane/connection" spirit), [[no-quick-wins-sustainable-only]].
>
> Grounded in the real mechanism: `CLIAgent` (`app/src/terminal/cli_agent.rs:134`,
> 14 agents) with `command_prefix()` (:154) and `is_cli_agent_installed()` (:615,
> backed by a background install scan, `CLIAgentInstallEvent`); tab launch via
> `Workspace::add_tab_with_specific_agent(agent)` (`app/src/workspace/view.rs:3879`,
> runs `execute_command_or_set_pending(agent.command_prefix())`); the in-app
> agent-mode path `PaneGroup::add_terminal_pane_in_agent_mode(initial_query, …)`
> (`app/src/pane_group/mod.rs:6014`, which *prefills* the input); and the existing
> per-CLI harness command-builders (`app/src/ai/agent_sdk/driver/harness/` —
> claude_code, gemini).

## 1. Goal & Non-goals

**Goal.** Preserve the value of Oz's contextual actions by sending their context as a
prompt to **the user's installed CLI agent**:
1. **Flagship (P1):** the invalid-`settings.toml` banner's action (today
   `FixSettingsWithOz`, button labelled "Fix with AI") launches the user's CLI agent
   with a "fix this settings error" prompt instead of the in-app agent-mode.
2. A reusable **"ask my agent about this context"** primitive that other call sites
   (command errors, command output) can adopt later.

**Non-goals.**
- Not reviving/branding an in-app agent. The Oz/agent-mode infra stays flag-off and
  preserved-as-template (it is *not* the target of these actions anymore).
- No new agent runtime — we shell out to the user's own CLI, which is the sanctioned
  external dependency.
- Not touching the flag-gated agent-mode "Fix with"/permission prompts (they live in
  the preserved template and aren't release-visible).

## 2. The mechanism (verified)

- **Which agents exist / are installed:** iterate `CLIAgent` variants, filter by
  `is_cli_agent_installed(agent)`. Each has `command_prefix()` (e.g. `claude`,
  `codex`, `gemini`).
- **Launching one:** `add_tab_with_specific_agent(agent)` opens a terminal tab and
  runs the agent's command. It currently takes **no prompt**.
- **Delivering the prompt — two options:**
  - **(A) Prefill (recommended for P1):** open the agent, then place the prompt in
    the input so the user reviews and presses Enter. Uniform across all 14 agents (no
    per-CLI flag knowledge), keeps the human in the loop, and mirrors the existing
    `add_terminal_pane_in_agent_mode(initial_query)` prefill pattern.
  - **(B) One-shot arg (later):** `claude -p "<prompt>"` / `codex "<prompt>"` etc.
    Faster but the exact flag differs per CLI — best done through the harness
    command-builders that already encode this, and only for agents that support it.

## 3. Agent selection UX

Given the set of installed agents `A`:
- **|A| == 0** → don't silently fall back to the in-app agent. Show a small hint /
  action: "Install a coding agent (Claude Code, Codex, …)" linking to setup. (Reuse
  the existing install affordance that the `+` "Coding Agents" menu already uses.)
- **|A| == 1** → button reads **"Fix with <Agent>"** (e.g. "Fix with Claude"); one
  click launches it with the prompt.
- **|A| > 1** → button reads **"Fix with AI ▾"** and opens a small picker listing the
  installed agents ("Fix with Claude / Codex / Gemini"). Selecting one launches it.
- **Optional (P3):** a `default_coding_agent` setting to skip the picker; when set,
  the button uses it directly and the picker is available via a caret.

There is no "preferred coding agent" setting today (agents are chosen per-tab), so P1
uses the installed-set rule above; P3 may add the setting.

## 4. Design of the reusable primitive

Introduce one workspace action that supersedes `FixSettingsWithOz`:

```
AskAgent {
    prompt: String,          // the fully-formed instruction incl. the context
    agent: Option<CLIAgent>, // None → resolve via the selection rule (§3)
    label_hint: AskAgentKind // Settings | CommandError | Explain | … (for analytics/UX)
}
```

Handler: resolve `agent` (or run the picker), then open a tab with that agent and
**prefill** the prompt (option A). `FixSettingsWithOz` becomes a thin caller that
builds the settings prompt and dispatches `AskAgent`. This keeps the existing button
wiring (`settings_view/settings_file_footer.rs`, `workspace/view.rs`) and just swaps
the target from agent-mode to the user's CLI.

Prompt for the flagship (unchanged wording, retargeted):
`My settings.toml file has an error: {error_description}. Please fix it.`

## 5. Phasing

- **P1 — Flagship.** Repurpose `FixSettingsWithOz` → `AskAgent` (settings prompt),
  installed-set selection (0/1/many), prefill delivery, dynamic button label. Remove
  the last dependence of a release-visible action on the in-app agent-mode.
- **P2 — Generalize.** Adopt `AskAgent` for command-error / block-output context
  ("Fix this command", "Explain this output") via right-click / block actions.
- **P3 — Polish.** One-shot delivery via harness for agents that support it;
  `default_coding_agent` setting; support launching the agent against the **active
  remote host** (tie-in with the daemon session layer) rather than always local.

## 6. Testability & verification

- Agent-selection logic (0/1/many, label text) is pure → unit-testable with a mocked
  installed-set.
- The launch/prefill path touches terminal-tab creation + input prefill and needs a
  **visual test build** to confirm timing (the prompt must land in the agent's input
  after the CLI starts). So P1 lands together with a test DMG, not blind.

## Appendix — code anchors
- `app/src/terminal/cli_agent.rs` — `CLIAgent` (:134), `command_prefix` (:154),
  `is_cli_agent_installed` (:615), `CLIAgentInstallEvent` (:577).
- `app/src/workspace/view.rs` — `add_tab_with_specific_agent` (:3879),
  `FixSettingsWithOz` handler (:19183), banner button (:17653).
- `app/src/workspace/action.rs` — `AddSpecificAgentTab` (:162), `FixSettingsWithOz`
  (:622).
- `app/src/pane_group/mod.rs` — `add_terminal_pane_in_agent_mode` (:6014, prefill).
- `app/src/settings_view/settings_file_footer.rs` — the button (:252).
- `app/src/ai/agent_sdk/driver/harness/` — per-CLI command builders (claude_code, gemini).
