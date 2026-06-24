# AGENTS.md

> This file is the navigation document for AI/automation agents working in this repository. It summarizes the overall architecture, the responsibility of each crate in the Cargo workspace, the boundaries of the submodules under the `app/` main binary, and the engineering conventions you must follow before making changes.
>
> It is the companion to `WARP.md`: `WARP.md` is the engineer's handbook (commands, style, process), this file is the **code map**. Read `WARP.md` first, then use this file to locate the right crate / module. (Note: `WARP.md` is not currently present in this repo; references to it are historical.)

---

## 1. Repository overview

Warp is a Rust-first **agentic terminal / development environment**: on top of an in-house UI framework (WarpUI) it integrates terminal emulation, AI agents, cloud sync (Drive), code review, completion, Notebook, settings, IPC, and more.

Top-level directories:

| Directory | Purpose |
|------|------|
| `app/` | Main binary crate (`warp`); assembles all subsystems, UI, database migrations, platform glue layer |
| `crates/` | 67 workspace members, library crates split by responsibility |
| `command-signatures-v2/` | Standalone subproject (`--exclude`d when running nextest) |
| `script/` | Cross-platform bootstrap, build, and presubmit scripts |
| `resources/` | Runtime resources: fonts, icons, shell integration scripts, shaders, etc. |
| `docker/` | Containerized build setup |
| `specs/` | Product/technical spec documents |
| `.agents/skills`, `.claude/skills` | Skill descriptions for agent workflows (create PR, fix bugs, feature gating, etc.) |
| `.warp/`, `.config/`, `.cargo/`, `.vscode/` | Various tool configurations |

Build system: Cargo workspace, `resolver = "2"`, with `default-members` deliberately narrowed to the subset that is frequently compiled/tested (see `Cargo.toml`). `serve-wasm` and `integration` are not in `default-members` by default.

License split:
- `crates/warpui` and `crates/warpui_core` → MIT
- everything else → AGPL-3.0-only

---

## 2. Top-level architecture layers

Roughly 4 layers, bottom to top. When adding code or locating a bug, first determine which layer the change belongs to — **do not introduce inverted cross-layer dependencies**.

```
app/  (main binary: assembly, entry point, platform glue, persistence migrations, UI view root)
  ↑
product-domain crates: ai / computer_use / vim / onboarding /
              warp_completer / lsp / languages / code-review …
  ↑
framework crates: warpui / warpui_core / warpui_extras / editor /
            ui_components / sum_tree / syntax_tree
  ↑
infrastructure crates: warp_core / warp_util / http_client /
                websocket / ipc / jsonrpc / persistence / graphql /
                managed_secrets / virtual_fs / watcher / asset_cache …
```

Key architectural patterns (see `WARP.md` for details):

1. **Entity-Handle system**: `App` globally owns all view/model entities; views reference each other via `ViewHandle<T>` rather than owning directly.
2. **Element / Action**: the UI is a declarative Element tree + an Action event system (Flutter style).
3. **Cross-platform**: native implementations for macOS / Windows / Linux + a WASM target; platform code is isolated with `#[cfg(...)]`.
4. **AI integration**: Agent Mode and context indexing; code is concentrated in `app/src/ai` (389 files) and `crates/ai`.
5. **Cloud sync**: `Drive` keeps objects in sync across devices; see `app/src/drive` and `crates/warp_files`.
6. **Feature flags**: runtime gating is preferred over `#[cfg]`; the enum is defined in `crates/warp_core/src/features.rs`.

---

## 3. `crates/` overview

The table below lists all 67 crates, grouped by topic. Each row gives a **one-line responsibility**; for implementation details, open the corresponding `crates/<name>/src/lib.rs` (many crates have `//!` module docs at the top of `lib.rs`).

### 3.1 UI framework / view layer

