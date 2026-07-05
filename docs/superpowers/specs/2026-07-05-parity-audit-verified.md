# Parity audit — claudeplex + claudeplex-desktop → zaplex (VERIFIED, 2026-07-05)

> Supersedes the estimated status in `2026-07-03-claudeplex-gap-analysis.md`.
> Every verdict below was checked against current `main` (`60e9df58`) with file:line
> evidence (3-way code audit). Legend: ✅ present (natively) · 🟡 partial · ❌ absent.
> **Strictness rule applied:** "the data type/primitive exists" ≠ "the feature is wired
> into the UI / user flow" — many cockpit signals are computed in the spine but not yet shown.

## Honest bottom line

**No — not yet full parity.** Of ~56 reference features: **~22 ✅ · ~15 🟡 · ~19 ❌.**
Roughly: the **read side + tools are at parity or better**; the **orchestration side (C4),
the whole GitHub block (C5), and several polish items are missing**, and a cluster of cockpit
signals are computed but **not yet wired into the UI**.

**zaplex already BEATS the reference** in: daemon-based persistent remote sessions (survive
lid/drop, offline auto-install — vs tmux fleet); host↔host file copy incl. remote↔remote +
directories, no scp dance; **native remote view/edit editor** (reference has none); Codex
support (reference is Claude-only).

## Verdicts by category (deltas vs the 2026-07-03 doc called out)

### (a) Account / Usage / Cost — 6 ✅ · 2 🟡 · 1 ❌
- ✅ Account discovery (Claude+Codex), token accounting 5h/today/week+cost, reset timers,
  soft budgets, **real per-account OAuth usage** (↑ was ❌; `app/src/cockpit/oauth.rs`),
  **plan-based budget estimate** (↑ was ❌; `windows.rs:140`, 4 coarse buckets, `~` marker not "est" text).
- 🟡 **Per-model bars (Opus/Sonnet):** parsed from the endpoint but **dropped** — not stored in
  `AccountUsage`, not rendered (`oauth.rs:107`). Aggregate header: has cost + "waiting" count,
  **no live/working counts**.
- ❌ `instances.json` overrides (label/color/order/hide).

### (b) Session monitoring — 2 ✅ · 6 🟡 · 0 ❌  ("heart of claudeplex")
- ✅ Live registry read (pid-alive) + status derivation active/**waiting**/monitor.
- 🟡 **The spine computes it, the UI doesn't fully show it:** account-status enum (derived, **not
  read in UI**, no IDLE), context-fill/model per session (populated, **not rendered**),
  last-activity (derived, **not rendered**; no output-text tail — **privacy invariant** forbids
  reading content). Waiting-first list ✅ **but no focus/jump action** (rows offer fork, not focus).
  Notification active→waiting ✅ as a toast, but **no per-card flash**. FS-watch is hybrid
  (home-watcher + 45 s poll). No distinct "stale" state (dead sessions filtered, not labelled).

### (c) Agent control — 4 ✅ · 1 🟡 · 4 ❌
- ✅ Interactive PTY, send/keys/menus, **image attach** (↑ was 🟡; paste+drag both wired),
  **fork "branch a copy"** (↑ was ❌; `--fork-session`, cockpit "⑂ fork/+worktree").
- 🟡 Rename = zaplex tab-label only, **no native `claude --name`**.
- ❌ Adopt idle CLI session (`--resume` in-place; daemon-adopt is a different mechanism),
  watch-mode (read-only follow), restart/fork-resume (plain `--resume`), CLI-agent persistence
  across app restart.

### (d) Multi-host / remote — 4 ✅ · 1 🟡 · 3 ❌
- ✅ Persistent daemon sessions (**better**), SSH host mgmt + `~/.ssh/config` import, shell-out PTY,
  **host↔host copy incl. dirs + remote↔remote** (↑ was 🟡; `relay_remote_to_remote`, **better**).
- 🟡 RAM governor: per-session ceiling + GC + global 256 MB host cap all exist, **no UI display**.
- ❌ **Tailscale discovery**, **Conductor tree** (Host▸Project▸Session + needs-me bubbling),
  **remote-fleet aggregation** (cross-host `list_sessions`).

### (e) Git / GitHub — 0 ✅ · 2 🟡 · 4 ❌  (**entirely open, = C5**)
- 🟡 Repo identity: `.git` detected for fork-into-worktree, but owner/repo/worktree **not shown**
  in the cockpit. Lenient JSON parsing exists but wired to zaplex's own AI pipeline, **not** to any
  GitHub flow.
- ❌ Quick-Issue, PR-Review, Issue-Triage (instance-driven via `gh`), freest-instance picker.

### (f) Launch / routing — 1 ✅ · 2 🟡 · 3 ❌  (**= C4, in progress**)
- ✅ "Ask my agent" (`AskAgent`).
- 🟡 Account pinning = **fork-only** (`fork_command_pinned`), no general pinned launch, **no
  API-key scrub**. Fresh-vs-resume primitives exist only in narrow harness/SDK paths.
- ❌ **New-agent wizard** ("on Host in Dir"), quick-launch folder-history + completion,
  **launch-on-freest / `pick_freest`** (heat computed, never used to route).
- (Title-bar Claude/Codex icons confirmed **de-listed by default**, user-enableable — intentional
  pending the C4 launcher.)

### (g) Transcript / history — 0 ✅ · 0 🟡 · 3 ❌  (**POLICY divergence, not a bug**)
- ❌ CLI transcript parser (jsonl→turns), transcript viewer (markdown/diffs/live/watch/intake),
  folder-history per account. **By design:** zaplex's cockpit privacy invariant reads token
  **counts only, never content** — so a full content transcript-viewer would *contradict* a
  deliberate policy. Reaching "≥ parity" here is a **product decision** (relax the invariant?),
  not just unbuilt work. `ConversationListView` covers only in-app agent-mode conversations.

### (h/i) Shell / misc — 5 ✅ · 1 🟡 · 1 ❌
- ✅ Command palette, toolbelt, themes, markdown, install/update (DMG + autoupdate).
- 🟡 Clipboard PNG: attached as AI context / Ctrl+V sim, **no literal write-PNG-to-file-then-path**.
- ❌ **i18n German** (only English ftl; `"de"` falls back to en).

## What "reaching ≥ parity" now requires, clustered

1. **Finish wiring the cockpit UI** (smaller — spine already computes it): per-model bars,
   account-status glyphs, context-fill, last-activity, per-card flash, a **focus/jump-to-session**
   action, live/working header counts. (b) + (a#7,9).
2. **C4 — Launcher / plexing engine** (in progress): wizard, general pinned launch + API-key scrub,
   `pick_freest`, folder-history, resume/rename. Closes (f) + (c#4,7,8) + the freest-picker in (e#13).
3. **C5 — GitHub flows** (entirely new): Quick-Issue, PR-Review, Issue-Triage. Closes (e).
4. **Remote breadth:** Conductor tree, Tailscale discovery, fleet aggregation, RAM display. (d).
5. **Policy call on (g):** decide whether the privacy invariant (counts-only) stays — if yes,
   transcript-viewer parity is *intentionally* out of scope; if relaxed, it's a C3/C5 build.
6. **Polish:** watch-mode, native rename, clipboard-PNG→file, i18n-DE, `instances.json` overrides.

## Method / caveats
Verified by a 3-way code audit (categories a/b/g · c/f · d/e/h/i) against `main 60e9df58`,
each verdict with a file:line anchor. Counts are approximate (a few items span categories).
This reflects *code presence*, not runtime behaviour on real hardware (devhost acceptance still pending).
