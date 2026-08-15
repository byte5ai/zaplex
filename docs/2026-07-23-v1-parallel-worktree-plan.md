# Zaplex 1.0: Parallel Worktree Implementation Plan

Date: 2026-07-23
Status: approved implementation scope; execution has not started
Integration source: `rc/master-plan`

## 1. Goal

Deliver Zaplex 1.0 as a stable release rather than another preview. Work is split
into independent, user-visible Codex sessions with their own worktrees. The plan
maximizes useful parallelism without allowing two sessions to edit the same
high-risk files at the same time.

The confirmed release sequence is:

1. protect data and credentials;
2. open and resume the correct agent session reliably;
3. replace the competing navigation surfaces with one coherent spine;
4. make the file manager cover the core Midnight Commander workflow;
5. bring all fork-owned UI to the quality of the inherited Warp UI;
6. harden documentation, versions, first-run behavior, and release validation.

## 2. Non-negotiable execution rules

- Never work in the main clone. All implementation stays below
  `~/projects/zaplex/repos/zaplex.worktrees/`.
- Do not create branches, worktrees, repositories, or remote state until the
  user has explicitly approved the exact targets at that moment.
- Do not push, open or merge a PR, start CI, build a DMG, publish, tag, or
  release without a fresh explicit approval for that action.
- Never force-push.
- Do not build the application locally. Focused tests and `cargo check` are the
  local verification gates.
- Every fix needs a test that is proven red without the fix and green with it.
  The handoff must preserve the exact command and observed failure.
- Before every commit, an independent critical review of the exact diff is
  mandatory. Data, security, money, protocol, and release work require a second
  model rather than a self-review alone.
- Code and code comments are English.
- Every lane uses its own `CARGO_TARGET_DIR` to avoid lock contention and
  cross-lane build artifacts.
- At most three worker sessions run concurrently in addition to the
  orchestrator.

## 3. Gate 0: create a trustworthy base

The current `rc/master-plan` worktree contains reviewed and provisional
uncommitted changes across session startup, session identity, SFTP safety, PID
handling, localization, and test wiring. It must not be copied into multiple
worktrees.

Before any implementation lane starts:

1. verify the exact worktree and branch;
2. finish all outstanding focused tests and controlled red/green proofs;
3. obtain an independent critical review for each logical commit;
4. resolve or explicitly reject every finding with evidence;
5. run `git diff --check`;
6. run the focused tests and `cargo check` with a dedicated target directory;
7. commit the changes as reviewable logical units;
8. verify a clean worktree;
9. record the resulting commit as `V1_BASE_SHA`.

Recommended Gate 0 target directory:

```text
/tmp/zaplex-v1-gate0-target
```

Only after Gate 0 may the integration branch `release/v1.0` be created from
`V1_BASE_SHA`. No lane may start from `b4b6ed4f`, `main`, `origin/main`, or a
dirty working tree.

Gate 0 model: `gpt-5.6-sol`, effort `high`. Escalate only a concrete unresolved
regression to `xhigh`; do not use `max` by default.

## 4. Dependency graph

```text
Gate 0: clean V1_BASE_SHA
        |
        +-- L1 SFTP integrity --> L5a MC interaction --> L5b transfers --+
        +-- L2 SSH security ---------------------------------------------+
        +-- L3 agent routing --> L4 cockpit truth -----------------------+
                                                                         |
                                                                         v
                                                               L6 navigation spine
                                                                         |
                                                                         v
                                                               L7 cockpit/SSH UI
                                                                         |
                                                                         v
                                                               L8 release hardening
```

L1, L2, and L3 start first. L4 starts only after L3 has been reviewed and
integrated, because both touch cockpit types and session boundaries. L5a starts
after L1 is integrated; L5b starts only after L5a is integrated. These
dependency-ready lanes fill free worker slots while preserving the limit of
three workers plus the orchestrator.

L6 starts only after L2, L3, L4, L5a, and L5b are integrated. This keeps
workspace actions, keymaps, host identity, favorites, and menu migration out of
parallel edits. L7 starts after L6. L8 is the final integration and release
gate.

### Exact issue-to-lane assignment