| Crate | Responsibility |
|-------|------|
| `warpui_core` | WarpUI framework core (MIT): `App` / `Entity` / `ViewHandle` / `AppContext` and other infrastructure |
| `warpui` | WarpUI higher-level components, Element tree, layout, render pipeline (MIT) |
| `warpui_extras` | Optional WarpUI extensions; not all features enabled by default |
| `ui_components` | High-level component library reused across views (buttons, inputs, lists, modals, etc.) |
| `editor` (`warp_editor`) | Text editor: buffers, selection, cursor, key mapping, undo stack |
| `sum_tree` | Persistent balanced B-tree; the core data structure for the editor / Notebook / large lists |
| `syntax_tree` | Tree-sitter wrapper and syntax-highlighting support |
| `markdown_parser` | Markdown parsing (for AI messages, doc views, Notebook, etc.) |
| `vim` | Vim-mode key bindings and operation semantics |
| `voice_input` | Voice input support |

### 3.2 Terminal

| Crate | Responsibility |
|-------|------|
| `warp_terminal` | Terminal emulation core: PTY management, ANSI/VT parsing, grid, scrolling, shell integration hooks |
| `input_classifier` | Terminal input intent classification (plain command / natural language / AI prompt) |
| `natural_language_detection` | Natural-language detection (works with `input_classifier`) |

### 3.3 AI / Agent

| Crate | Responsibility |
|-------|------|
| `ai` | AI model clients, prompt orchestration, agent protocol, tool-calling framework |
| `computer_use` | Rust-side implementation of "Computer Use" tool capabilities (screenshot, click, type, etc.) |
| `command-signatures-v2` | Command signatures v2 (command-classification metadata for the AI); standalone project, not part of the main workspace test set |
| `onboarding` | New-user onboarding flow data/state |

### 3.4 Network / protocol / IPC

| Crate | Responsibility |
|-------|------|
| `http_client` | Workspace-wide unified HTTP client wrapper |
| `http_server` | Embedded HTTP server (local RPC, login callbacks, etc.) |
| `websocket` | WebSocket abstraction shared by native and WASM, adapted to `graphql_ws_client` |
| `ipc` | Generic typed IPC request/response protocol (inter-process) |
| `jsonrpc` | JSON-RPC implementation |
| `lsp` | Language Server Protocol client implementation |
| `remote_server` | Server-side logic for the remote sshd mode |
| `serve-wasm` | Helper server that hosts the WASM build output (not part of the default compile) |
| `firebase` | Firebase client utilities (crash/analytics channels, etc.) |

### 3.5 Persistence / files / resources

| Crate | Responsibility |
|-------|------|
| `persistence` | Diesel + SQLite persistence base; **migrations live in `app/migrations/`, the schema in `app/src/persistence/schema.rs`** |
| `warp_files` | Syncable file objects: Drive files, Workflows, Notebooks, etc. |
| `virtual_fs` | Filesystem abstraction (test-time mock and production real FS share one interface) |
| `repo_metadata` | Repository metadata: file-tree building, `.gitignore` handling, filesystem watching |
| `watcher` | Filesystem watcher (wrapper around `notify`) |
| `asset_cache` | Disk/memory cache for assets |
| `asset_macro` | Asset-reference macros such as `bundled!` / `theme!` |
| `managed_secrets` / `managed_secrets_wasm` | Keychain / DPAPI / Linux Keyring abstraction + WASM proxy |

### 3.6 Configuration / settings

| Crate | Responsibility |
|-------|------|
| `settings` | Settings storage and change dispatch |
| `settings_value` | `SettingsValue` trait: controls TOML serialization semantics |
| `settings_value_derive` | `#[derive(SettingsValue)]` procedural macro (e.g. enum variants to snake_case) |
| `warp_features` | High-level feature-flag API (consumer side) |
| `channel_versions` | Release channels (stable/preview/dogfood) and version comparison |

### 3.7 Commands / completion / languages

| Crate | Responsibility |
|-------|------|
| `command` | Safe wrapper for cross-platform process spawning, **with special handling for Windows' `no_window` flag**; all new child processes go through here |
| `warp_completer` | Completion engine (supports `--features v2`) |
| `languages` | Language / file-extension / Tree-sitter grammar registration |
| `warp_ripgrep` | Thin ripgrep wrapper for `warp_cli` |
| `warp_cli` | In-binary CLI subcommand parsing (`warp <subcmd>`) |
| `fuzzy_match` | Fuzzy matching + glob-style wildcards, used for path search and the command palette |

