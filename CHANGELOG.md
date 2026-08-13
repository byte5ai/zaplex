# Changelog

This document records key changes: the Zap/Warp release history inherited before the zaplex fork (translated from the original Chinese), followed by zaplex's own work on top of it. Only functional commits are listed; internal dev/stable rolling tags are omitted.

## [1.0.1] — unreleased

- **Cockpit:** Account-Panes mit vorhandenen Sessions stürzen beim Layout der
  virtualisierten Sitzungstabelle nicht mehr ab.
- **Sidebar:** Host-, Daemon- und Multiplexer-Zeilen reservieren die flexible
  Breite für ihre Identität; wiederholte Sekundäraktionen erscheinen als feste
  Icons mit lokalisierten Tooltips statt als platzraubende Textbuttons.
- **UI-Qualität:** Ein gemeinsamer `CompactRowAction`-Baustein, ein verbindliches
  HTML-Artefakt, Entwicklungsrichtlinien und ein billiger PR-Frühcheck schützen
  die migrierten kompakten Sidebar-Zeilen mechanisch vor Regressionen.

## [1.0.0] — 2026-07-30

- **Remote-Sitzungen:** versionsgebundener Host-Dienst, persistente PTYs,
  exaktes Reattach, Replay und idempotenter Shell-Bootstrap für Bash, fish und
  PowerShell.
- **Cockpit:** Host–Projekt–Session-Spine, mehrere Claude-/Codex-Konten,
  belastbare Nutzungs- und Kostendarstellung, explizite Loading-/Fehlerzustände
  und sichere Sitzungsaktionen.
- **SSH:** atomarer und referenzsicherer Credential-Lifecycle, OneKey-Editor
  mit Save/Discard/Cancel sowie strikte Endpunkt- und Host-Key-Prüfung.
- **Dateimanager:** MC-Tastatursteuerung, stabile Dateiidentitäten, sichere
  lokale und entfernte Operationen sowie eine gestreamte, fortsetzbare
  Transfer-Queue für lokale, entfernte und hostübergreifende Transfers.
- **Dateimanager-Sicherheit:** Remote SFTP-Mutationen behalten ihre Operation-ID
  über Verbindungsabbrüche, werden idempotent wiederaufgenommen und bewahren die
  Quelle, bis der daemonseitige Commit ausdrücklich bestätigt ist.
- **Oberfläche:** englische und deutsche Kernoberflächen, gemeinsame
  Modal-/Status-Komponenten und responsive Cockpit- und Dateimanager-Panes.
- **Markdown-Viewer:** extern geöffnete Markdown-Dokumente starten in einem
  eigenen Fenster ohne Sidebar und behalten am Dokumentende sichtbaren Abstand.
- **Agenten:** Antigravity ersetzt die eingestellte Gemini CLI; Claude Code,
  Codex, Grok und DeepSeek/CodeWhale liefern über lokale, selbst verwaltete
  Integrationen verlässliche Arbeits-, Freigabe- und Abschlusszustände.
- **Distribution:** konsistente Version `1.0.0`; das Apple-Silicon-DMG wird mit
  Developer ID signiert und über Apple notarisiert.

