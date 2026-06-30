# Cockpit — Increment 1 Design: Account & Usage Data Spine (Claude + Codex)

> First increment of the zaplex **cockpit** — the "plex" half of the product: the
> overview that answers *what needs my attention, what am I spending, how loaded is
> each subscription* and (later) routes/switches work across multiple Claude + Codex
> subscriptions. This increment builds the **read-only data foundation only**; the
> UI, the live session inventory, and launch/switching are later increments.
>
> Grounded in: the on-disk footprint of the real CLIs (verified, see §3 + Appendix),
> the `claudeplex` reference TUI (`~/projects/zaplex/claudeplex/src`, Claude-only —
> Codex is net-new for us), and the existing zaplex model/watcher patterns.

## 1. Goal & Non-goals

**Goal.** A native Rust data layer that, for **both** Claude Code and Codex CLIs:
1. discovers the logged-in accounts / subscriptions (possibly several per provider),
2. aggregates token usage from the CLIs' own session transcripts into rolling
   windows (5h block / today / week) with reset times,
3. derives **cost** (per-model pricing table) and **heat** (load vs. budget),
4. exposes the result as a `CockpitModel` singleton emitting change events, refreshed
   by **file-watch** (not polling).

This mirrors how the remote-session layer started: data/protocol first, UI later —
the proven increment style for this project.

**Non-goals (explicitly deferred to later increments):**
- Any **UI** (account cards, heat bars, cost) — Increment 2.
- Live **session/agent inventory** + "needs attention" state — Increment 3.
- **Launch-on-freest** routing + credential-swap launching + subscription switching — Increment 4.
- **Multi-host** usage aggregation (over the daemon) + **persisted history**/charts — Increment 5.
- Reading or storing OAuth tokens/secrets (we read only token *counts* + account *metadata*).

## 2. Why this first / mission fit

The remote-session layer (done) ensures agents survive disconnects; the **cockpit is
the actual value proposition** ([[mission-and-reference-sources]]) — drastically
reducing mental load across many parallel Claude/Codex sessions. Everything the
cockpit shows or decides (heat bars, cost, "launch on freest", switching) is a
function of one thing: **per-account usage over time**. So the first increment is
that spine. It is also independently testable headless (no GUI, no network), exactly
like the remote-session data layer.

Note: zaplex's inherited `AIRequestUsageModel` (`app/src/ai/request_usage_model.rs`)
was deliberately **gutted to an "unlimited" stub** (module doc lines 1-19) — the old
server-driven quota/cost concept was removed. The cockpit is its intended
replacement, not a competitor. Self-contained directive: both provider data layers
native from day 0, no Bun/`claudeplex` subprocess.

## 3. Verified data sources

### 3.1 Claude Code (matches `claudeplex` `discover.ts`/`collect.ts`)
- **Account discovery** = config *directories*, deduped from: a `$HOME` scan for
  `.claude` and `.claude-*` (excluding `*mem/backup/bak/old/tmp/temp/observer`);
  running processes carrying `CLAUDE_CONFIG_DIR=…` (so live-but-non-default accounts
  are found); and `$CLAUDE_CONFIG_DIR`. A dir qualifies if it has `.claude.json` or a
  `projects/` or `sessions/` subdir. (`claudeplex` `discover.ts:60-183`.)
- **Identity** from `<dir>/.claude.json` → `oauthAccount`: `emailAddress`,
  `displayName`, `organizationName`, `organizationRole`, `organizationType`,
  `organizationRateLimitTier`. **No tokens/expiry are read** — presence of
  `oauthAccount` defines "a real account". The default dir `~/.claude` reads its
  account from `~/.claude.json` in `$HOME`. (`collect.ts:461-487`.)
  - Plan label: `organizationRateLimitTier` matched `max_(\d+x)` → "Max 20x"; else
    `organizationType == "claude_max"` → "Max"; else strip `claude_`.
- **Usage** = parse `<dir>/projects/<proj>/<session>.jsonl` line by line; for each
  line with `type=="assistant"` and `message.usage`, read `input_tokens`,
  `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`,
  `message.model`, `timestamp`. (Verified on a real transcript — see Appendix A.)