### 3.8 Platform / system services

| Crate | Responsibility |
|-------|------|
| `app-installation-detection` | Detects apps installed on the system (for launcher integration) |
| `prevent_sleep` | Suppresses sleep (during long tasks / AI agents) |
| `isolation_platform` | Compatibility layer for running in sandboxes such as Docker / GitHub Actions |
| `node_runtime` | Auto-install/manage Node.js and npm (macOS/Linux/Windows × multiple architectures) |
| `warp_js` | Helper abstraction for manipulating JavaScript values/functions from Rust |

### 3.9 General utilities / communication

| Crate | Responsibility |
|-------|------|
| `warp_core` | The lowest-level "core" in the workspace: platform abstraction, the `FeatureFlag` enum and `DOGFOOD/PREVIEW/RELEASE_FLAGS` in `features.rs` |
| `warp_util` | General utility functions reused across many crates |
| `warp_logging` | Unified logging configuration entry point |
| `simple_logger` | Simple async file logger for stderr-only processes such as `remote_server` |
| `warp_web_event_bus` | Web-side event bus (for the embedded web view) |
| `field_mask` | gRPC/Proto-style FieldMask utilities |
| `string-offset` | Offset base types (byte/char/utf16) |
| `handlebars` | Handlebars templating-engine wrapper |
| `integration` | Integration-testing framework; test-only |

> Naming gotchas: the package name of `crates/editor` is `warp_editor`; `crates/isolation_platform` is `warp_isolation_platform`; `crates/managed_secrets` is `warp_managed_secrets`; `crates/virtual_fs` is `virtual-fs` (hyphen); `crates/string-offset` is `string-offset` (hyphen).

---

## 4. `app/` submodule navigation

`app/src/` flatly contains 60+ product-domain directories, each roughly corresponding to one product feature line. Grouped by topic below; the number in parentheses is the approximate `.rs` file count, to gauge module size:

### 4.1 Startup / assembly / global
- `bin/` (7) — multiple binary entry points (main program, accompanying tools).
- `lib.rs` / `app_state.rs` / `app_state_tests.rs` — application state root.
- `app_menus.rs`, `app_services/`, `app_id_test.rs`
- `appearance.rs`, `gpu_state.rs`, `font_fallback.rs`, `global_resource_handles.rs`
- `dynamic_libraries.rs`, `alloc.rs`, `tracing.rs`, `profiling.rs`
- `crash_recovery.rs`, `crash_reporting/` (4)
- `features.rs` — `app/`-side consumption of `warp_core::FeatureFlag`; adding a flag usually requires wiring it in both places.
- `channel.rs`, `download_method.rs`, `autoupdate/` (8)

### 4.2 Terminal
- `terminal/` (427) — the bulk: shell process, PTY, grid, blocks, shell integration, command execution, I/O pipeline.
- `default_terminal/` (2) — default terminal launch logic.
- `shell_indicator.rs`, `prefix.rs` / `prefix_test.rs` (command-prefix parsing), `vim_registers.rs`

### 4.3 AI / Agent
- `ai/` (389) — includes Agent UI, conversation model, agent management, tools/MCP, Cloud Agent, Plan/Diff views, artifacts, blocklist, execution profiles, etc. **This is the largest subtree in the repo**; before changing it, grep within the directory for the specific subtopic (`agent_*`, `conversation_*`, `cloud_agent_*`, `mcp`, `tool_*`).
- `ai_assistant/` (9) — legacy AI-assistant entry/adapter.
- `chip_configurator/`, `context_chips/` (22) — Agent context-chip selection/construction.
- `coding_entrypoints/` (5), `coding_panel_enablement_state.rs`
- `prompt/` (2), `tips/` (3), `voice/` (2), `completer/` (3)

### 4.4 Editor / code / review
- `editor/` (38) — main editor integration.
- `code/` (52) — code view, diff, navigation.
- `code_review/` (36) — code review flow.
- `notebooks/` (30), `workflows/` (22)

### 4.5 Search
- `search/` (172) — multi-target search (files, commands, agent history, etc.).
- `search_bar.rs`