| Lane | Issues | Exclusive result |
|---|---|---|
| L0 Integration | [#124](https://github.com/byte5ai/zaplex/issues/124) and Gate 0 | clean `V1_BASE_SHA`, scheduling and integration |
| L1 SFTP integrity | [#125](https://github.com/byte5ai/zaplex/issues/125), [#126](https://github.com/byte5ai/zaplex/issues/126), [#127](https://github.com/byte5ai/zaplex/issues/127), [#128](https://github.com/byte5ai/zaplex/issues/128) | safe file semantics, stable actions and SFTP click targets |
| L2 SSH security | [#130](https://github.com/byte5ai/zaplex/issues/130), [#131](https://github.com/byte5ai/zaplex/issues/131) | consistent credentials, host identity and endpoint validation |
| L3 Agent routing | [#129](https://github.com/byte5ai/zaplex/issues/129), [#132](https://github.com/byte5ai/zaplex/issues/132), [#133](https://github.com/byte5ai/zaplex/issues/133), [#134](https://github.com/byte5ai/zaplex/issues/134), [#135](https://github.com/byte5ai/zaplex/issues/135) | safe process control, exactly-once startup and PTY routing |
| L4 Cockpit truth | [#141](https://github.com/byte5ai/zaplex/issues/141) | correct token and price accounting |
| L5a MC interaction | [#138](https://github.com/byte5ai/zaplex/issues/138), [#139](https://github.com/byte5ai/zaplex/issues/139) | keyboard, focus, layout and pane-owned F legends |
| L5b Transfers | [#140](https://github.com/byte5ai/zaplex/issues/140) | streaming transfer queue, conflicts, cancellation and source safety |
| L6 Navigation | [#136](https://github.com/byte5ai/zaplex/issues/136), [#137](https://github.com/byte5ai/zaplex/issues/137) | joined host tree, migrated favorites and curated plus menu |
| L7 Cockpit/SSH UI | [#142](https://github.com/byte5ai/zaplex/issues/142) | remaining cockpit/SSH state, persistence and component integration |
| L8 Release | [#143](https://github.com/byte5ai/zaplex/issues/143) | pre-build readiness, approved artifact and runtime matrix |

## 5. Branches, worktrees, ownership, and models

The paths below are the nominal host worktree paths. A Codex-managed visible
task may receive a managed physical path; the orchestrator must record the
actual path and verify that it is based on the approved branch.

### L0 — Integration

- Branch: `release/v1.0`
- Nominal worktree: `zaplex.worktrees/release-v1.0`
- Base: `V1_BASE_SHA`
- Epic: [#124](https://github.com/byte5ai/zaplex/issues/124)
- Owner: orchestrator only
- Model: `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-integration-target`

The integration lane owns the master plan, dependency state, merge order,
cross-lane verification, and final release decision. Worker sessions do not
edit the central plan.

### L1 — SFTP integrity

- Branch: `fix/v1-sftp-integrity`
- Nominal worktree: `zaplex.worktrees/v1-sftp-integrity`
- Base: `release/v1.0` at `V1_BASE_SHA`
- Issues: [#125](https://github.com/byte5ai/zaplex/issues/125),
  [#126](https://github.com/byte5ai/zaplex/issues/126),
  [#127](https://github.com/byte5ai/zaplex/issues/127),
  [#128](https://github.com/byte5ai/zaplex/issues/128)
- Primary ownership: `app/src/sftp_manager/`
- Model: `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-sftp-integrity-target`

Scope:

- make directory moves failure-safe so a failed copy cannot delete the source;
- prove that a remote-to-remote move with a Skip conflict preserves the entire
  source tree;
- treat untransferred symlinks and special files as an incomplete move that can
  never trigger source or source-tree deletion;
- identify selected files by stable identity rather than a stale list index;
- use exclusive per-operation temporary paths and clean them safely;
- classify and handle files, directories, links, broken links, sockets, FIFOs,
  and devices correctly;
- make overwrite, conflict, cancellation, and cleanup behavior explicit;
- keep SFTP breadcrumb, `..`, rescan, transfer-cancel, and file-row mouse state
  alive across re-renders; cockpit and workspace click targets remain outside
  L1;
- add failure-injection tests for every data-loss boundary.

L1 owns SFTP behavior and safety. It does not redesign the command bar or
keyboard workflow. L5a must not begin editing this subtree before L1 is
integrated.

### L2 — SSH and credential security

- Branch: `fix/v1-ssh-security`
- Nominal worktree: `zaplex.worktrees/v1-ssh-security`
- Base: `release/v1.0` at `V1_BASE_SHA`
- Issues: [#130](https://github.com/byte5ai/zaplex/issues/130),
  [#131](https://github.com/byte5ai/zaplex/issues/131)
- Primary ownership:
  - `app/src/ssh_manager/`
  - `crates/warp_ssh_manager/`
- Model: `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-ssh-security-target`

Scope:

- remove all child-host secrets when deleting an SSH folder;
- make credential operations repeatable and reference-safe, with explicit,
  visible compensation between database, keychain, and sync without implying
  one atomic commit across those systems;
- verify unknown and changed host identities instead of disabling the check;
- prevent deletion of a shared OneKey credential while hosts still reference
  it, or provide an explicit safe reassignment;
- validate non-empty hosts and ports from 1 through 65535 consistently for
  saving, testing, and connecting;
- prevent an older connection-test result from replacing a newer one;
- surface keychain-copy and sync-version failures;
- route process execution through an injectable Workspace command factory based
  on `crates/command`;
- enforce a static guard against direct `std::process::Command` use in the SSH
  Workspace domain, covered together with the factory by focused tests.

L2 must not edit `left_panel.rs` or redesign the host navigation. L6 consumes
the resulting safe host APIs later.

### L3 — Agent and terminal-session routing

- Branch: `fix/v1-agent-session-routing`
- Nominal worktree: `zaplex.worktrees/v1-agent-session-routing`
- Base: `release/v1.0` at `V1_BASE_SHA`
- Issues: [#129](https://github.com/byte5ai/zaplex/issues/129),
  [#132](https://github.com/byte5ai/zaplex/issues/132),
  [#133](https://github.com/byte5ai/zaplex/issues/133),
  [#134](https://github.com/byte5ai/zaplex/issues/134),
  [#135](https://github.com/byte5ai/zaplex/issues/135)
- Primary ownership:
  - terminal CLI-session and daemon protocol code;
  - agent-session identity and routing;
  - `app/src/workspace/view/spawn_card.rs`;
  - the cockpit routing files listed under shared hotspots.
- Model: `gpt-5.6-sol`, effort `xhigh`
- Target directory: `/tmp/zaplex-v1-agent-routing-target`

Scope:

- require process start-time/fingerprint identity on platforms that support it;
  if identity cannot be proven, fail closed without sending a signal;
- assign one stable command ID to each startup and retain it through retries
  and reconnects;
- keep the startup pending until a daemon acknowledgement, not merely a local
  writer enqueue; deduplicate the same ID after a lost acknowledgement or
  mid-flight reconnect, and never evict an unacknowledged startup from the
  pending buffer;
- introduce a reliable agent-session-to-PTY binding so a live remote session
  can be opened instead of duplicated;
- define bind and unbind lifecycle, session generation, rejection of foreign or
  stale PTY IDs, and explicit `agent-pty-binding` capability negotiation using
  real serialized legacy fixtures;
- allow multiple historical agent sessions per PTY but only one attachable live
  foreground agent; reject a second live binding unless an explicit controlled
  handoff replaces the first;
- keep host, provider, account, configuration directory, and session ID in the
  identity;
- execute startup exactly once after the terminal is ready;
- never leave a visible bootstrap or resume command in terminal input;
- accept an explicit home directory or an absolute remote directory, and reject
  ambiguous relative paths;
- remove Claude effort controls that do not affect the launched command, or
  make them functional;
- provide visible, accurate failure states when an existing session cannot be
  attached.

This is the only implementation lane that starts at `xhigh`: it changes the
cross-process contract and the user-visible meaning of opening a live session.
Fish and pwsh body support is a separate serial substage after the negotiated
PTY protocol is reviewed and integrated. L4 and L6 must wait for L3's stable
APIs.

### L4 — Honest cockpit usage and pricing

- Branch: `fix/v1-cockpit-truth`
- Nominal worktree: `zaplex.worktrees/v1-cockpit-truth`
- Base: the reviewed and integrated L3 result
- Issue: [#141](https://github.com/byte5ai/zaplex/issues/141)
- Primary ownership:
  - `crates/zaplex_cockpit/src/codex.rs`
  - `crates/zaplex_cockpit/src/codex_sessions.rs`
  - `crates/zaplex_cockpit/src/pricing.rs`
  - `crates/zaplex_cockpit/src/format.rs`
  - `crates/zaplex_cockpit/src/types.rs`
- Model: `gpt-5.6-terra`, effort `high`
- Target directory: `/tmp/zaplex-v1-cockpit-truth-target`

Scope:

- stop counting cached Codex input twice;
- distinguish unavailable pricing from a real zero cost;
- mark estimates as estimates;
- add exact fixtures for token and cost calculations.

L4 starts only after L3 is integrated. It must not edit
`app/src/cockpit/{model,pane,panel,capabilities}.rs` or
`crates/zaplex_cockpit/{lib,sessions,conductor,guardrails}.rs`. L3 owns those
files. UI wiring is completed by L6 or L7.

### L5a — Midnight Commander interaction and layout

- Branch: `feat/v1-file-manager-mc`
- Nominal worktree: `zaplex.worktrees/v1-file-manager-mc`
- Base: the reviewed and integrated L1 result
- Primary ownership: file-manager UI and input handling in
  `app/src/sftp_manager/`
- Issues: [#138](https://github.com/byte5ai/zaplex/issues/138) and
  [#139](https://github.com/byte5ai/zaplex/issues/139)
- Model: `gpt-5.6-terra`, effort `high`
- Target directory: `/tmp/zaplex-v1-file-manager-mc-target`

Scope:

- cycle forward with Tab and backward with Shift-Tab;
- implement the documented F-key behavior, including distinct view and edit
  actions;
- make the active pane unmistakable;
- render an optional compact F-key legend owned by each pane at that pane's
  lower edge; never render one global bar spanning both panes;
- drop pane-local captions responsively before they can overlap;
- remove dead lower space while preserving room for file lists;
- add keyboard, focus, narrow-window, and layout tests.

L5a consumes L1 safety behavior and may not replace it with UI shortcuts.
SFTP raw-error humanization remains in L1/Issue 01; SSH raw-error humanization
remains in L7/Issue 18.

### L5b — Streaming transfer queue

- Branch: `feat/v1-file-manager-transfers`
- Nominal worktree: `zaplex.worktrees/v1-file-manager-transfers`
- Base: the reviewed and integrated L5a result
- Primary ownership:
  - new SFTP registry, queue and job modules;
  - transfer-specific pane-group integration;
  - workspace transfer activity.
- Issue: [#140](https://github.com/byte5ai/zaplex/issues/140)
- Model: `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-file-manager-transfers-target`

Scope:

- choose visible and hidden pane targets deterministically;
- stream local-to-remote, remote-to-local, and remote-to-remote transfers
  without buffering whole files;
- implement conflict decisions, progress, ETA, pause, cancellation, and global
  activity;
- preserve source and existing destination on failure, cancellation, skipped
  conflicts, untransferred links, or special files;
- revalidate stable entry identities before destructive completion.

L5b is strictly serial after integrated L5a. It owns transfer behavior and does
not redesign the pane layout or keymap.

### L6 — Integrated navigation spine

- Branch: `feat/v1-navigation-spine`
- Nominal worktree: `zaplex.worktrees/v1-navigation-spine`
- Base: the reviewed and integrated L2, L3, L4, L5a, and L5b results
- Issues: [#136](https://github.com/byte5ai/zaplex/issues/136) and
  [#137](https://github.com/byte5ai/zaplex/issues/137)
- Primary ownership:
  - `app/src/workspace/view/left_panel.rs`
  - root workspace navigation and actions;
  - plus-menu construction;
  - cockpit and SSH navigation wiring.
- Model: `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-navigation-spine-target`

Scope:

- before implementation, update the authoritative spine specification at
  `docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md` with the
  latest user decision: the host section of the plus menu contains only
  favorite hosts, each with a right-opening Terminal/New Agent submenu;
- join registered SSH hosts with the live cockpit inventory deterministically
  by stable host ID rather than display name; render every host and the local
  host exactly once, enrich registered hosts with live status, keep registered
  offline hosts visible, and never route a removed registered host through a
  stale live entry;
- make hosts, projects, and sessions one coherent sidebar hierarchy;
- remove competing primary destinations for Cockpit and host overview;
- show only favorite hosts in the plus menu;
- give each favorite a right-opening submenu for terminal connection and new
  agent;
- preserve or explicitly migrate existing project, session, and launch favorite
  data without loss, even though those records no longer appear as flat
  host/agent menu rows;
- route new-agent actions through the real provider/account/project chooser;
- keep the compact account area secondary to the operational tree;
- preserve keyboard and accessibility behavior.

Required red/green coverage includes
`registered_and_live_hosts_join_once`,
`registered_offline_host_remains_visible`,
`live_status_enriches_registered_host_without_duplicate`,
`same_display_name_hosts_remain_distinct_by_stable_id`,
`local_host_appears_exactly_once`,
`removed_registered_host_is_never_routable`, and
`project_session_and_launch_favorites_survive_menu_migration`.

L6 starts only after L2, L3, L4, L5a, and L5b are integrated. It does not edit
internal SFTP widget layout. This serial boundary prevents Workspace action and
keymap conflicts with both file-manager lanes.

### L7 — Remaining cockpit and SSH premium integration

- Branch: `feat/v1-premium-ui`
- Nominal worktree: `zaplex.worktrees/v1-premium-ui`
- Base: the reviewed and integrated L6 result
- Issue: [#142](https://github.com/byte5ai/zaplex/issues/142)
- Primary ownership: remaining cockpit and SSH state, persistence and visual
  integration
- Model: `gpt-5.6-terra`, effort `high`
- Review: `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-premium-ui-target`

Scope:

- eliminate remaining cockpit and SSH clipping, overlap, transparent bleed,
  and broken split boundaries;
- use inherited Warp components, typography, spacing, dialog behavior, and
  state patterns consistently;
- keep persistence failures visible and retain the last good state;
- replace raw SSH implementation errors with understandable localized messages
  without hiding diagnostic detail;
- protect unsaved dialog edits from accidental outside dismissal;
- validate compact and narrow-window layouts.

File-manager work remains in L5a/L5b. Gate 0 loading, generation, and routing
fixes plus the ellipsis catalog guard are not reimplemented here; they run only
as integration regressions against the combined state. The remaining behavior
is specified, so Terra is appropriate for implementation and Sol for the
critical review. Because a macOS app build is prohibited locally, this lane
must create HTML mockups and headless screenshots, inspect them critically, and
compare them with the supplied screenshots and approved design documents.

### L8 — Release hardening

- Branch: `release/v1.0` for integration decisions; use a separate
  `docs/` or `chore/` lane only when the changes are independently reviewable
- Base: reviewed and integrated L1 through L7
- Issue: [#143](https://github.com/byte5ai/zaplex/issues/143)
- Mechanical documentation and version inventory: `gpt-5.6-terra`, effort
  `medium`
- Final risk decision: orchestrator on `gpt-5.6-sol`, effort `high`
- Target directory: `/tmp/zaplex-v1-release-hardening-target`

Phase A — pre-build readiness:

- update all release documentation and version locations consistently;
- audit what can fail for the first ten real users;
- scan RC/Beta wording only in shipped UI/resources and active user
  documentation;
- prepare the runtime matrix and acceptance checklist;
- complete all checks that do not require a built artifact.

After Phase A is green, stop and request fresh approval immediately before
starting CI or a DMG build.

Phase B — approved build and runtime:

- start CI/DMG only after that approval;
- verify the delivered artifact through clean install, first run, keychain,
  SSH, devhost, agenthost, Claude/Codex, restart, network loss, file
  manager, and damaged settings;
- require no visible script text, correct file-manager navigation and marking,
  reliable agent launch/resume, and coherent UI.

L8, Issue 19, and the umbrella epic close only after Phase B against the
delivered artifact and the runtime matrix are complete.

Use `max` only for a concrete unresolved release-critical decision, never as
the default session setting.

## 6. Shared hotspots

The following files and areas have one active owner at a time:

- `app/src/workspace/view.rs`
- `app/src/workspace/action.rs`
- `app/src/workspace/view/left_panel.rs`
- `app/src/workspace/view/spawn_card.rs`
- `app/src/cockpit/model.rs`
- `app/src/cockpit/pane.rs`
- `app/src/cockpit/panel.rs`
- `app/src/cockpit/capabilities.rs`
- `crates/zaplex_cockpit/src/lib.rs`
- `crates/zaplex_cockpit/src/sessions.rs`
- `crates/zaplex_cockpit/src/conductor.rs`
- `crates/zaplex_cockpit/src/types.rs`
- `app/src/sftp_manager/browser.rs`
- `app/src/sftp_manager/file_list.rs`
- `app/src/sftp_manager/sftp_backend.rs`
- `app/i18n/en/warp.ftl`
- `app/i18n/de/warp.ftl`
- `.github/workflows/pr-check.yml`
- workspace manifests and lockfiles.

When a lane needs a hotspot owned by another active lane, it stops and reports
the required interface. The owning lane lands a small reviewed interface
change first. Sessions do not make parallel "temporary" edits and resolve them
later.

Language catalogs are a known soft conflict. Every handoff lists added or
changed keys. After each integration, the catalog consistency test runs before
the next lane is based.

## 7. Commit, test, and review gates

For every logical commit:

1. identify the user-visible failure and the smallest meaningful test;
2. run the test against the unfixed behavior and record the expected failure;
3. apply the fix and run the same test green;
4. run nearby focused tests;
5. inspect the exact diff;
6. obtain an independent critical review before committing;
7. resolve every actionable finding;
8. run `git diff --check`;
9. run `cargo check` with the lane's own `CARGO_TARGET_DIR`;
10. verify `git status --short`;
11. commit with the configured identity and a conventional commit message.

Review depth:

- normal, isolated behavior: implement with Terra where appropriate and review
  with Sol at `medium` or `high`;
- data loss, credentials, money, process control, protocol, or architecture:
  Sol at `high` plus an independent second model;
- release-critical unresolved risk: Sol at `xhigh`; `max` only with a written
  reason.

No CI run substitutes for the controlled red/green proof or independent
review.

## 8. Integration strategy

- Worker branches target `release/v1.0`, never `main`.
- Start L1, L2, and L3 as the first three workers.
- Start L5a after integrated L1 and L4 only after integrated L3, using whichever
  worker slot becomes dependency-ready.
- Start L5b only after integrated L5a.
- Start L6 only after integrated L2, L3, L4, L5a, and L5b.
- Run L7 only on the reviewed and integrated L6 state.
- Run L8 last.
- Do not rebase a published branch and never force-push.
- If a branch has already been shared and needs the latest integration state,
  merge `release/v1.0` into that lane normally, resolve conflicts in the lane,
  and repeat its proofs and review.
- The orchestrator does not improvise conflict fixes directly in the
  integration lane. The owning worker receives the conflict.

Until the user grants fresh approval for remote and CI activity, worker
branches remain local. A PR must not be opened merely to "see whether CI is
green", because opening it may itself start CI. After approval, lane PRs target
`release/v1.0`; the completed release branch reaches `main` through the final
release PR. Stable tags are created only from `main` and only on explicit
release instruction.

## 9. Required lane handoff

Every worker returns:

```text
Issue:
Lane:
Branch:
Actual worktree:
Base SHA:
Head SHA and commits:
CARGO_TARGET_DIR:

Completed:
Explicitly out of scope:
Files changed:
Shared hotspots touched:

Red proof:
- exact command
- expected failure
- observed failure

Green proof:
- exact commands
- results

Independent review:
- reviewer/model/effort
- findings
- resolution of every finding

User-visible acceptance:
- exact steps
- screenshots or mockups for UI

Remaining risks:
Integration notes:
git status --short:
Push/PR/CI/DMG/release performed: no
```

The orchestrator verifies the handoff rather than accepting a worker's
"finished" claim at face value.

## 10. Orchestrator operating model

The orchestrator runs on `gpt-5.6-sol` with effort `high`. It owns scheduling,
state verification, dependencies, review gates, integration, and user
communication. It does not default to `max` and does not keep an expensive
worker alive merely to watch another session.

When `list_projects`, `create_thread`, `wait_threads`, and
`send_message_to_thread` are available, it may create and coordinate visible
Codex tasks in separate worktrees itself. These tools are verified in the
current app, but every orchestrator session checks its own available tools
instead of assuming them. It starts only from existing user-approved lane
branches, keeps no more than three workers active, waits for state changes, and
sends precise follow-ups. If any required thread tool is unavailable, it
reports that immediately and gives the user the exact lane prompts and branches
for manual task creation.

The user must still explicitly approve:

- the exact branch and worktree refs before creation;
- any push or PR that can alter remote state or trigger CI;
- every CI or DMG build at the moment it is started;
- tagging, publishing, merging to `main`, or releasing.
