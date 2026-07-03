# Agent Session Fork & Worktree Launch — "try the other approach" as a first-class verb

> **Status:** F1+W1+FW implemented (local fork, fork-into-worktree, cockpit session-row surfaces) — 2026-07-03. Fork capability verified against both CLIs: `claude --resume <id> --fork-session`, `codex fork <id>` (F2 mechanics ready in `CLIAgent::fork_command`; a Codex surface waits on Codex session discovery). Open: W2 remote worktree via daemon `startup_command`, block-header surface. Runtime acceptance pending the next user test build.
>
> **Quelle der Entscheidung:** User 2026-07-03 — adapt the two session-UX ideas from the Gitlawb/zero analysis (`zero exec --fork <session-id>`, `zero exec --worktree`); gap analysis `2026-07-03-claudeplex-gap-analysis.md` lines 49-50 already lists fork-resume as a small open gap ("Branch a copy … an Transcript-/Session-UI andocken").
>
> **Direktive:** Fork operates on the **agent conversation**, not the PTY — a daemon-PTY "fork" has no meaningful semantics and is explicitly out of scope. Worktree launch reuses the inherited tab-config worktree machinery and the daemon `startup_command` seam; no new daemon protocol messages.
>
> **Goal:** From any known agent session, one action branches a copy of the conversation into a new block — optionally into a fresh, isolated git worktree — so trying a second approach never risks the first one.

---

## 1. The two verbs (borrowed semantics, zaplex-native mechanics)

Zero models a workflow truth we want: *exploration should be cheap.* Its `--fork` re-enters a past session divergently; its `--worktree` isolates a run's file effects. In zaplex both verbs become **launch-time options on existing surfaces** (Agent Tree / adopt sidebar / block header / future C4 wizard), not new modes:

- **Fork** = resume a copy: new block, same conversation history, divergent future. The original session keeps running (or stays idle) untouched.
- **Worktree launch** = run the agent in a fresh `git worktree` of the repo, local **or on a remote host inside a daemon session**. Combined ("fork into worktree"), this is the killer move: second opinion on the same conversation without touching the first approach's working tree.

## 2. Fork — per provider, honest capability

| Provider | Mechanism | Verify at build |
|---|---|---|
| Claude | `claude --resume <session-id> --fork-session` with the account's `CLAUDE_CONFIG_DIR` | flag exists in current CLI (gap doc cites it; confirm exact name/behavior) |
| Codex | resume-by-id equivalent + fork semantics | flag unverified (concept §10.2 already flags Codex resume for verification) — if absent: **no fake fork**; surface disabled with hover reason (§11.9) |
| zero (once Z1 of `2026-07-03-zero-agent-integration-design.md` lands) | `zero exec --fork <session-id>` / `zero sessions fork` | payload of session-id in hooks |

Mechanics: fork is a *launch* with a prefilled command — it flows through the existing launch path (`add_tab_with_specific_agent` / startup-command injection), inheriting account pinning (`CLAUDE_CONFIG_DIR`) from the source session. For sessions on resilient hosts, the fork opens as a **new daemon session** on the same host with the fork command as `startup_command` — persistence from birth. No daemon protocol change: a fork is just a new session whose startup command happens to reference an old conversation.

**Surfaces (in priority order):**
1. Context action **"Fork session"** in the cockpit session list / adopt sidebar (C3a spine already knows session-id, cwd, account).
2. Block-header menu on a running agent block.
3. C4 Launch Wizard: "Start from: fresh | resume | fork of …" (folds into the existing planned fresh-vs-resume choice — one mental model, not a new one).

## 3. Worktree launch — local and remote

**Local:** reuse the inherited tab-config machinery as-is — `build_worktree_config_toml` already emits `git worktree add -b <branch> <path> <base>` as a tab setup command (`app/src/tab_configs/`). The agent-launch path gains an "isolated worktree" toggle that routes through this existing config instead of duplicating it.