### 4.6 Server communication / Drive / sync
- `server/` (55) — HTTP/WS interaction with the warp backend (corresponds to the local dev mode `with_local_server`).
- `drive/` (45) — cloud object sync entry point.
- `cloud_object/` (12) — cloud-object abstraction layer (workflow, notebook, etc.).
- `remote_server/` (5) — client-side glue for connecting to the remote-mode sshd.

### 4.7 Settings / user config / themes / onboarding
- `settings/` (46), `settings_view/` (63)
- `user_config/` (6), `themes/` (11), `appearance.rs`
- `experiments/` (7), `tab_configs/` (15), `launch_configs/` (4)
- `tips/`, `banner/` (3), `quit_warning/` (1), `wasm_nux_dialog.rs`, `referral_theme_status.rs`

### 4.8 Auth / billing / usage
- `auth/` (22) — login, token, SSO.
- `billing/` (3), `pricing/` (1), `usage/` (1), `reward_view.rs`

### 4.9 Persistence
- `persistence/` (9) — Diesel migration assembly, `schema.rs` (generated by Diesel), migration runner.
- Migration files live in the top-level `migrations/` directory (managed by the Diesel CLI).

### 4.10 Platform / system integration
- `platform/` (2), `system/` (3) / `system.rs`
- `login_item/` (3), `antivirus/` (3), `network.rs`
- `external_secrets/` (1), `env_vars/` (14)
- `keyboard.rs` / `keyboard_test.rs`, `safe_triangle.rs` / `safe_triangle_tests.rs` (menu-hover safe triangle)

### 4.11 View root / panels / general UI
- `root_view.rs` / `root_view_tests.rs`
- `pane_group/` (35) — split-pane layout.
- `tab.rs`, `command_palette.rs`, `modal.rs`, `menu.rs` / `menu_test.rs`
- `palette.rs`, `notification.rs`, `resource_center/` (10)
- `view_components/` (20), `ui_components/` (14)
- `workspace/` (54), `workspaces/` (10), `voltron.rs` (multi-window / multi-workspace coordination)
- `session_management.rs`, `undo_close/` (3), `word_block_editor.rs`
- `suggestions/` (2), `input_suggestions.rs` / `input_suggestions_test.rs`
- `plugin/` (21) — plugin system integration.
- `uri/` (7) — `warp://` URL handling.
- `debug_dump.rs`, `debounce.rs`, `interval_timer.rs`, `throttle.rs`
- `linear.rs`, `resource_limits.rs`, `warp_managed_paths_watcher.rs`
- `preview_config_migration.rs` / `preview_config_migration_tests.rs`
- `window_settings.rs`, `projects.rs`

### 4.12 Test infrastructure
- `integration_testing/` (79) — end-to-end integration-test support.
- `test_util/` (6) — shared unit-test utilities.

---

## 5. Engineering discipline (hard constraints for agents)

> These are compiled from `WARP.md` and project-specific rules; this file's verification requirement for agents is `cargo check`.

### 5.1 Must-read conventions
- **Code language is English (repo policy).** All comments and doc-comments in
  our own code (`zaplex_*` crates and any edits we make to inherited files) must
  be written in English. The codebase inherits extensive Chinese comments from
  upstream `zerx-lab/zap`; these are being migrated to English and **no new
  Chinese may be introduced** (enforced by the `no-new-cjk` CI guard). This
  supersedes the former "Simplified-Chinese only" rule, which was an upstream
  artifact. (Project docs and commit messages follow the German byte5ai
  convention; this rule is about *code*.)
- For searching/grepping within the git index, use the `fff` tool or `rg -n "<keyword>" <path>`; use `read_file` only for images/binaries.
- Before opening a PR / pushing a new commit, you **only** need to pass: `cargo check`.
- Changes must be precise: **every modified line must be traceable to a user request**; do not casually "improve" unrelated code, comments, or formatting.
- Prefer simplicity: do not introduce abstraction, configuration, error handling, or extra features for a single use site.
- Explain options and surface uncertainty rather than silently making choices for the user.
- worktree path: .worktrees/<worktree_name>/

