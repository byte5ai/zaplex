<div align="center">

<pre>
███████╗ █████╗ ██████╗ ██╗     ███████╗██╗  ██╗
╚══███╔╝██╔══██╗██╔══██╗██║     ██╔════╝╚██╗██╔╝
  ███╔╝ ███████║██████╔╝██║     █████╗   ╚███╔╝ 
 ███╔╝  ██╔══██║██╔═══╝ ██║     ██╔══╝   ██╔██╗ 
███████╗██║  ██║██║     ███████╗███████╗██╔╝ ██╗
╚══════╝╚═╝  ╚═╝╚═╝     ╚══════╝╚══════╝╚═╝  ╚═╝
</pre>

**The terminal cockpit for AI coding agents on remote machines.**

Close the laptop. Lose the network. Your agents keep working — history intact when you return.

[About](#about) · [How it works](#how-it-works) · [Features](#features) · [Agents](#agents) · [Status](#status--roadmap) · [Install](#install) · [Docs](docs/zaplex-concept.md)

[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE-AGPL) ![Platform: macOS](https://img.shields.io/badge/platform-macOS-1f1f1f)

<!-- HERO SLOT: ~10s GIF — cockpit visible, agent running → lid closes → reopen → session alive with replayed history -->

</div>

## About

Running coding agents on remote machines forces a choice between three things you want at once: a **real terminal** (ssh and your emulator), **sessions that survive disconnects** (tmux or mosh bolted around it), and **an overview of what your agents need** (nothing does this). zaplex refuses that trade-off — one native macOS terminal that owns all three.

zaplex is a fork of [Zap](https://github.com/zerx-lab/zap) — the open-source, local-first fork of [Warp](https://github.com/warpdotdev/warp)'s terminal — rebuilt into a cockpit for developers who drive **Claude Code** and **Codex** on one or many remote hosts, often with many sessions in parallel. It makes three promises:

1. **No tool switching.** Sessions, files, accounts, and costs in one place — the agent CLIs stay fully usable underneath; zaplex adds the overview on top.
2. **Nothing breaks.** A dropped connection, an empty battery, or a closed lid never interrupts an agent: sessions live in a native daemon on the host, not in your SSH connection.
3. **Less mental load.** What needs you *right now* is one glance and one hotkey away — across hosts, accounts, and providers.

## How it works

```text
  macOS — zaplex client                 your remote host — zaplex daemon
┌──────────────────────┐              ┌──────────────────────────────┐
│ GPU terminal, blocks │     ssh      │ ├─ session 62af · PTY + ring │
│ cockpit, file panes  │◄────────────►│ ├─ session b3e1 · PTY + ring │
└──────────────────────┘              │ └─ agents keep running       │
    close the lid ─╳                  └──────────────────────────────┘
    reopen ─────────► re-attach + byte-exact replay
```

1. **A session daemon on each host.** zaplex installs and maintains its own headless session host on your remote machines (automatic install ladder, offline-capable via bundled binaries). Sessions are PTYs owned by the daemon under persistent IDs, with a replay ring buffer: re-attach and your scrollback is replayed to the exact byte. Lifecycle is governed — idle sessions are garbage-collected under a host-wide RAM ceiling.
2. **Real terminals, real PTYs.** The client is a GPU-rendered, block-based terminal (Warp's proven core). Agents run as ordinary interactive CLIs in real PTYs — zaplex never wraps, proxies, or replaces them.
3. **A cockpit that reads, no telemetry.** Account discovery, usage heat, and session states come from read-only parsing of local data (config dirs, JSONL transcripts, session registries) on your machines — no zaplex cloud, no zaplex account. One disclosed exception: for Claude accounts, zaplex by default also asks Anthropic's own read-only OAuth usage endpoint for your exact quota utilization (capped at one request per account per 15 minutes); turn `cockpit.oauth_usage` off for local-estimate-only heat with zero outbound requests.

**What runs on your hosts:** one binary under `~/.zaplex/remote-server/`, spoken to exclusively over your existing SSH connection. It keeps session scrollback in bounded RAM, writes no telemetry, and retires itself when idle with no live sessions. Delete the directory and it is gone.

## Features

- **Persistent remote sessions** — agents survive lid-close, network roaming, and app restarts; re-attach replays history seamlessly. No tmux, no byobu, no mosh setup on the host.
- **Agent cockpit (Conductor)** — every subscription account with rolling 5h/week utilization heat and reset timers, backed by real numbers from Anthropic's own OAuth usage endpoint (on by default, read-only, opt-out), plus a cross-host **Host ▸ Project ▸ Session** tree; sessions **waiting on you** bubble up as `✋ N waiting`.
- **Guardrails & review loop** — pause, stop, or kill any agent (or all of them); review a session's diff and approve, redirect, commit, or open a PR in one flow.
- **Native agent awareness** — blocks know when an agent needs input, finished, or got blocked: banner, footer, notification center.
- **Multi-account, multi-provider** — all your Claude and ChatGPT/Codex subscription logins discovered and monitored side by side; launching a new agent can route to the freest account, including on a remote host, straight from the new-session menu.
- **Adopt any session** — daemon sessions started elsewhere appear in the sidebar; Enter attaches one as a block, history included.
- **Session fork & worktree launch** — fork a running session (same history, divergent future) or fork straight into an isolated git worktree to try another approach without disturbing the original.
- **File manager pane mode** — flip any terminal pane into a host-aware local or remote file manager; MC-style keyboard control, dual-pane cross-host copy/move (local↔local, local↔remote, remote↔remote via relay), streamed conflict handling, progress and cancellation, plus view/edit over SSH.
- **GitHub flows from the launcher** — draft a quick issue, review a PR, or triage an issue on the freest account; the agent drafts the exact `gh` command, you confirm before anything runs.
- **Session transcript viewer** — parsed, Markdown-rendered `.jsonl` transcripts, plus a live `◇ log` tail of the running session.
- **Tailscale host discovery** — Tailscale peers show up as ready-to-add SSH hosts.
- **Localized** — user-facing chrome is translated into German; falls back to English per key where translation is still incremental.
- **A full terminal first** — blocks, command palette, SSH host manager, themes: everything the Warp core does, without its cloud.

## Agents

**First-class: [Claude Code](https://github.com/anthropics/claude-code) and [Codex](https://github.com/openai/codex).** zaplex's orchestration layer — multi-account discovery, usage heat, account routing — is built around subscription accounts, because their rolling rate windows are what make heat tracking and "launch on the freest account" meaningful.

**Bring the rest.** zaplex is not an agent and does not ship one. Block-level support (status, banners, notifications) covers Antigravity, OpenCode, Copilot, DeepSeek/CodeWhale, Goose, and more — inherited from Zap and extended. More deep integrations will follow as the base solidifies.

**zero, natively.** zaplex is adopting [zero](https://github.com/Gitlawb/zero)'s schema-versioned stream-JSON protocol to render headless zero runs as typed, native timelines — tool calls, permission requests with risk levels, usage. zero stays your agent; zaplex becomes its cockpit. (Designed — see status below.)

## Status & roadmap

Zaplex 1.0.0 is in final validation. No public binary has been released yet;
the table reflects the integrated source state, not an unperformed runtime
acceptance:

| Area | State |
|---|---|
| Native session daemon: persistence, attach/replay, multi-session, GC | ✅ merged ([PR #16](https://github.com/byte5ai/zaplex/pull/16)) |
| Cockpit: account discovery, usage/heat/cost, live session states | ✅ merged |
| Cross-host Conductor: Host▸Project▸Session tree, guardrails, review loop | ✅ merged |
| File manager: host-aware pane mode, cross-host dual-pane copy/move, view/edit over SSH | ✅ merged |
| Fix/ask with *your* agent (routes to your own CLI agent) | ✅ merged |
| Real subscription utilization (OAuth usage endpoint) | ✅ merged |
| Subscription-routed launch: launch-on-freest account, incl. remote hosts (new-session menu) | ✅ merged |
| Guided launch wizard (modal) | 📋 planned |
| Session fork & isolated-worktree launches | ✅ merged |
| GitHub flows (quick issue, PR review, triage) — human confirms before anything runs | ✅ merged |
| zero integration: block support + stream-JSON rendering | 📋 designed |
| Safe dual-pane and cross-host file transfer queue | ✅ merged |
| MCP backchannel (`zaplex-mcp`) | 📋 planned (post-v1) |
| mosh-grade UDP transport (roaming, predictive echo) | 📋 planned (capability negotiation reserved, transport unbuilt) |
| Mobile companion | 🔭 outlook |

Every designed item has a dated design doc in [`docs/superpowers/`](docs/superpowers/); the product concept is [`docs/zaplex-concept.md`](docs/zaplex-concept.md) (German).

## Install

The first public Zaplex 1.0.0 binary will appear on
[Releases](https://github.com/byte5ai/zaplex/releases) after the documented
runtime matrix passes. The macOS DMG is built in GitHub Actions, signed with
Developer ID, notarized by Apple, and shipped together with its matching Linux
host daemon. Nothing needs to be installed manually on a remote host.

Installation and recovery steps are in the
[Zaplex 1.0 guide](docs/release/1.0-user-guide.md).

## Lineage & acknowledgements

zaplex stands on two excellent projects:

- [**Warp**](https://github.com/warpdotdev/warp) — the terminal core: GPU renderer, blocks, the Rust foundation.
- [**Zap**](https://github.com/zerx-lab/zap) — the open-source, local-first fork that removed the mandatory cloud and wired CLI agents into blocks. From Zap's docs: [migrating from Warp](docs/migrate-from-warp.md) · [Zap roadmap](docs/roadmap.md).

zaplex diverges where they stop: subscription-account orchestration across providers, and a native persistence layer for remote sessions.

## Contributing

Issues and PRs are welcome — [open an issue](https://github.com/byte5ai/zaplex/issues). To see how features are planned here, start with the [concept](docs/zaplex-concept.md) (German) and the dated design docs in [`docs/superpowers/specs/`](docs/superpowers/specs/).

## License

AGPL-3.0 for the app ([LICENSE-AGPL](LICENSE-AGPL), inherited from upstream); the UI crates (`warpui`, `warpui_core`) are MIT ([LICENSE-MIT](LICENSE-MIT)).