### 3.2 Codex (net-new — no `claudeplex` prior art; verified on disk here)
- **Account** = `~/.codex/auth.json`: `auth_mode`, `last_refresh`,
  `tokens.{account_id,access_token,id_token,refresh_token}`. The email/plan are **not**
  plaintext at top level — they are likely in the `id_token` JWT claims. We will
  decode only the **unverified JWT payload** for `email`/plan-ish claims and **never
  read or store** the token strings. (Open question §10: confirm claim names; possible
  fallback to `~/.codex/*.sqlite`.)
- **Usage** = `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Line `type`s:
  `session_meta`, `turn_context`, `response_item`, `event_msg`. Token fields present:
  `input_tokens`, `output_tokens`, `cached_input_tokens`, `reasoning_output_tokens`,
  `total_tokens`, and `last_token_usage` / `total_token_usage` envelopes
  (`reasoning_output_tokens` is Codex-specific). (Verified — Appendix B.)
- **Multi-account**: Codex's multi-config story (a `CODEX_HOME`-style override?) is
  unconfirmed — §10.

### 3.3 Existing zaplex seams that already touch these paths
- `app/src/ai/mcp/mod.rs:49,93` — `MCPProvider::{Claude,Codex}` + `home_config_file_path()`
  → `~/.claude.json`, `~/.codex/config.toml`. Reuse the provider enum + path helpers.
- `app/src/ai/mcp/file_mcp_watcher.rs` — already **watches** `~/.claude.json` and the
  `~/.codex` subtree via `HomeDirectoryWatcher` and re-parses on change. This is the
  template for cockpit discovery (watch, don't poll).
- `app/src/terminal/cli_agent.rs:134` — `CLIAgent::{Claude,Codex,Gemini,…}` — the
  provider key to reuse.

## 4. Data model (in `crates/zaplex_cockpit`)

```
Provider          = CLIAgent::{Claude, Codex}            // reuse existing enum
Account { provider, key /* stable "c1","c2"… by path */, config_dir,
          label, email, org, role, plan_tier, is_default }
UsageEntry { ts, provider, model, input, output, cache_create, cache_read,
             reasoning /* Codex */ }
WindowTotals { input, output, cache_create, cache_read,
               work /* = input+output+cache_create */, total,
               cost_usd, messages }
AccountUsage { account, block5h, today, week,
               reset5h, reset_week, heat /* = work/budget, 0..1+ */ }