### 5.2 Rust style (from `WARP.md`)
- Do not write redundant type annotations on closure parameters.
- Consolidate `use` statements at the top; do not write long path-qualified names inline; `#[cfg]` branches are an exception.
- Name the context parameter `ctx` and put it last; if there is also a closure parameter, put the closure last.
- **Delete** unused parameters rather than prefixing them with `_`, and update the call sites accordingly.
- Use inline format arguments in macros like `println!` / `format!` (`"{x}"` rather than `"{}", x`) to satisfy `uninlined_format_args`.
- **Do not use the `_` wildcard** in `match` statements (unless genuinely needed); keep matches exhaustive.
- Do not delete/modify existing comments because of an unrelated change.

### 5.3 Terminal model lock (high priority!)
- Calling `TerminalModel::lock()` is highly prone to deadlock (on macOS this shows up as a frozen UI / beachball).
- Before adding a new `model.lock()`, confirm no caller higher up the call stack already holds the lock; prefer passing an already-locked reference down the call stack rather than locking again.
- Minimize the locked scope, and do not call functions that might lock again while holding the lock.

### 5.4 Feature flags
- Adding: add a variant to the `FeatureFlag` enum in `crates/warp_core/src/features.rs`; add it to `DOGFOOD_FLAGS` / `PREVIEW_FLAGS` / `RELEASE_FLAGS` as needed.
- Using: **prefer** the runtime `FeatureFlag::Xxx.is_enabled()` over `#[cfg(...)]`; use `cfg` only when it otherwise would not compile (platform / optional dependency).
- Wrap an entire product feature, not every call site; once stable, **clean up the flag and the dead branches**.
- The UI entry point and the code path should use the same flag.

### 5.5 Database
- ORM: Diesel + SQLite.
- Any new/changed schema must go through a migration: add a new directory under `migrations/` (`up.sql` / `down.sql`); do not hand-edit `app/src/persistence/schema.rs` (generated by `diesel print-schema`).

### 5.6 Testing
- Use `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`.
- Put unit tests in `${filename}_tests.rs` or `mod_test.rs`, wired at the end of the original file with:

  ```rust
  #[cfg(test)]
  #[path = "filename_tests.rs"]
  mod tests;
  ```

- Use the `crates/integration` framework for integration tests; examples are in `app/src/integration_testing/`.

### 5.7 Cross-process commands
- Do not call `std::process::Command::new(...)` directly (it pops a window on Windows in particular); always go through `crates/command`.

### 5.8 Subagents / multi-agent
- Split large tasks into subtasks with **non-overlapping write domains** and dispatch them in parallel; information-gathering tasks can run in parallel.
- Do simple tasks directly; do not over-decompose.

---

## 6. Common entry-point cheatsheet

| What you want to do | Starting point |
|---------|------|
| Change terminal grid / shell integration | `crates/warp_terminal/src/`, together with `app/src/terminal/` |
| Change Agent UI / conversation | grep within `app/src/ai/` by topic (`agent_*` / `conversation_*`) |
| Change command completion | `crates/warp_completer/` (mind `--features v2`) |
| Change AI model / tool-calling protocol | `crates/ai/` |
| Add a new setting | `crates/settings_value*`, `crates/settings`; UI in `app/src/settings_view/` |
| Add a feature flag | `crates/warp_core/src/features.rs` + use sites |
| Change cloud-sync objects | `crates/warp_files` + `app/src/drive/` + `app/src/cloud_object/` |
| Change persistence structure | add a migration under `migrations/` + `crates/persistence` |
| Add a new binary tool | `app/src/bin/` |
| Platform-specific code | use `#[cfg(target_os = "...")]`; UI platform glue in `app/src/platform/` |
| Vim mode | `crates/vim` + `app/src/vim_registers.rs` |
| Notebook / Workflow | `app/src/notebooks/`, `app/src/workflows/`, `crates/warp_files` |
| Cross-platform process spawning | `crates/command` |
| File search / watching | `crates/repo_metadata`, `crates/watcher`, `crates/warp_ripgrep` |

---

## 7. Pre-change checklist

Before touching the keyboard to change code, ask yourself once:

