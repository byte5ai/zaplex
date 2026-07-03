# Cockpit C3b — Real OAuth Usage ("the desktop jewel")

> **Status:** Implemented (milestones 1–3) — 2026-07-03. Runtime acceptance (real credentials, network-kill fallback) pending the next user test build.
>
> **Quelle der Entscheidung:** User 2026-07-03 (adopt the OAuth-usage capability identified in the zero analysis); gap analysis `2026-07-03-claudeplex-gap-analysis.md` line 17 marks this ❌ and calls it the "Desktop-Juwel — höchster Genauigkeitsgewinn"; scheduled there as increment C3b.
>
> **Direktive:** Deliberate, scoped departure from the C1 non-goal "no reading of OAuth tokens/secrets" (`2026-06-30-cockpit-increment1-account-usage-design.md` line 31). The departure is exactly one read + one endpoint — nothing else about the privacy posture changes.
>
> **Goal:** Cockpit heat bars show the **real** Anthropic subscription utilization (5h + week, with reset times) instead of a token-count estimate against a guessed budget — per Claude account, with honest fallback to the estimate.

---

## 1. Why

Today `zaplex_cockpit` derives heat purely from transcript token counts vs. `windows.rs::plan_budgets()` (a flat plan-tier guess) or user-set soft budgets. That is an *estimate of spend*, not the *actual rate-limit position* — it drifts from reality (cache tokens, server-side accounting, plan changes) and cannot see resets authoritatively. claudeplex-desktop solved this by asking Anthropic directly; users of the reference app consider it the single most valuable accuracy feature. The whole cockpit promise ("heat status in peripheral vision, without looking") is only as good as this number.

## 2. Mechanism (claudeplex-desktop as the template, built natively)

Per discovered Claude account (each `CLAUDE_CONFIG_DIR`):

1. Read the OAuth **access token** from `<config_dir>/.credentials.json` (written and refreshed by Claude Code itself; we never write it, never trigger refresh).
2. `GET https://api.anthropic.com/api/oauth/usage` with that bearer token.
3. Parse per-window utilization + reset timestamps (**verify exact response schema at build time** against the claudeplex-desktop reference implementation, `~/projects/zaplex/claudeplex-desktop`).
4. Cache **15 minutes per account** (in-memory, in `CockpitModel`); refresh piggybacks on the existing off-thread reconcile cadence — no new timers.
5. On any failure (no credentials file, expired token, 401/403, network down, schema drift): **fall back to the existing estimate** and mark the value's provenance.

## 3. Design decisions

- **Crate purity preserved:** `zaplex_cockpit` stays network-free. It gains only the *types* (`OauthUsage`, `UsageProvenance {Real, Estimate}`) and the merge logic (`apply_oauth_usage(snapshot, per_account_usage)`). The HTTP call lives in the app wiring (`app/src/cockpit/model.rs`, off-thread, next to the existing refresh) using the app's existing HTTP stack. This keeps the crate fixture-testable and the network dependency at the edge.
- **Provenance is visible but calm:** heat bars driven by real data get no extra chrome; estimate-driven bars get a subtle marker (e.g. `~` prefix on the percentage, hover explains). Honest degradation (§2.3), no noise (§5.7).
- **Token hygiene (hard rules):** token is read, used for this one request, and dropped — never logged, never persisted, never shown in UI or errors, never used for any other endpoint. Errors are reported as status categories ("credentials unavailable", "endpoint rejected"), never with payloads.
- **Setting:** `cockpit.oauth_usage: bool`, default **on** (it is a read-only call with the user's own credentials on their own machine, and it is the accuracy jewel). Turning it off returns fully to estimates. No per-account granularity (not a real need yet).
- **Codex side:** out of scope. Codex rate-limit data source is still unverified (concept §10.2); when it is located, an analogous `Real` provenance can be added behind the same `UsageProvenance` type. Until then the Codex column stays estimate-based — honest asymmetry, stated in UI hover, no fake parity (§11.9).

## 4. Policy boundary (why this is safe, and where the line is)

Anthropic prohibits and since 2026-04 technically blocks using subscription OAuth tokens for **model/completions calls in third-party tools** (usefully documented in `Gitlawb/zero` `docs/oauth-subscriptions.md`). zaplex never does that — the agents themselves own their auth and their API traffic. C3b only reads the **usage/quota endpoint** for the user's own account for local display, exactly what claudeplex-desktop has done. The line, stated in code comments and docs: *the cockpit may ask "how full is my quota", it may never spend the quota.* If Anthropic ever gates the usage endpoint, the fallback path already handles it (degrade to estimate + one calm notice, no breakage).

## 5. Milestones & acceptance

| Step | Scope | Abnahme |
|---|---|---|
| 1 | schema verification against claudeplex-desktop reference + recorded fixture | fixture checked into `zaplex_cockpit` tests |
| 2 | types + merge logic in `zaplex_cockpit`, fetcher in `app/src/cockpit/model.rs`, 15-min cache | with valid credentials: heat = real utilization incl. reset times; kill network → estimate + `~` marker within one refresh cycle |
| 3 | provenance UI in `panel.rs`/`pane.rs` + `cockpit.oauth_usage` setting | toggle off → estimates everywhere, no fetches occur (verify via logging in dev build only) |

## 6. Code seams (verified 2026-07-03, worktree `feat/stage2-client-attach`)

- `crates/zaplex_cockpit/src/windows.rs` — `build_account_usage()`, `plan_budgets()` (estimate path to fall back on)
- `crates/zaplex_cockpit/src/claude.rs` — `discover_accounts()` (source of per-account `config_dir`)
- `crates/zaplex_cockpit/src/types.rs` — `AccountUsage` (gains provenance + optional real windows)
- `app/src/cockpit/model.rs` — off-thread refresh + 45s reconcile tick (fetch + cache lives here)
- `app/src/cockpit/{panel.rs,pane.rs}` — heat bar rendering (provenance marker)
- `app/src/cockpit/settings.rs` — `CockpitSettings` (new flag)
- Reference: `~/projects/zaplex/claudeplex-desktop` (endpoint + response schema + 15-min cache pattern)