Work landed on top of the [Zap fork](https://github.com/zerx-lab/zap), starting from the native session-daemon merge ([PR #16](https://github.com/byte5ai/zaplex/pull/16)). Grouped by area; PR numbers are representative, not exhaustive — see `git log` for the full history.

- **Remote-session daemon**: persistent session IDs across app/SSH restarts, byte-exact replay ring buffer, multi-session support, idle-session garbage collection under a host-wide RAM ceiling (PR #16).
- **Cockpit / Conductor**: multi-account Claude + Codex discovery; real cost/heat from the Anthropic OAuth usage endpoint (#25); cross-host **Host ▸ Project ▸ Session** tree with "needs-me" bubbling (#60, #61, #78, #79); guardrails — pause/stop/kill per agent and stop-all (#83); review loop — diff → approve/redirect/commit/PR (#82); attention model with ambient dock badge + inbox (#80); spawn card with model/effort launch attribution (#81, #84); transcript viewer (parser + Markdown, `◇ log` verb) and live transcript watch (#48, #49, #63, #69); favorites and per-project grouping (#46, #67); account overrides — rename/recolor/reorder/hide (#47, #62); visual design pass unifying the cockpit language (#85); Conductor tree + favorites + spawn-card + CI gate integrated into the UX spine (#98).
- **Launch & routing**: launch-on-freest-account routing from the new-session menu, including on remote SSH hosts (C4-1…C4-4, #51–#54); session fork and fork-into-worktree — try another approach without disturbing the original session (#26).
- **File manager**: MC-style keyboard navigation + function-key bar, in-slot terminal⇄file-manager toggle (#27); F3/F4 view/edit over SSH, local and remote (#32, #33); F5/F6 copy/move across connections — local↔remote and remote↔remote via a local relay, with overwrite-conflict handling and a destination picker when more than one other pane is open (#28–#37).
- **GitHub flows**: quick-issue draft, PR review, and triage reachable from the launcher; the agent drafts the exact `gh` command, the user confirms before anything runs (C5, #45, #57, #66).
- **Host discovery**: Tailscale peers show up as ready-to-add SSH hosts; per-session RAM display in the daemon session list (#47, #68).
- **Fix/ask with your agent**: problem banners route to the user's own CLI agent instead of Zap's built-in AI ("Oz-repurpose" P1).
- **Localization**: German (`de`) locale added and completed for cockpit, sessions, launcher, SSH hosts, settings, and common UI chrome; English remains the fallback for the rest (#50, #71–#75).
- **Rebrand**: product identity renamed Zap → zaplex across the app (app name, install paths under `~/.zaplex/`, permission dialogs, panic/crash strings, CI quarantine hints).
- **CLI/UX polish**: CLI-agent detection reaches the UI reliably, self-update repoints to the zaplex release channel, uppercase Attention-Inbox hotkey (`cmd/ctrl-shift-O`), consistent launch-effort keys, waiting-state transitions keyed by stable host identity (#93–#97, #109).

## Zap — [Unreleased] (inherited at fork time)

- **AI / BYOP**: ported opencode's `applyCaching`, enabling prompt caching; `write_to_long_running_shell_command` now rejects embedded LF in line mode; the BYOP LRC monitor fallback moved to a silent subtask; fixed a sender leak in the 50 ms window of `cancel_execution` (#134 follow-up, #137)
- **Cloud strip-out, phase 1–2**: added a `cloud-disabled` channel predicate; removed billing/pricing, referral/reward, and cloud-sharing dialog UI; unsubscribed the RTC `UpdateManager`; retired the notebook/folder sync queue
- **Platform**: fixed a panic when launching macOS via Spotlight/Finder/Launchpad; `run_shell_command` stdout now falls back to the command grid
- **Infrastructure**: `.gitattributes` now forces LF; added a stale-issue bot and a Claude Code GitHub workflow
- **Editor**: code/Markdown viewers gained syntax highlighting for 15 more languages (Dart, Zig, SCSS, R, Julia, OCaml, Erlang, Nix, Groovy, Solidity, GraphQL, Protobuf, Clojure, Elm, CMake)

## [v2026.05.06.preview] — 2026-05-06

- **AI**
  - Integrated the DeepSeek CLI agent; improved LSP install reliability
  - LSP moved to a global `enabled_lsp_servers` setting; removed the `/index` command and the codebase-indexing runtime
  - `/plan` now faithfully reproduces Plan Mode (system prompt + hard tool guardrails)
  - Agent dynamic tool whitelist, `persist_conversations` setting, `ask_user_question` always asks under auto-approve
  - BYOP supports provider extra headers
- **Fixes**
  - `apply_file_diffs` schema changed from `const` to `enum` to accommodate Gemini
  - Root-caused the SSE stutter — genai gzip was off by default + workflow was split
  - Plan-folder notebooks are now created immediately in cloud-free environments
- **Branding**: logo and icons switched to a white background; BYOP mode hides the credits/billing UI

## [v2026.05.04.preview] — 2026-05-04

- **SSH Manager**: data layer + persistence + keychain landed; full UI/UX integration (panel + central pane + drag-and-drop + collapse + Connect + Command Palette)
- **AI**: distinguished the model's "no suggestion" output and refined the prompt system; BYOP history multimodal support extended to PDF/audio, opencode-style ERROR replacement; `UserQuery.context.images` kept alive end-to-end
- **UI**: title-bar search box can now be hidden; fixed contrast for keybinding-settings edit state and shortcut badges
- **i18n**: localized the remaining fixed strings in the main UI to Chinese; `/model` now defaults to `alt-shift-/`
- **Fixes**: Anthropic adapter now sends the 1M-context beta header by default; BYOP ToolCall emits a placeholder card on the first frame; the OpenAI-strict provider no longer echoes back `reasoning_content`
- **Infrastructure**: CI fix for the `.deb` build; enabled PR tests

## [v2026.05.03.preview(.2/.3/.4)] — 2026-05-03

- **Upstream sync**: merged a large batch of warp-upstream commits (cross-window tab drag, shell-script detection, IME cursor, remote-server init refactor, SSH remote-server auto-upgrade, cross-window tab drag, etc.); established `rerere` + a `zap-ours` merge driver; added a blocklist doc
- **AI / BYOP**: added a coercion layer for type-mismatched tool-parameter output; tightened the suspicious-backslash scan to eliminate false positives on `ls`/`diff`
- **i18n**: completed remaining Chinese localization (settings panel, etc.)
- **Website**: unified the GitHub URL to `zerx-lab/warp`; fixed mobile horizontal overflow
- **Fixes**: aligned the Windows taskbar ICO with the upstream format; restored NLD-in-terminal defaulting to true so Chinese input auto-routes to AI

## [v2026.05.02.preview] — 2026-05-02

- **AI / BYOP**
  - Completed the conversation-compaction loop — the `byop_compaction` module, settings persistence, auto-prune, overflow pass-through — a 1:1 port of opencode's behavior
  - Moved reasoning effort from provider settings to the input-box picker
  - Wired multimodal attachment support into the BYOP path
  - Local BYOP webfetch/websearch integrated with Exa
  - System-prompt templates now selected by model identifier; added several new templates
- **Privacy / cloud strip-out**
  - Physically removed P4 easily-strippable dead code (`anonymous_id` / `EXPERIMENT_ID_HEADER` / settings sync / `app_focus`)
  - Cut four closed-source outbound channels: telemetry, Sentry, `anonymous_id`, settings sync
  - Flipped three privacy toggles' defaults from true to false
  - Two cleanup passes on `cloud_conversations` (UI / privacy / FeatureFlag / AIClient / cargo feature)
- **Refactor**: removed blocklist AI-response scoring and its tracking; removed `agent_attribution` and the Oz changelog toggle
- **CI**: weekly builds now cut a formal release with normalized tags

## [v2026.05.01.preview] — 2026-05-01

- **Cloud strip-out**: physically removed 6 cloud LLM tools + `child_agent` + orchestration; physically removed the share-modal trio and the billing-denied modal; website switched to a monochrome logo
- **AI**
  - Wired Workflow Autofill into BYOP one-shot
  - BYOP LRC keeps injecting context on later turns + hardened sanitization + control-key tokens
  - Chat stream now surfaces remote-login session hints and reasoning pass-through
  - Refined genai error mapping into Stream / Other variants
  - Chat-stream adapter: fixed `ToolCall` `None` handling
- **Platform**: `warpui_core` avoids rescanning system fonts; sync commands now unconditionally disable the pager, using `PAGER=cat` to preserve the real exit code
- **Website**: full site component and i18n refactor, synced with Tailwind and global styles

## [v2026.04.30.oss] — 2026-04-30

- **CI**: renamed the `preview` channel to `oss`; fixed Windows/macOS build failures
- **Refactor**: removed leftover `cloud_mode` code and settings

## [v2026.04.30.preview] — 2026-04-30

First preview release of the Zap community fork.

- **Branding & positioning**: renamed Zap, redesigned the logo, community-fork README
- **BYOP**
  - Replaced `async-openai` with `genai`, supporting 5 natively-bound protocols
  - Providers sub-page + a models.dev data source + a quick-add search box
  - Trimmed the prompt templates
- **Decentralization cleanup**: removed the `UseComputer` / `RequestComputerUse` tools, the Drive "Create team" / "Join team" entry points, and referral-related code
- **i18n**: Fluent infrastructure + 12 translated `settings_view` files; completed i18n for the ai / features / teams pages
- **Website**: new BYOP landing page (Astro + Tailwind, bilingual EN/ZH); responsive improvements
- **AI**: CJK input classification, reasoning split out, BYOP `tool_call` diagnostics, LRC tag-in synthesizes a virtual subagent + floating spawn flow
- **CI**: Release workflow explicitly declares `contents: write` permission, fixing a 403

[Unreleased]: https://github.com/byte5ai/zaplex/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/byte5ai/zaplex/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/byte5ai/zaplex/releases/tag/v1.0.0
[v2026.05.06.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.04.preview...v2026.05.06.preview
[v2026.05.04.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.03.preview.4...v2026.05.04.preview
[v2026.05.03.preview(.2/.3/.4)]: https://github.com/zerx-lab/warp/compare/v2026.05.02.preview...v2026.05.03.preview.4
[v2026.05.02.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.01.preview...v2026.05.02.preview
[v2026.05.01.preview]: https://github.com/zerx-lab/warp/compare/v2026.04.30.oss...v2026.05.01.preview
[v2026.04.30.oss]: https://github.com/zerx-lab/warp/compare/v2026.04.30.preview...v2026.04.30.oss
[v2026.04.30.preview]: https://github.com/zerx-lab/warp/releases/tag/v2026.04.30.preview