CockpitSnapshot { accounts: Vec<AccountUsage>, generated_at }
```
`work` (excludes cheap cache reads) is the load signal for heat + later
launch-on-freest, per `claudeplex` `usage.ts:57-66`.

## 5. Cost & heat — explicit approximations

- **Cost** = per-model pricing table, USD per 1M tokens, with distinct
  cache-write/cache-read rates, keyed by model-name substring:
  `cost = (in·p_in + out·p_out + cache_create·p_cw + cache_read·p_cr) / 1e6`.
  Seed from `claudeplex` `usage.ts:29-48` (opus/sonnet/haiku) **but refresh against
  current Anthropic + OpenAI pricing** (use the `claude-api` skill for Anthropic
  rates; Codex/OpenAI rates from their pricing page — `~/.codex/models_cache.json`
  has model metadata but **no price fields**, confirmed). **Flag:** rates drift; keep
  the table in one place, updatable, with an unknown-model fallback that is logged,
  not silently mispriced.
- **Heat** = `work / budget` over the window, clamped for display. Budget is **not**
  an API-exposed real cap (Anthropic/OpenAI don't publish per-plan token budgets);
  `claudeplex` uses a flat guess (`BUDGET_5H=20M`, `BUDGET_WEEK=300M`,
  `instances.ts:43-44`). We will (a) map `organizationRateLimitTier` → a per-tier
  budget estimate where possible, (b) fall back to the flat guess, and (c) make both
  overridable via settings/env. Document heat as an *estimate of headroom*, not a
  guarantee.
- **Reset times** = ccusage-style rolling block (first activity floored to the hour;
  a gap ≥ window starts a new block), computed for 5h and 7d (`usage.ts:108-120`).

## 6. Architecture & integration

- **New crate `crates/zaplex_cockpit`** (own-provenance naming, [[crate-naming-by-provenance]]).
  Pure data layer; deps: `warpui` (Entity/SingletonEntity/ModelContext/spawner),
  `ai` (CLIAgent), `warp_ssh_manager`? no — `watcher` (file-watch), `serde`/`serde_json`.
  UI stays out (later in `app/src/cockpit/`). Wire like `zaplex_remote_session`:
  root `Cargo.toml [workspace.dependencies]`, `app/Cargo.toml` dep gated on the
  existing `local_fs` feature (disk access).
- **`CockpitModel`** — `SingletonEntity` (templates: `crates/ai/src/api_keys.rs:215`
  storage-backed singleton; `crates/remote_server/src/manager.rs:478-496` for the
  `spawner: ctx.spawner()` field). Holds the latest `CockpitSnapshot`; emits
  `CockpitEvent::Updated`. Registered in `app/src/lib.rs` (~line 1601, after
  `ApiKeyManager`) via `ctx.add_singleton_model(|ctx| CockpitModel::new(ctx))`.
- **Discovery = file-watch + reconcile tick:**
  1. Primary: wrap `BulkFilesystemWatcher` exactly like
     `crates/watcher/src/home_watcher.rs` / `file_mcp_watcher.rs`; register
     `~/.claude*` and `~/.codex*`. On a debounced change event, re-discover + re-parse
     **off the main thread** via `ctx.spawn(async { read+parse }, |me, snap, ctx| { me.apply(snap); ctx.emit(Updated) })`
     (the `file_mcp_watcher.rs:531` `spawn_config_parse` pattern; `async_fs::read_to_string`).
  2. Secondary: a low-frequency reconciliation tick (the `start_gc_timer` loop,
     `app/src/remote_server/server_model.rs:2257-2285`: `background_executor().spawn`
     + `spawner.spawn(|me,ctx| …).await`, break on `Err` = model dropped) — recomputes
     window membership / reset times / heat decay even when no file changed (e.g. the
     5h block rolls over). ~30–60s is plenty.
- **Parsing performance** (mirror `claudeplex` `collect.ts:159-162,626`): cache parsed
  transcripts by `(mtime, size)`; only parse files whose mtime is within the widest
  window (week). Transcripts can be large and numerous.

## 7. Settings & persistence

- **`define_settings_group!(CockpitSettings, …)`** (template `app/src/settings/ssh.rs`;
  register in `app/src/settings/init.rs:95`) for **scalar** policy only:
  `cockpit.enabled` (bool), `cockpit.budget_5h` / `cockpit.budget_week` overrides (int,
  0 = use tier estimate), and `cockpit.switching_policy` (enum, consumed later).
  Scalar settings can't hold a list of accounts — that's fine; accounts are
  *discovered*, not configured.
- **Per-account pins** (custom label/color/order/hide, like `claudeplex`
  `instances.json`): serialized JSON in secure storage (the `ApiKeyManager` pattern) —
  **Increment 2+**, not now.
- **Persistence of history: deferred.** Increment 1 computes everything live from
  transcripts (cached); no DB. If long-term trend charts are wanted later, add a
  `cockpit_usage` table via the `ring_ceiling` migration pattern
  (`crates/persistence/migrations/…` + `model.rs` + a repository in `zaplex_cockpit`).

## 8. Increment plan

- **Increment 1 (this doc) — data spine.** Crate + `CockpitModel` + Claude & Codex
  account discovery + usage/cost/heat aggregation + watch+reconcile + `CockpitEvent`
  + `CockpitSettings` (scalar) + headless unit tests. **No UI** beyond an optional
  hidden debug command to dump the snapshot.
- **Increment 2 — UI.** Account cards (label/plan), heat bars, today/5h/week cost +
  tokens, reset timers. Design-latte (the claudeplex-desktop polish: status glyphs,
  load bars). Subscribes to `CockpitEvent::Updated`.
- **Increment 3 — live inventory.** Claude `<dir>/sessions/*.json` registry + pid
  liveness + transcript-derived `working/needs-attention/idle`; the Codex equivalent;
  "what needs my attention" surface.
- **Increment 4 — launch-on-freest + switching.** Pick lowest-`work` non-busy
  account; launch the real CLI with the chosen account (env/config-dir swap; never an
  API key). Multi-subscription balancing to dodge rate limits.
- **Increment 5 — multi-host + history.** Aggregate per-host snapshots over the
  daemon; optional persisted history/trends.

## 9. Test strategy (headless, no secrets)

- **Fixtures, not real creds.** Build temp config dirs with synthetic `.claude.json`
  (a fake `oauthAccount`) and fixture transcripts (Claude `.jsonl`, Codex
  `rollout-*.jsonl`); assert: account discovery (count, label, plan tier), `UsageEntry`
  extraction per provider (incl. Codex `reasoning_output_tokens`), window bucketing
  with a **fixed `now`** (5h/today/week boundaries), cost against golden numbers, heat
  = work/budget. Pin one known model's cost so a pricing-table edit is a conscious
  test change.
- **Secrets:** the parser must read only token counts + account metadata; a test
  asserts no token/secret field is ever surfaced. Never require a real `~/.claude`/`~/.codex`.
- Runs locally (`cargo test -p zaplex_cockpit`); no GUI/network. **Run the full
  affected-crate suite, not just `-p warp`** (lesson from the remote-session work —
  a sibling crate's tests silently broke when only the app crate was run).

## 10. Risks & open questions

1. **Codex account/plan discovery (net-new).** Confirm where email/plan live —
   `id_token` JWT claims vs. a `~/.codex/*.sqlite` table. Decode only the unverified
   JWT payload; never store tokens. Small spike at the start of Increment 1.
2. **Codex usage semantics.** `total_token_usage` may be cumulative-per-session vs.
   `last_token_usage` per-turn — pick one consistently to avoid double-counting.
   Prefer summing per-turn deltas (parity with the Claude per-message approach).
3. **Codex multi-account.** Is there a `CODEX_HOME`-style override enabling several
   logins, or is it single-account? Determines whether discovery scans dirs (Claude)
   or is a single `~/.codex` (Codex) in v1.
4. **Pricing & budget are approximations** (both flagged). Keep the pricing table
   centralized + refresh on model launches; map tier→budget where possible, else the
   flat guess, both overridable.
5. **Privacy.** Read only token counts + account metadata — **never transcript
   content**. Document this prominently; it's a trust point for the product.
6. **Performance/footprint.** Many large transcripts → `(mtime,size)` cache + week
   cutoff; the watcher must debounce (transcripts are appended frequently during an
   active session).

## Appendix A — Claude transcript usage block (verified)

```json
{"type":"assistant","model":"claude-opus-4-8",
 "usage":{"input_tokens":19370,"cache_creation_input_tokens":9716,
  "cache_read_input_tokens":19748,"output_tokens":230,
  "cache_creation":{"ephemeral_1h_input_tokens":9716,"ephemeral_5m_input_tokens":0},
  "service_tier":"standard"}}
```

## Appendix B — Codex session (verified)

`~/.codex/sessions/2026/06/30/rollout-*.jsonl`; line `type`s: `session_meta`,
`turn_context`, `response_item`, `event_msg`. Token fields seen: `input_tokens`,
`output_tokens`, `cached_input_tokens`, `reasoning_output_tokens`, `total_tokens`,
`last_token_usage`, `total_token_usage`. `~/.codex/auth.json` keys: `auth_mode`,
`last_refresh`, `tokens.{access_token,account_id,id_token,refresh_token}` (values
never read/stored).

## Appendix C — Code seams (verified, file:line)

- Singleton + storage: `crates/ai/src/api_keys.rs:215` (ApiKeyManager); gutted
  replacement target: `app/src/ai/request_usage_model.rs` (stub).
- Spawner round-trip / timer: `crates/remote_server/src/manager.rs:541-613`,
  `app/src/remote_server/server_model.rs:2257-2285`.
- File-watch: `crates/watcher/src/{lib.rs:144,195,229,home_watcher.rs}`;
  `app/src/ai/mcp/file_mcp_watcher.rs:531` (`spawn_config_parse`, `async_fs::read_to_string`).
- Provider enums + paths: `app/src/terminal/cli_agent.rs:134`,
  `app/src/ai/mcp/mod.rs:49,93`.
- Settings: `app/src/settings/ssh.rs:5-28`, `app/src/settings/init.rs:95`,
  `crates/settings/src/macros.rs:703`.
- Persistence pattern (if history later): `crates/persistence/migrations/2026-06-29-000000_add_ring_ceiling/`,
  `crates/persistence/src/model.rs:1463-1497`, `crates/warp_ssh_manager/src/repository.rs`.
- Reference (Claude-only): `~/projects/zaplex/claudeplex/src/{discover,usage,collect,instances}.ts`.
