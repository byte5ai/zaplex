# Cockpit sidebar redesign (B2 / B3 / B4 / D1+C2)

Status: **implemented; compact-row amendment approved 2026-08-12**. This file is
the re-established WS4 opening step for the RC master plan
(`2026-07-12-rc-master-plan.md`); the original 2026-07-11 draft was lost from the
working tree before it was committed. Reconstructed faithfully from the
converged design decisions.

> Trigger: the owner vetoed the rendered cockpit on the `v0.rc-spine` DMG (main
> `8a7f94eb`): a chaotic host↔agent style mix, an overloaded status line, a false
> "No agent CLI installed", EN/DE mixing, and ⚡ emoji despite the #107 icon pass.
> This spec is the systematic answer, converged with the owner over mockups
> B → B2 → B3 → B4 → D1+C2.

## 0. Already addressed by RC WS1–WS3 (do not redo)
The RC branch `rc/master-plan` has, since this spec was first drafted, landed:
- **S0 detection bug — FIXED** (CLI detection reaches the UI; the Spawn-Karte now
  observes `ScanComplete`). The "No agent CLI installed" false-negative is gone.
- **⚡ / 📁 / ◈ / ◇ / ⑂ glyphs → icon font** (WS3): the SSH host-persistence mark
  is `Icon::Lightning`; review/log/fork verbs are `Eye`/`History`/`GitBranch`.
- **i18n**: the sidebar's runtime metric strings and the Spawn-Karte placeholder
  are now translated (WS2); the modal chrome is unified (WS1).

So WS4 is now purely the **layout/structure/data-model** redesign below.

## 1. Goal
One calm, glanceable sidebar that reads as **one** object model — `Host ▸ Project
▸ Session` — with a fixed, non-jumping metric column, where emphasis comes from
**content**, not from nested containers. Selection opens detail in the dashboard
pane; the sidebar itself stays a glance surface.

## 2. Converged decisions (binding once approved)

### 2.1 Two flat zone-cards
Two flat cards (`surface_1`, 0.5px border, radius 12, **no** shadow):
- **Hosts** (top) — the object tree `Host ▸ Project ▸ Session`.
- **AI-Accounts** (bottom) — the account/usage cards.
Emphasis via content and spacing, never via heavy container chrome.

### 2.2 Session identity = worktree / branch (not model)
A session row is identified by its **worktree / branch**, not its model — this is
what disambiguates several parallel `opus` agents. **Data-model change:** the
snapshot must expose worktree/branch, not just `cwd`.

### 2.3 Density D1+C2 — one fixed line per session
Each session is a single fixed line: `Dot · Branch · Provider-icon · Model ·
ctx% · State`. `waiting` rows are always expanded; `running`/`idle` collapse
behind a summary ("N running · M idle — show") that expands **on click**.

### 2.4 Provider ≠ Plan (two fixed slots)
Always render `Provider · Plan` as two fixed slots. **Data-model fix:** Codex
currently mis-reports `plan_tier = "chatgpt"` (`codex.rs:86`) — the provider name
leaks into the plan field. Separate them.

### 2.5 Accounts
Each account shows **both** a 5h and a week meter; the provider icon leads; spend
is rounded, today's on the right. The fleet total lives in the header
(today · week).

### 2.6 Interaction
Click → selection (stable highlight) → detail in the **dashboard pane**. The
reset timers live only in the pane, not the sidebar row.

### 2.7 Hover rule (binding)
Hover changes **colour / fill only** — never a layout-shifting reveal. The list
must never jump on hover. (This is the rule the owner called out on the vetoed
build.)

### 2.8 Compact-row width priority (binding amendment, 2026-08-12)
The host identity receives the entire flexible width. Status, count and
secondary actions use fixed slots. Repeated secondary actions are icon-only:
`AiAssistant` opens an agent, `Folder` opens files, and `DotsHorizontal` manages
the host; their localized text appears in tooltips, never as repeated row
labels. Favorite and secondary actions share the same 20 px square and hover
grammar. At the 250 px minimum width, the host name may ellipsize only after
the fixed slots have been reserved, and hover must not change geometry.

## 3. §12 — supaterm learnings folded in (additive; §0–§2 unchanged)
- **L1** computed semantic palette with a contrast test (generalize `heat_coloru`;
  fold into the tokens step).
- **L2** attention co-located and the loudest row colour at every collapse level.
  Refinement: *calm = slow / low-contrast, not "no motion"* — a glow may move, but
  quietly.
- **L3** peek popover as a **fixed-size floating** element (respects the hover
  rule; three tiers glance → click → pane; #115).
- **L4** the structured plan list is the primary activity surface in the pane;
  the transcript is drill-down only (#114).
- **G** guardrail: *keep every agent visible without losing the terminal*.

Deliberately out of scope (their own design items): pane/tab glow (#120),
spaces + tab pinning (#117), per-event notifications (the ambient bit stays).

## 4. Code anchors (branch `rc/master-plan`; line numbers approximate after WS1–3)
- Sidebar: `app/src/cockpit/panel.rs` — `render_header`, the conductor tree, the
  account `render_card`, `heat_bar`.
- Tokens: `app/src/cockpit/style.rs` — `heat_coloru`, `icon_verb_button`,
  `icon_word_verb`, `GLYPH_COL_WIDTH`.
- Compact row actions: `app/src/ui_components/compact_row_action.rs` — the
  icon-only API shared by Cockpit and SSH-manager rows.
- Format: `crates/zaplex_cockpit/src/format.rs` — `binding_window`, `format_reset`.
- Pane (detail target): `app/src/cockpit/pane.rs`.
- Data model: the `zaplex_cockpit` snapshot types (worktree/branch exposure; the
  Codex `plan_tier` fix in `codex.rs`).

## 5. Build order (once approved)
S1 tokens (fold in L1) → S2 data model (worktree/branch + provider≠plan) →
S3 Hosts card → S4 Accounts card → S5 pane detail → S6 polish (empty states,
scrim) → S7 runtime acceptance on the RC DMG.

## 6. Approval history
The original redesign was approved and implemented on the V1 line. The
compact-row amendment in §2.8 was explicitly requested on 2026-08-12 after the
first V1 start exposed repeated text actions crowding out host identities. This
user-visible correction advances the source version to 1.0.1 under the project
versioning policy; it does not release 1.0.1, whose runtime matrix remains open.
