# Cockpit Sidebar & Dashboard-Pane — visual-language redesign

Status: **design spec, awaiting user sign-off before implementation** · 2026-07-11 · converged
over an iterative concept review with the user (mockups B → B2 → B3 → B4 → D1+C2).

> This spec follows the process rule the user set on 2026-07-11 (see
> `memory/feedback-ui-ux-concept-first.md`): **every UI/UX change gets a concept + plan first,
> one system and one strategy, then sign-off, then code.** It extends — does not replace — the
> Nordstern in `docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md` (the object
> model `Host ▸ Project ▸ Session ▸ Agent {model·effort·context·state}` and "one premium visual
> language" stay binding). It turns the still-open "polish debt / D7" of that doc into a concrete,
> code-bound design.

## 0. The one sentence

The left panel today reads as **two design languages stacked** — airy text rows for hosts,
chunky filled cards for accounts — under a cryptic, label-less summary line. The redesign makes
it **one calm system with two role-cards** (`Hosts` = the object tree · `AI-Accounts` = accounts +
usage), unified by shared tokens, with detail reached by **click → Dashboard-Pane**, never by
layout-shifting hover.

## 1. Problems in the shipped build (evidence-backed, main @ `8a7f94eb`)

| # | Problem (user veto 2026-07-11) | Code |
|---|---|---|
| P1 | Host rows vs account "cards" = two visual languages in one column | `panel.rs:354` `render_conductor` (transparent, borderless rows) vs `panel.rs:226` `render_card` (`fg_overlay_1` fill, radius 6, padding 8, dense) |
| P2 | Summary line overloaded, unlabeled, cramped | `panel.rs:708` `render_header` — `SpaceBetween` row jams count + `"$X 5h · $Y wk"` + expand affordance with no labels/spacing |
| P3 | `Max` (a **plan**) and `chatgpt` (a **provider**) shown in the same badge slot — two different axes conflated | `render_card` badge = `acct.account.plan_tier`; Codex returns `plan_tier = "chatgpt"` (`crates/.../codex.rs:86`) |
| P4 | Sessions under a project shown only by model → parallel `opus` agents in different worktrees are indistinguishable | conductor rows label by agent/model, not by the session's worktree/branch/dir |
| P5 | Account rows are clickable (open dashboard) but nothing signals it | whole card is a `Hoverable`/`PointingHand` (`panel.rs:337`) with no visible affordance |
| P6 | Emojis remain despite the "#107 premium icon pass" | `⚡` at `ssh_manager/panel.rs:2200` (host persistence mark) and `spawn_card.rs:752` (`⚡ Freest`); #107 only touched the spine tree glyphs (★/⋯/＋) |
| P7 | Mixed EN/DE UI | `spawn_card.rs` hardcodes English literals, bypassing the `t!()` i18n that the sidebar uses → German elsewhere, English here |
| P8 | "No agent CLI installed" false-negative + contradictory launch form | `spawn_card.rs:203` `installed_agents` reads `cfg.{claude,codex}.installed`; detection `cli_agent.rs:862` `cli_agent_search_dirs` probes `$PATH` + a hardcoded allowlist that misses the user's install location (macOS GUI-PATH) |

P7/P8 are tracked here because they belong to the *same* "one system, one strategy" coherence,
but P8's detection fix is a pure correctness bug and ships on its own track (see §9).

## 2. Design decisions (converged, user-approved concept)

1. **One container language: two flat zone-cards.** `Hosts` on top, `AI-Accounts` below. Each is a
   **flat tile** — `surface_1` fill, `0.5px` outline, radius `12`, **no shadow/elevation** (CDS:
   "dense lists = bordered rows, not raised cards"; keeps two stacked cards from reading heavy).
2. **Hosts card = the object tree** `Host ▸ Project ▸ Session`. Emphasis between the two cards
   comes from **content** (accounts carry meters + spend), not from one being a card and the other
   not.
3. **Session identity = its worktree/branch** (fallback: directory), not its model. Model · effort
   · ctx · state are metadata on the same line.
4. **Density = D1 + attention-first (C2).** Every session is a **fixed single line** (no reflow).
   `waiting` sessions always shown; `running`/`idle` fold into one faint "N läuft · M idle —
   anzeigen" line, expanded by **click** (never hover).
5. **Provider ≠ Plan.** Two consistent slots everywhere: Provider (`Claude` / `ChatGPT`) leads
   (icon + name); Plan (`Max` / `Pro` / `Plus` / `Free`) is a separate chip. Order always
   `Provider · Plan`.
6. **Accounts show both rolling windows.** Each account renders a `5h` meter **and** a `Woche`
   meter, each with its own %.
7. **Clickability is visible.** Interactive rows get a hover tint **and** a trailing `›`; click
   selects (stable highlight) and shows full detail in the **Dashboard-Pane**.
8. **Hover rule (binding).** Hover may change **color/background only** — never reveal data that
   changes size/position and shifts following content. Detail reveal = explicit click or the pane.
9. **Status dot inherits the worst child state.** A host/project dot is amber if any descendant is
   `waiting`, green if any is `working`, else grey — attention is visible while collapsed.
10. **No emoji in chrome.** All UI-chrome glyphs come from the icon font (`icons::Icon` via
    `style::icon_verb_button`), including the host-persistence mark and the freest-account mark.
11. **All user-facing strings via `t!()`.** No hardcoded UI literals; both `de` and `en` complete.

### Confirmed defaults (user, 2026-07-11)
- **Global summary** stays, as a **fleet total** (today + week summed over all accounts), in the
  `AI-Accounts` header — labeled, spaced.
- **Reset timers** (`5h ↻ / wk ↻`) move to the **Dashboard-Pane only**; out of the sidebar row.
- **Spend** is **rounded** (`$1000`), scoped to **today**, right-aligned on the account row.

## 3. Visual-language system (build on `style.rs`, do not invent a parallel set)

**Container tokens.** New shared helper `style::zone_card(child)` → `Container` with
`theme.surface_1()` bg, `Border::all(0.5)` in `theme.outline()`, radius `12`, uniform padding
`10`, `margin_bottom` `CARD_SPACING*2`. No shadow. Used by both zone cards.

**Row anatomy (one grammar for host / session / account rows).**
```
[ leading glyph ] [ primary label (flex, ellipsis) ] [ inline meta (muted) ] [ trailing state / value ] [ ›(hover) ]
```
- leading glyph: status dot (`style::heat_coloru`/attention) for host/session rows; provider icon
  for account rows.
- fixed leading column width `GLYPH_COL_WIDTH` (`style.rs:34`) so every row aligns.
- hover: background `fg_overlay_1`, cursor `PointingHand`, trailing `Icon::ChevronRight` faded-in
  via **opacity/color only** (no width change).

**Color / heat.** Reuse `style::heat_coloru(HeatLevel)` (single source, `style.rs:57`) and
`HeatLevel::from_fraction` bands (`format.rs:24`). Dots: green = working, amber = waiting
(`attention_coloru`, `style.rs:71`), grey = idle (`disabled_text_color`).

**Typography.** `ui_font_subheading` for zone labels + primary labels; `ui_font_body` for meta;
one muted color role (`sub_text_color`). No new sizes.

**Icons (retire emoji).** Provider: `Icon::` marks for Claude / ChatGPT (add if missing).
Host-persistence mark: replace the literal `"⚡"` (`ssh_manager/panel.rs:2200`) with an
`Icon::Bolt`-class glyph via `icon_verb_button`. Freest mark: replace `"⚡ Freest"`
(`spawn_card.rs:752`) with icon + `t!()` label. Expand: `Icon::ChevronRight/Down`. Folder:
`Icon::Folder`.

## 4. Component specs

### 4.1 `Hosts` card — `render_conductor` rewrite (`panel.rs:354`)
- Header: `t!("cockpit-hosts")` left, `Icon::Plus` (→ `WorkspaceAction::AddSshHost`) right. The old
  free-standing `＋ Host hinzufügen` row and the `render_header` summary block move out of the
  conductor.
- **Host row:** `[chevron] [status-dot(inherited)] [name] [★/⋯ on hover] [count]`. Star/⋯ keep
  their current actions (`star_button` `panel.rs:441`, `ManageSshHost` `panel.rs:462`) but appear
  only as a color-hover, never shifting layout. Registered hosts stay roots even with no live agent
  (unchanged from #100).
- **Project row:** `[indent] [Icon::Folder] [project name]` (muted). Group key = repo/dir.
- **Session row (D1, single fixed line):**
  `[indent] [status-dot] [branch/worktree (flex, ellipsis)] [provider-icon] [model] [ctx%] [state]`.
  - identity = worktree/branch (see §5 data model); model = `opus`/`gpt-5`; ctx% from context fill;
    state = `wartet`/`läuft`/`idle` colored by heat/attention.
  - effort + usage + reset are **not** on this row → Dashboard-Pane.
- **Attention-first fold (C2):** within a project, render every `waiting` session; collapse
  `running`+`idle` into one faint row `t!("cockpit-sessions-folded", n_running, n_idle)` +
  `anzeigen`, expanded on click. Extends the existing `host_auto_collapsed` / `SIDEBAR_MAX_ROWS_PER_HOST`
  (`panel.rs:36,389`) logic to the session level.

### 4.2 `AI-Accounts` card — `render_header` + `render_card` rewrite (`panel.rs:226,708`)
- Header: `t!("cockpit-ai-accounts")` left; **fleet total** right —
  `t!("cockpit-usage-fleet", today, week)` → e.g. `heute $1000 · Woche $8 593` (labeled, spaced),
  summed over accounts (the existing `cost5h`/`cost_wk` sums, relabeled today/week).
- **Account row (fixed height, two sub-lines):**
  - line 1: `[provider-icon] [account label (flex)] [Provider · Plan (muted)] [spend today (rounded)] [›]`
  - line 2a: `[5h] [meter] [%]`
  - line 2b: `[Woche] [meter] [%]`
  - meters reuse `heat_bar` (`panel.rs:178`) but pinned to fixed windows (5h, week) instead of the
    `binding_window` "fullest of N" pick (`format.rs:90`). Provenance/heat coloring unchanged.
- Whole row selects → Dashboard-Pane (keep `OpenDashboardPane`, add visible `›` affordance).

### 4.3 Dashboard-Pane (`app/src/cockpit/pane.rs`)
- Adopt the same tokens (zone cards, provider/plan slots, meters, dots) so sidebar and pane are one
  language.
- **Selected-session detail:** branch/worktree title · `Provider · model · effort` · `Context %` ·
  `today $` · `last activity` · **reset timers** (moved here) · actions (open/attach/review).
- **Selected-account detail:** provider · plan · 5h + week meters (full) · reset timers · token
  totals · per-session breakdown for that account.

## 5. Data-model fixes (prerequisite for §4)

1. **Provider vs plan.** Split the conflated field: account exposes `provider ∈ {Claude, ChatGPT}`
   **and** `plan_tier` (real subscription tier). Fix the Codex path that returns
   `plan_tier = "chatgpt"` (`crates/.../codex.rs:86`) — that string is the provider, not a plan.
   Claude side already carries a real tier (`"Max 20x"`); normalize both to `provider + plan`.
2. **Session worktree/branch.** The conductor session must expose its **worktree/branch** (or
   working directory) as a first-class identity field, not only `cwd`. Where a git branch is
   resolvable, prefer branch; else the leaf dir name; else a session title. This is what makes
   parallel same-model sessions distinguishable (P4).

## 6. Coherence fixes folded in (P6/P7)

- **Emoji → icon font (P6):** `ssh_manager/panel.rs:2200` and `spawn_card.rs:752` plus any chrome
  `⚡`. Terminal *text* status lines (`event_loop.rs` "⚡ Zaplexify active") are content, out of
  scope. Audit: `rg -n '⚡|✧|📌' app/src` must show zero UI-chrome hits after.
- **i18n (P7):** every user-facing literal in `spawn_card.rs` (and any sibling that hardcodes)
  routed through `t!()`; add matching `app/i18n/de/warp.ftl` + `en` keys. Add a lightweight guard
  (grep/test) that flags bare English string literals in render paths.

## 7. Spawn-card empty/degraded + dismiss (P8 UX half)

- When `installed_agents` is empty, **do not render the full launch form**. Show only: title, one
  calm line `t!("spawn-no-cli")`, an install CTA, and Cancel. No Model/Effort/Account/Host/Directory
  rows, no dimmed Launch (today they still render — `spawn_card.rs:648`).
- Add **click-outside-to-dismiss** (scrim) in addition to the existing Escape (`spawn_card.rs:158`)
  and Cancel — the "how do I close it" gap.

## 8. Build increments (ordered; each build-verifiable)

- **S0 (separate track, ships now):** detection fix (§9) — pure correctness, no design gate.
- **S1 Tokens:** `style::zone_card`, provider icons, retire chrome `⚡`. Unit-renderable.
- **S2 Data model:** provider/plan split + session worktree field (+ tests). No UI yet.
- **S3 Hosts card:** `render_conductor` rewrite — tree rows, D1 session line, C2 fold, header `＋`.
- **S4 AI-Accounts card:** `render_header`+`render_card` rewrite — provider/plan, 5h+week meters,
  fleet total, `›` affordance.
- **S5 Dashboard-Pane:** same tokens, selected-session/account detail incl. reset timers.
- **S6 i18n sweep + spawn-card empty state + scrim dismiss.**
- **S7 Visual/boot verification** on devhost DMG (runtime acceptance — the step that keeps being
  skipped).

## 9. S0 — detection fix (pure bug, no design gate)

**Corrected root cause (2026-07-11, after user provided real paths).** The first hypothesis
(hardcoded PATH allowlist misses the install location) was **wrong**: the user's binaries are
`claude` at `/opt/homebrew/bin` (unconditionally in `extend_common_cli_dirs`, `cli_agent.rs:877`)
and `codex`/`claude` at `~/.nvm/versions/node/v22.18.0/bin` (covered by the nvm `read_dir`,
`cli_agent.rs:893`). Both are covered, so the scan *should* detect them. Shell is `fish` — but
detection is a **filesystem probe**, not a shell-PATH probe, so the shell is irrelevant.

The real path: install status lives in `CLIAgentInstallModel` (`cli_agent.rs:774`), a singleton
created at startup (`lib.rs:1498`) that runs `scan_cli_agent_installations` (`cli_agent.rs:826`,
uses `cli_agent_search_dirs` → the covering allowlist) **asynchronously**, caching into
`cache: Option<HashMap<CLIAgent,bool>>`. `is_cli_agent_installed` returns **`false` while
`cache == None`** (scan incomplete, `cli_agent.rs:804`). `open_spawn_card` **snapshots** the flag
once (`view.rs:17788`) and bakes it into the immutable `SpawnCardConfig`; the spawn card does
**not** subscribe to `CLIAgentInstallEvent::ScanComplete` (the workspace view does —
`view.rs:2816` — the card violates the documented pattern at `cli_agent.rs:772`). So if the card
reads the flag before the scan is ready, it says "No agent CLI installed" and never recovers for
that dialog's lifetime.

On `main @ 8a7f94eb` the scan is eager (startup) and covers the paths, so detection *should*
work by the time a user manually opens the card. Note the last two shipped DMGs
(`FIXED-2026-07-10` at branch tip `9c2b159b` and `8a7f94eb`) carry **identical code**, and the ⚡
marks live in `main` — so the emoji/`✧` observations are **consistent with being on a spine
build**, not evidence of an older one, and **reinstalling won't change detection**. The persistent
false is therefore most likely a **genuine race** in the packaged app: the spawn card reads the
async flag before the startup scan has populated `cache`, and — lacking a `ScanComplete`
subscription — never recovers. The fixes below make it correct regardless of scan timing.

Fixes (robust regardless of build):
1. While `!is_scan_complete()`, the spawn card shows a **"checking for CLIs…"** state — never a
   definitive "No agent CLI installed" during the scan window.
2. The spawn card **subscribes to `ScanComplete`** and re-reads `is_cli_agent_installed` / redraws
   (the pattern `cli_agent.rs:772` prescribes and `view.rs:2816` already uses).
3. Don't persist "uninstalled" to per-agent settings (`on_scan_complete` → `sync_per_agent_from_scan`)
   from a first, possibly-empty scan without at least one completed real scan.

**S0 step 0:** confirm which build is actually running (see the build-identity check) before
concluding there is a code bug on `main`.

## 10. Acceptance criteria

- Sidebar renders as two flat zone-cards; host rows and account rows share one row grammar (same
  leading column, typography, hover behavior). No filled "card" vs "bare row" split.
- Three parallel `opus` sessions in three worktrees of one project are individually identifiable by
  branch, on fixed-height single lines, with no layout shift on hover.
- Every account shows Provider · Plan (never a provider in the plan slot) and both 5h + week meters.
- `rg '⚡|✧|📌' app/src` → zero UI-chrome hits; UI is single-language for a given locale.
- Clicking a session/account selects it (stable highlight) and shows full detail in the pane; the
  sidebar list never reflows from hover.
- Spawn card with no CLI shows only the install prompt (no phantom form) and is dismissable by
  Escape, Cancel, **and** click-outside.
- Detection reports Claude+Codex present when they are on the user's login-shell PATH.

## 11. Open items

- Exact provider icon glyphs (need `Icon::` variants for Claude/ChatGPT — add SVGs if absent).
- Whether project grouping shows the repo name or the worktree-parent (default: repo name; worktrees
  listed as sessions under it).
- Confirm `ctx%` stays on the D1 session line (assumed yes per D1 choice).

## 12. supaterm-Learnings (2026-07-12)

Distilled from a comparison with **supaterm** (`supabitapp/supaterm`, https://supaterm.com — a
local, native-macOS agent terminal on libghostty) after a code audit of our current cockpit. These
five refine or reinforce the decisions above; they **add** to this spec and change nothing in §0–§11.
Only the learnings that shape *this* surface (a sidebar session/account row and its detail) are in
scope — see "Deliberately out of scope" at the end.

**L1 — One computed, contrast-guaranteed semantic palette (reinforces §3 + the "one premium visual
language" mandate).** supaterm just moved its whole chrome palette into one sRGB model (`SupaTheme`:
`ThemeColor` / `ColorMath` / `ReferencePalette` / `Palette`): semantic roles (`accent` / `warning` /
`success` / `danger` / `merged`) are **derived** from a few fixed reference anchors via OKLCH
lightness math, readable `on*` foreground tokens are computed from the result, and `ChromePaletteTests`
**asserts** the contrast per color-scheme. Our `style::heat_coloru` (`style.rs:57`, single source) is
the same idea applied to heat — the learning is to **generalize it**: the tokens §3 introduces
(`zone_card`, dot states, provider/plan slots, meters) should read their colors from *derived semantic
roles* against the actual surface they render on, with a contrast test in CI — not hand-picked hexes
per view. This is the systemic cure for the "two design languages stacked" root problem (§0/P1) and
the still-open premium-visual-language debt this spec inherits. *Build hook:* fold into **S1 Tokens**
(§8) as "semantic role tokens + contrast test", ahead of the per-component work.

**L2 — Attention co-located and loudest, at every zoom (reinforces §2.9 + §4.1 C2 fold).** supaterm's
signal is a **glow on the object that needs you**, so peripheral vision alone tells you *which* row is
blocked. Our design already bubbles the worst child-state into the collapsed dot (§2.9) and renders
every `waiting` session first (§4.1 C2). The learning sharpens the *treatment*: `waiting`/amber
(`attention_coloru`, `style.rs:71`) must be the **loudest, highest-contrast** state in the row grammar
(§3) and visible at each collapse level (session → project dot → host dot), so a blocked agent is
discoverable without expanding anything. Keep it **ambient and calm** — the signal is *slow and
low-contrast* (a soft steady tint, or at most a slow ~2–3 s breathe), **never a fast blink/pulse** and
never a per-event notification; the distinction that matters is **speed + contrast, not motion
yes/no** — a glow can move and still be calm. The pane/tab glow *surface* is a separate design point
(issue #120); here it's the sidebar row/dot treatment. *No new build step; it's an acceptance
sharpening of S3.*

**L3 — Peek-before-navigate, within the binding hover rule (refines §2.8).** supaterm lets you **hover
a row to see what the agent is up to** — a cheap glance before any navigation. Our §2.8 rule forbids
hover that *reveals data which shifts size/position and reflows following content* — and that rule
stays binding. The reconciliation: a peek is allowed **only as a fixed-size floating popover**
(absolutely positioned, does not reflow the list), never as inline row expansion. That gives three
tiers of disclosure — glance (hover popover) → select (click → stable highlight) → full detail
(Dashboard-Pane, §4.3) — without violating §2.8. Popover contents: current state word, the current
plan step (see L4), last activity line, account/host. *Build hook:* optional, after **S5**; tracked
build-side as issue #115.

**L4 — A structured plan is the primary "what's it doing" surface; the transcript is drill-down
(shapes §4.1 state + §4.3 detail).** supaterm reads the agent's task/todo list from the transcript
and shows it as an ordered, progressive checklist — far more legible than a scrolling conversation
feed. For this spec: the Dashboard-Pane selected-session detail (§4.3) should make the agent's
**structured plan/todo list** (ordered, with a progress count) the primary activity surface, with the
raw transcript reachable as a secondary drill-down — not a log as the first thing shown. The L3 peek
popover surfaces just the *current* step of that plan. *Build hook:* the extraction is tracked
build-side as issue #114; this spec fixes *where* it renders (pane detail primary; peek popover
current-step).

**G — Guardrail (adopt supaterm's leitsatz verbatim into intent).** "*Keep every coding agent visible
without losing the terminal.*" Agent status is **peripheral and ambient**; the terminal stays
**focal**. This is a binding lens for every decision here: the sidebar and Dashboard-Pane must never
compete with the terminal for the user's focus — they inform at a glance and yield on click.

**Deliberately out of scope for this spec** (tracked/decided elsewhere, would overload the sidebar
redesign): the **pane/tab glow** itself (touches the tab-strip and terminal pane, own design point —
issue #120);
**Spaces** as a grouping layer above tabs and **tab pinning** (structural, above the row — issue #117);
and **per-event desktop notifications** (supaterm does them; we deliberately chose the calmer
ambient-bit — dock badge + single chime + inbox — and keep it).
