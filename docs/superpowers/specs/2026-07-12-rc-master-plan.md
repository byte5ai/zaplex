# Zaplex RC master plan — the last mile to a usable Release Candidate

Status: **binding execution contract** · 2026-07-12 · derived from two independent read-only
UI/UX audits (codex + grok) that converged on the same five workstreams.

> Goal (owner's words): a real Release Candidate one can work productively with — only bugfix
> and polish left, no fundamental construction sites. Coordinated, systematic, closing the app
> concept cleanly. No more isolated single measures.

## 0. Yardstick
- `docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md` — the Nordstern (one tool,
  one object model `Host ▸ Project ▸ Session ▸ Agent`, one premium visual language).
- `docs/superpowers/specs/2026-07-11-cockpit-sidebar-redesign.md` — the sidebar redesign
  (B2/B3/B4/D1+C2). NOTE: this file was drafted earlier but is currently **not in the tree** (lost
  from the main working dir before it was committed). Its decisions survive in
  `memory/cockpit-sidebar-redesign-spec.md` and the mockup review; **re-establishing this spec in
  the tree is the opening step of WS4.**

## 1. Audit verdict (codex + grok, converged, evidence-backed)
NOT yet a "polish-only" RC. Blockers:
- **Modal fragmentation.** ≥4 parallel modal systems (`Modal<T>` `modal.rs:18`, `Dialog`
  `ui_components/dialog.rs`, cockpit hand-roll `spawn_card.rs`/`attention_inbox.rs`, SFTP
  `sftp_manager/dialogs.rs`); explicit consolidation TODO `modal.rs:110`. Even the "standard"
  dialogs (session_config, new_worktree, params, git) each re-build their own header/close/footer.
- **i18n mix is the fallback path, not a one-off.** EN = 3,233 Fluent keys, DE = 333 → ~2,900
  missing DE keys silently fall back to English; plus many surfaces bypass `t!` with hard literals.
- **Icon pass half-done.** `⚡ ◈ ◇ ⑂` are literal glyphs alongside the icon font.
- **Sidebar spine incomplete.** Built, but model/effort/% are trailing flex children, not a fixed
  right metric column → values jump horizontally (spec forbids this).
- **Offered-but-nonfunctional / unsafe (Class A).** Reachable `unimplemented!()` panic
  `cockpit/pane.rs:1725`; MCP Delete/Logs/Run are log-only stubs (`mcp_servers/list_page.rs`);
  attention-inbox remote rows are non-clickable while the same sessions are clickable in the
  sidebar (breaks one-object-model).
- **Already done (do not redo):** launch grammar, titlebar attention/pulse, spawn-card close
  affordances (✕/Cancel/Esc/click-outside now present), directory picker, detection fix,
  autoupdate repoint (byte5ai/zaplex), About build-id + splash.

## 2. Scope decisions (owner, 2026-07-12)
- **i18n depth = prioritized.** Complete German on all primary user-facing surfaces now + a rule
  banning hard UI literals; the deep inherited-Warp long tail gets a reviewed bulk-translate pass,
  completed over time. Not all 2,900 keys gate the RC.
- **Sidebar = full redesign now.** Implement B2/B3/B4/D1+C2 from the sidebar spec (two zone-cards,
  worktree/branch session identity, provider≠plan incl. data-model fix, 5h+week meters, detail in
  the dashboard pane), not just a normalization.

## 3. The five workstreams (ordered; WS1 is the foundation)

### WS1 — One binding modal/dialog contract, then migrate every dialog
Define ONE contract and a single reusable frame (extend `Modal`/`Dialog` or a thin shared
`cockpit`-aware wrapper): scrim (one `modal_scrim`), corner radius, width band, title typography,
✕ close (`ActionButton` + `Icon::X`), Escape, click-outside policy, footer order + button
components. Migrate the foreign bodies — **spawn card**, **attention inbox**, **enum-creation
card** — and normalize `session_config` / `new_worktree` / `params` / `git` / `sftp` onto it.
Retire the per-dialog hand-rolled headers/scrims. (Supersedes the ad-hoc ✕/scrim just added to the
spawn card.) Kills audit items M1–M9, V1–V2, V14.

### WS2 — i18n: DE+EN clean, no hard literals
1. **Rule + guard:** every user-facing string via `crate::t!` / `t_static!`; add a CI/grep guard
   that flags bare string literals in render paths.
2. **Primary-surface German completion:** cockpit sidebar runtime strings (`panel.rs`), all
   migrated dialogs (WS1), `cockpit/settings.rs` descriptions, git-dialog copy, main settings
   pages, menus, onboarding, the `spawn_card` remote-dir placeholder, add DE for
   `workspace-left-panel-ssh-manager-connecting`.
3. **Long tail:** reviewed bulk-translate pass for the remaining inherited keys (post-RC-safe).
Kills I1–I7.

### WS3 — Finish the icon pass
Replace chrome/action glyphs `⚡ ◈ ◇ ⑂` with `icons::` font glyphs (host-persistence mark, review
verb, dashboard-expand, fork); remove `⚡`/`📁` text from `warp.ftl`. Keep the intentional status
vocabulary `● ✋ ◦` (spine §2.5). Kills IC1–IC2, IC4.

### WS4 — Full sidebar redesign (B2/B3/B4/D1+C2)
Per `2026-07-11-cockpit-sidebar-redesign.md`: two flat zone-cards (Hosts tree + AI-Accounts),
**fixed right metric column** (no horizontal jump), status dot inherits worst child state, session
identity = worktree/branch, provider≠plan (with the data-model fix for Codex `plan_tier`), 5h+week
meters, click→detail in the dashboard pane, hover = colour only (no layout-shift). Kills V3, V5,
C1, and the spine metric-column defect.

### WS5 — Remove offered-but-nonfunctional / unsafe interactions
"Works or is not offered." Implement or safely remove: the `cockpit/pane.rs:1725`
`unimplemented!()` overflow path; MCP Delete/Logs/Run (`mcp_servers/list_page.rs`); reconcile the
attention-inbox remote-row affordance with the sidebar (same object, same grammar); the blind
`AddSpecificAgentTab` when cockpit disabled. Kills C2(panic)/16/17, C4.

## 4. Execution model
- Order: **WS1 → (WS2 ∥ WS3 ∥ WS5) → WS4 → verification.** WS1 first so dialogs are translated
  once in their final structure. WS4 last (largest, self-contained).
- **codex + grok** are used read-only for design review and adversarial verification of each
  workstream's diff (autonomous write-mode is intentionally not used); implementation + the
  `cargo check --bin zaplex --locked` gate are owned here.
- **Batch, not churn:** no per-change DMG. Exactly **one** RC DMG at the end for runtime
  acceptance on devhost. `exports/` holds only that one DMG.
- Each workstream lands as its own commit(s) on the RC branch with a green `cargo check`.

## 5. RC acceptance criteria
- One modal language: every dialog uses the shared frame (scrim/radius/width/close/Esc/footer);
  no hand-rolled modal chrome remains.
- Locale is consistent on all primary surfaces: `rg`-clean of bare UI literals in render paths;
  German locale shows zero English on cockpit, dialogs, menus, main settings.
- `rg '⚡|◈|◇|⑂' app/src` → zero chrome/action hits (status vocab `● ✋ ◦` may remain).
- Sidebar matches the redesign spec incl. fixed metric column (no horizontal jump); provider≠plan.
- No reachable `unimplemented!()`; every offered action functions or is hidden by capability.
- Runtime-accepted on a single RC DMG (About shows the RC build id).

## 6. Progress ledger
- [x] WS1 modal contract + migrations — `ui_components/modal_frame.rs` (one scrim /
  radius 10 / width band / padding / header / close ✕ / dismiss policy); migrated
  Spawn-Karte, attention inbox, session_config, new_worktree, params headers + enum
  shell; aligned generic `Modal<T>` tokens. Reviewed read-only by codex + grok (no
  blockers; both should-fix items applied). Branch `rc/master-plan`, all commits green.
- [ ] WS2 i18n (rule+guard, primary German, bulk tail) — NOT STARTED. Full worklist
  scoped (primary-surface literals in `cockpit/panel.rs`, `cockpit/settings.rs`, the
  migrated dialogs, `git_dialog/*`, `ai_page.rs`, `native_modal.rs`; missing DE key
  `workspace-left-panel-ssh-manager-connecting`; EN 3233 vs DE 333 keys). Deferred
  intentionally rather than half-done (a partly-translated locale is more incoherent
  than a cleanly-scoped not-yet-started one).
- [x] WS3 icon pass — `⚡ ◈ ◇ ⑂` → `icons::*` (Eye/History/GitBranch/Lightning) via the
  new `icon_word_verb` helper; stripped glyphs from `warp.ftl` (en+de) + doc comments.
  Status vocab `● ✋ ◦` kept. Acceptance `rg '⚡|◈|◇|⑂' app/src` → 0. codex-reviewed, green.
- [ ] WS4 sidebar redesign — NOT STARTED. Blocked on two things: (a) the lost
  `2026-07-11-cockpit-sidebar-redesign.md` spec must be re-established in the tree
  (WS4's opening step), and (b) per the owner's standing rule, a UI/UX redesign of this
  size needs the design concept + explicit approval **before** code. Recommend: re-draft
  the spec, get sign-off, then build.
- [~] WS5 nonfunctional/unsafe removal — PARTIAL. Fixed the one Class-A **safety** item:
  the reachable `unimplemented!()` panic in `cockpit/pane.rs` → no-op. The remaining
  items are non-crashing and need runtime verification to fix safely (better done against
  the RC DMG): MCP Delete/Logs/Run are log-only *defensive* arms whose button visibility
  is status-derived (gating needs runtime confirmation of what's actually offered per
  item type); the attention-inbox remote-row ↔ sidebar reconcile is feature wiring; the
  cockpit-off blind `AddSpecificAgentTab` (C4) is an intentional, documented fallback
  (Class B).
- [ ] Verification: one RC DMG, devhost acceptance