1. Which layer / crate / `app/src/<submodule>` does this belong to? Does the change cross a layer boundary?
2. Do you need a new dependency? If an existing workspace dependency can be reused, prefer reusing it via `Cargo.toml` `[workspace.dependencies]`.
3. Is this a product feature? Does it need to be wrapped in a feature flag?
4. Does it involve the terminal model? Does the current call stack already hold the `TerminalModel` lock?
5. Does it spawn a child process? Did you go through `crates/command`?
6. Does it involve persistence? Does it need a migration?
7. Have you written the corresponding `${file}_tests.rs`?
8. Is `cargo check` green?
9. Can every changed line be traced back to the user request? Should any incidental "small refactor" be reverted?

Go through all 9 before delivering.

---

## 8. Engineering Standards (byte5ai) — Git-Workflow für alle Agenten

> Diese Sektion ist zaplex-spezifisch (kein Upstream-Zap-Inhalt). Sie ergänzt die obige
> Code-Map um die verbindlichen Workflow-Regeln. Status & Quelle:
> `.github/engineering-standards.yml` (`status: applied`, Quelle `byte5ai/engineering-standards`).
> Diese Regeln gelten für **alle** Agenten (Claude, Codex, Copilot) und für Menschen.

- **Nie direkt auf `main` pushen.** Jede Änderung läuft über Feature-Branch + Pull Request.
- **Branch-Naming:** `feat/` · `fix/` · `refactor/` · `docs/` · `chore/` · `test/` · `release/vX.Y` · `dev/vX.Y.devN`.
- **Conventional Commits:** `feat:` · `fix:` · `docs:` · `chore:` · `refactor:` · `test:` · `release:` · `dev:`.
- **Keine `Co-Authored-By:`-Trailer** für Claude oder andere KI-Agenten. Commits unter der konfigurierten Git-Identität, ohne Model-Attribution-Footer.
- **Nie force-push** auf geteilte Branches. **Nie Secrets committen** (`.env`, Tokens, Keys). **Nie `--no-verify`.**

### Worktree-Disziplin (durchgesetzt via `.hooks/pre-commit`)

Der Haupt-Klon empfängt **keine** Commits. Jede Änderung — auch ein Einzeiler — landet in einem
Worktree. Branch-agnostisch: fängt auch den Fall, dass eine parallele Session den HEAD des
Haupt-Klons verschoben hat.

~~~bash
git worktree add ../zaplex-<feature> -b <branch> main   # neu mit Branch
git worktree add ../zaplex-<feature> <existing-branch>  # oder bestehenden anhängen
git worktree remove ../zaplex-<feature>                 # nach Merge/Discard
git branch -D <feature>                                 # Branch-Ref entfernen
~~~

Auf diesem Host liegen Worktrees als Sibling unter `<repo>.worktrees/<branch>/` (Host-Konvention).
Orphans aufräumen: `script/prune-worktrees` (dry-run; `--yes` zum Anwenden).

**Bypass-Stufen** (aufsteigende Persistenz): `ALLOW_MAIN_TREE_BRANCH=1 git commit …` (einmalig) ·
`git config engineering-standards.main-tree-discipline false` (pro Repo) ·
`status: exempt` in `.github/engineering-standards.yml` (alles aus).

### Pre-push Hook

`.hooks/pre-push` blockiert direkte Pushes auf `main`/`master`. Override nur auf explizite
Anweisung: `ALLOW_PUSH_TO_MAIN=1 git push origin main`.

### Releases

- **Stable-Tags (`vX.Y`, `vX.Y.Z`) nur aus `main`** — Merge-Commit auf `main` taggen, nie den
  HEAD eines Feature/Dev-Branches. Durchgesetzt via `.github/workflows/release-tag-guard.yml`.
- **Pre-Release-Tags** (`vX.Y.Z-rc1`, `vX.Y.devN`, …) dürfen aus Feature/Dev-Branches kommen.
- **Nie eigenmächtig releasen** — nur auf explizite User-Anweisung.

### Hooks aktivieren (frischer Klon)

~~~bash
script/setup            # installiert Deps via script/bootstrap + setzt core.hooksPath
# oder nur:  git config core.hooksPath .hooks
~~~