**Remote (daemon sessions):** the daemon knows nothing about git and stays that way. Composition happens client-side in the `startup_command`:

```
git worktree add -b <branch> <path> <base> && cd <path> && <agent-cmd>
```

- **Path convention:** `<repo>.worktrees/<branch-slug>/` as a sibling of the repo (matches the maintainer's host convention, is compatible with `git worktree prune`, and is visible/debuggable on the host — unlike a hidden `~/.zaplex/worktrees` tree). One fixed convention, no setting (anti-speculation, §11).
- **Precondition check, honest:** before offering the toggle for a remote cwd, verify the cwd is a git repo (cheap `git -C <cwd> rev-parse` probe over the existing transport). Not a repo → toggle disabled with reason, never a broken session.
- **Failure mode:** if `git worktree add` fails, the session shows the real error in the block (it *is* the session's first output) — no swallowing, no auto-retry.
- **Branch naming:** reuse `autogenerate_branch_name` from `new_worktree_modal.rs` (same names locally and remotely — one convention).

**Cleanup (explicitly deferred):** worktree lifecycle management (prune on session close, "abandoned worktree" listing) is real but separate; this design only *creates* worktrees where the user asked for isolation. A follow-up design owns cleanup once usage patterns are visible. Until then the FM pane and plain shell make worktrees inspectable/removable by hand — stated in docs, not hidden.

## 4. Milestones & acceptance

| Step | Scope | Abnahme |
|---|---|---|
| F1 | Fork for Claude from cockpit session list + block header (local + daemon hosts) | fork of a waiting session opens a new block with full history; original session unchanged; account pinning inherited |
| F2 | Codex fork behind capability verification | if flag exists: parity with F1; if not: action visibly disabled with reason |
| W1 | Local worktree launch toggle on agent launch | agent starts in fresh worktree branch; repo working tree untouched |
| W2 | Remote worktree launch via daemon `startup_command` + repo probe | on a resilient host: toggle → daemon session in new sibling worktree; non-repo cwd → toggle disabled |
| FW | "Fork into worktree" combined action | one action from a session → new block, same history, isolated worktree |

## 5. Risks & open questions

- **Flag drift:** `--fork-session` (Claude) and Codex resume/fork flags must be verified per release; capability check degrades honestly, never guesses.
- **Worktree on dirty/locked repos:** `git worktree add` fails loudly on conflicts — acceptable (error is visible in-block); no pre-flight dirty-check in v1.
- **C4 dependency is soft:** F1/W1/W2 attach to existing surfaces (session list, block header, tab configs) and do not block on the C4 wizard; C4 later absorbs them as launch options.
- **Interaction with shared cargo/build caches in worktrees** (project memory: shared target dir): agent builds in sibling worktrees inherit the host's own setup — zaplex does not manage build caches; out of scope.

## 6. Code seams (verified 2026-07-03, worktree `feat/stage2-client-attach`)

- `app/src/tab_configs/tab_config.rs` — `build_worktree_config_toml`, `generated_worktree_repo_dir` (local worktree machinery)
- `app/src/tab_configs/new_worktree_modal.rs` — `autogenerate_branch_name`
- `app/src/tab_configs/session_config.rs` — `enable_worktree`, `WORKTREE_BRANCH_PARAM`
- `app/src/terminal/daemon_tty/terminal_manager.rs` — `OpenSessionParams.startup_command` (remote composition seam)
- `app/src/terminal/daemon_tty/event_loop.rs` — `on_session_opened` (startup command injection)
- `app/src/workspace/view.rs:3984` — `add_tab_with_specific_agent` (launch path)
- `crates/zaplex_cockpit/src/sessions.rs` — session-id/cwd/account source for fork surfaces
- `app/src/remote_server/headless_connect.rs` — transport for the remote repo probe
- Gap doc: `2026-07-03-claudeplex-gap-analysis.md` lines 49-50 (fork-resume ❌)
