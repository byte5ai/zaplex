# Zero Agent Integration — zaplex as the natural terminal for Gitlawb/zero

> **Status:** Design (read-only plan, no code yet) — 2026-07-03
>
> **Quelle der Entscheidung:** User 2026-07-03 — adopt zero's stream-JSON protocol so zaplex can address the zero community as its natural terminal partner ("zero + zaplex = perfect match"); User 2026-07-03 (same day, follow-up): multi-provider support beyond Claude/Codex was never excluded — Claude and Codex were must-haves and a complexity guard for the start; further agent integrations are explicitly sensible once the base is solid. See §6 for how this reconciles with `zaplex-concept.md` §11.7.
>
> **Direktive:** No quick wins — build the integration on zaplex's existing agent machinery (CLIAgent registry, OSC 777 listener, cockpit spine), not as a parallel bolt-on. zaplex stays a *terminal/cockpit* for agents; it never becomes an agent itself and never competes with zero.
>
> **Goal:** A zero user opens zaplex and gets first-class treatment: zero blocks with live status, needs-input detection, native rendering of headless zero runs via stream-JSON — and zaplex becomes the reference interactive surface for zero's protocol.

---

## 1. Context

[Gitlawb/zero](https://github.com/Gitlawb/zero) (Go, MIT, ~474 ★, created 2026-05, very active) is "a terminal coding agent you own": 25+ providers, local sessions, permission/sandbox policy, TUI + headless `zero exec`. It is **not** a competitor to zaplex — zero is an *agent*, zaplex is the *terminal cockpit that hosts agents*. The strategic play: zero's community consists of exactly zaplex's target audience (terminal-native, ownership-minded devs running agents), and zero has **no terminal partner** and no GUI story. zaplex supporting zero natively is a visibility multiplier for both projects.

Three assets in zero's repo matter to us:

1. **`docs/STREAM_JSON_PROTOCOL.md`** — a schema-versioned (v2) JSONL protocol for headless runs: `run_start → text/tool_call/tool_result/permission_request/permission_decision/usage → final → run_end`, with structured risk metadata (`sideEffect`, `risk.level`, `block`). Cleaner than Claude Code's stream-json (explicit versioning, permissions as first-class events).
2. **Session semantics** — `zero exec --resume`, `--fork <session-id>`, `--worktree`; sessions stored on disk, searchable. (Fork/worktree UX adaptation is a separate design: `2026-07-03-agent-session-fork-and-worktree-launch-design.md`.)
3. **Hooks/plugins** — `zero hooks` (lifecycle hooks) and `zero plugins`, the anchor for a zaplex-side event plugin analogous to our existing Claude/Codex/Gemini plugins.

**Known protocol gap (our upstream opportunity):** zero's stream-JSON *input* accepts only `message`/`prompt` events. Headless `exec` has **no interactive permission responder** — a prompt-gated tool emits `permission_request` followed by a denied `tool_result`. An interactive surface that answers permission requests over stdin does not exist yet. That is precisely what a terminal partner would contribute (§3, Z3).

## 2. Goals & non-goals

**Goals**

- G1 — zero is a first-class *block agent* in zaplex: detection, banner, footer, needs-input status, notification center (parity with Claude/Codex/Gemini blocks).
- G2 — zaplex can natively render a headless `zero exec --output-format stream-json` run: typed timeline of tool calls, permission events, usage, result.
- G3 — the stream-JSON consumer is built as a reusable protocol layer (own crate), so a later Claude Code stream-json adapter reuses the same internal event model instead of a second pipeline.
- G4 — upstream engagement: propose the interactive permission-responder input event to Gitlawb/zero and get zaplex listed as a compatible client (community visibility).

**Non-goals**

- No zero entry in the cockpit *orchestration* layer (accounts, heat, "launch on freest"): zero is BYOK/local and has **no subscription rate windows** — heat semantics do not apply. Honest degradation per concept §2.3/§3.4, not fake parity.
- No zero session *inventory* in the cockpit spine (C3a) in this iteration. Zero stores sessions on disk and this may come later — it is real, not speculative, but out of scope until Z1/Z2 have landed.
- zaplex does not embed, fork, or wrap zero itself. No bundled zero binary.

## 3. Architecture — three stages

### Z1 — Block-level integration (small, pure parity work)

Uses only existing machinery; this is the same shape as every other supported CLI agent:

1. `CLIAgent::Zero` variant in `app/src/terminal/cli_agent.rs` (registry at line ~134): `command_prefix() = "zero"`, display name, icon (Zap icon set; fallback generic agent glyph until an icon exists).
2. Listener support in `app/src/terminal/cli_agent_sessions/listener/mod.rs` (`DefaultSessionListener` + `is_agent_supported`).
3. `plugin_manager/zero.rs` — install flow for a zaplex hook plugin on the zero side (`zero hooks`), emitting the OSC 777 `warp://cli-agent` events (`SessionStart`, `PromptSubmit`, `ToolComplete`, `Stop`, `PermissionRequest`, `QuestionAsked`) exactly like the Claude plugin does. **Verify at build time:** zero's hook API surface (event set, payload, whether hooks can write to the controlling TTY).
4. Zaplexify remote detection (`app/src/terminal/ssh/zaplexify.rs`): recognize `zero` on hosts like `claude`/`codex`.

Result: zero TUI sessions in zaplex blocks get banner/footer/blocked-status/notification parity. A zero user's first-run experience already beats every other terminal.

### Z2 — Native stream-JSON rendering (the protocol layer)

New pure crate **`zaplex_agent_protocol`** (naming per concept §4.2 discipline; fixture-tested like `zaplex_cockpit`):

- Typed serde model of zero stream-JSON **schemaVersion 2**: input events (`message`, `prompt`) and output events (`run_start`, `text`, `tool_call`, `permission_request`, `permission_decision`, `tool_result`, `usage`, `error`, `final`, `run_end`) including `risk`/`block` metadata. Unknown-field strictness mirrors zero's own contract (they reject unknown fields → we parse strictly and surface schema drift as a typed error, never a silent skip).
- A small **internal event model** (`AgentRunEvent`) that the UI consumes. Zero's adapter is the first producer; a later Claude Code `stream-json` adapter is the second. Model exactly what both real cases need — no speculative third-protocol abstraction (concept §11.7 spirit).
- Line-framed reader over a child process's stdout (`zero exec --output-format stream-json`), tolerant of interleaved stderr.

**First consumer surface:** headless agent runs launched *by zaplex* — the "Fix with your agent" flow and future PR-review/quick-issue flows (gap-analysis area (e)) — rendered as a native run timeline in the block instead of raw text scrollback: tool calls with side-effect badges, permission requests with risk level, usage line, final result. Interactive TUI sessions (Z1) are **not** re-routed through stream-JSON; they stay real PTYs (concept: agent CLIs remain fully usable directly).

### Z3 — Upstream engagement (the community play)

Ordered smallest-ask-first; **every outward action requires explicit user confirmation at the moment of action** (iron rule — external repo interactions are outward-facing):

1. **Issue/RFC in Gitlawb/zero:** propose a `permission_decision` **input** event (schemaVersion 3 or a capability flag) so an interactive surface can approve/deny prompt-gated tools in `exec` mode. zaplex is the motivating reference client; the protocol doc already models the output side, so the ask is small and natural.
2. **Upstream the zero-side hook plugin** (from Z1) as a zero plugin, so zero users get zaplex integration from zero's own ecosystem.
3. **Cross-listing:** README mention on our side (see `2026-07-03-public-readme-design.md`), and ask for a "works great with" mention in zero's docs once Z1/Z2 are real and demoable.
4. If (1) is accepted and shipped: implement the native permission modal for headless zero runs — zaplex renders `permission_request` with risk metadata, answers over stdin. This is the "perfect match" end state.

## 4. Protocol mapping (zero → zaplex)

| zero stream-JSON (v2) | zaplex internal (`AgentRunEvent`) | UI effect |
|---|---|---|
| `run_start {runId, sessionId, provider, model, cwd}` | `RunStarted` | run header in block |
| `text {delta}` | `AssistantText` | streamed markdown |
| `tool_call {name, args, sideEffect}` | `ToolCall` | timeline row + side-effect badge |
| `tool_result {status, truncated}` | `ToolResult` | row completion state |
| `permission_request {action, risk, block, reason}` | `PermissionAsked` | native risk banner; Z3(4): modal |
| `permission_decision {action, decisionReason}` | `PermissionDecided` | banner resolution |
| `usage {promptTokens, completionTokens}` | `Usage` | usage line (display only — **not** fed into cockpit heat; zero has no subscription windows) |
| `error {code, recoverable}` | `RunError` | error block |
| `final {text}` / `run_end {status, exitCode}` | `RunFinished` | result block + exit status |

`schemaVersion != 2` → explicit "protocol version unsupported" notice (honest degradation), never a best-effort parse.

## 5. Milestones & acceptance

| Stage | Scope | Abnahme |
|---|---|---|
| **Z1** | registry + listener + zero hook plugin + zaplexify detection | a `zero` TUI session in a zaplex block shows banner/footer/status; permission prompt in zero → block flagged Blocked; notification fires |
| **Z2a** | `zaplex_agent_protocol` crate, fixture round-trip tests for all v2 events | `cargo test -p zaplex_agent_protocol` green on recorded real `zero exec` fixtures |
| **Z2b** | run-timeline rendering for zaplex-launched headless zero runs | "Fix with your agent" via zero renders typed timeline incl. tool calls + usage |
| **Z3** | upstream RFC + plugin PR + cross-listing (each gated on user confirm) | RFC filed; zaplex named in zero ecosystem docs |

## 6. Concept amendment (required, small)

`zaplex-concept.md` §11.7 ("Provider-Enum ja — Spekulations-Enum nein") stays intact in spirit: we still model **only real cases**. What changes: zero is now a *real* case (User decision 2026-07-03), and the user has clarified that Claude/Codex-only was a launch-complexity guard, not a ceiling. Amendment wording (one paragraph in §11.7 + a note in §3.4): the **orchestration** Provider enum remains `{Claude, Codex}` (subscription-window semantics); **block-level agent support** is open by design (the `CLIAgent` registry already models 13 agents) and zero joins it as a first-class entry with a protocol-level rendering layer. A third *orchestration* provider is added when a real one with rate-window semantics exists — unchanged rule.

## 7. Risks & open questions

- **zero hook API fit (Z1.3):** verify zero's hook events and whether a hook can emit OSC to the TTY. If not: fall back to a wrapper-script plugin (like the Codex OSC 9 path) — visible degradation, still functional.
- **Protocol churn:** zero is young; schemaVersion may bump. Strict typed parsing + fixtures makes drift loud and cheap to fix. Pin fixtures to zero release versions.
- **Upstream reception:** the RFC may be declined or stall. Z1/Z2 deliver full value without it; Z3(4) is the only dependent piece.
- **License:** zero is MIT — no constraint on reading the protocol spec or shipping an adapter. Our plugin contributed upstream would carry zero's license terms; fine.
- **Effort guard:** Z1 is small (mirrors existing per-agent code); Z2a/Z2b is the real work (new crate + one rendering surface). No daemon or cockpit-spine changes required anywhere in this design.

## 8. Code seams (verified 2026-07-03, worktree `feat/stage2-client-attach`)

- `app/src/terminal/cli_agent.rs:134` — `enum CLIAgent` registry (add `Zero`)
- `app/src/terminal/cli_agent_sessions/listener/mod.rs` — per-agent listener handlers
- `app/src/terminal/cli_agent_sessions/event/mod.rs` — `CLIAgentEvent` / OSC 777 payloads (reused as-is)
- `app/src/terminal/cli_agent_sessions/plugin_manager/{claude,codex,gemini}.rs` — plugin install patterns (template for `zero.rs`)
- `app/src/terminal/ssh/zaplexify.rs` — remote agent detection
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` — "Fix with your agent" (first Z2b consumer)
- `crates/zaplex_cockpit/` — crate layout/test pattern to mirror for `zaplex_agent_protocol`
- Reference: `Gitlawb/zero` `docs/STREAM_JSON_PROTOCOL.md` (schemaVersion 2), `docs/oauth-subscriptions.md`, README §exec
