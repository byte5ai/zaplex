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
3. **A cockpit that reads, never phones home.** Account discovery, usage heat, and session states come from read-only parsing of local data (config dirs, JSONL transcripts, session registries) on your machines. No cloud, no account, no telemetry.

**What runs on your hosts:** one binary under `~/.zap/remote-server/`, spoken to exclusively over your existing SSH connection. It keeps session scrollback in bounded RAM, writes no telemetry, and retires itself when idle with no live sessions. Delete the directory and it is gone.

## Features

- **Persistent remote sessions** — agents survive lid-close, network roaming, and app restarts; re-attach replays history seamlessly. No tmux, no byobu, no mosh setup on the host.
- **Agent cockpit** — every subscription account with rolling 5h/week utilization heat, cost, and reset timers; sessions **waiting on you** bubble up as `✋ N waiting`.
- **Native agent awareness** — blocks know when an agent needs input, finished, or got blocked: banner, footer, notification center.
- **Multi-account, multi-provider** — all your Claude and ChatGPT/Codex subscription logins discovered and monitored side by side.
- **Adopt any session** — daemon sessions started elsewhere appear in the sidebar; Enter attaches one as a block, history included.
- **File manager pane mode** — flip any terminal pane into a host-aware file manager; dual-pane cross-host copy is on the roadmap.
- **A full terminal first** — blocks, command palette, SSH host manager, themes: everything the Warp core does, without its cloud.

## Agents

**First-class: [Claude Code](https://github.com/anthropics/claude-code) and [Codex](https://github.com/openai/codex).** zaplex's orchestration layer — multi-account discovery, usage heat, account routing — is built around subscription accounts, because their rolling rate windows are what make heat tracking and "launch on the freest account" meaningful.

**Bring the rest.** zaplex is not an agent and does not ship one. Block-level support (status, banners, notifications) covers Gemini CLI, OpenCode, Copilot, DeepSeek, Goose, and more — inherited from Zap and extended. More deep integrations will follow as the base solidifies.

**zero, natively.** zaplex is adopting [zero](https://github.com/Gitlawb/zero)'s schema-versioned stream-JSON protocol to render headless zero runs as typed, native timelines — tool calls, permission requests with risk levels, usage. zero stays your agent; zaplex becomes its cockpit. (Designed — see status below.)

## Status & roadmap

zaplex is in **early, active development** — no releases yet. Honest state of affairs:

| Area | State |
|---|---|
| Native session daemon: persistence, attach/replay, multi-session, GC | 🔍 daemon core merged; client attach in review ([PR #16](https://github.com/byte5ai/zaplex/pull/16)) |
| Cockpit: account discovery, usage/heat/cost, live session states | 🔍 built, in review |
| File manager pane mode (stage 1) | 🔍 built, in review |
| Fix/ask with *your* agent (routes to your own CLI agent) | 🔍 built, in review |
| Real subscription utilization (OAuth usage endpoint) | 📋 designed |
| Launch wizard + launch-on-freest account routing | 📋 planned |
| Session fork & isolated-worktree launches | 📋 designed |
| zero integration: block support + stream-JSON rendering | 📋 designed |
| Dual-pane cross-host file copy | 📋 planned |
| GitHub flows (quick issue, PR review) | 📋 planned |
| MCP backchannel (`zaplex-mcp`) | 📋 planned (post-v1) |
| mosh-grade UDP transport (roaming, predictive echo) | 📋 planned |
| Mobile companion | 🔭 outlook |

Every designed item has a dated design doc in [`docs/superpowers/`](docs/superpowers/); the product concept is [`docs/zaplex-concept.md`](docs/zaplex-concept.md) (German).

## Install

**Pre-release.** There are no binary releases yet — zaplex currently targets contributors and the adventurous:

- **macOS app:** build from source with a Rust toolchain (dependency list: `script/linux/install_build_deps`; macOS builds need Xcode), or via the repository's GitHub Actions DMG workflow (`test-dmg.yml`).
- **Host daemon:** nothing to install by hand — the app installs and updates it automatically on first connect to a resilience-enabled host.

Watch [Releases](https://github.com/byte5ai/zaplex/releases) for the first tagged builds.

## Lineage & acknowledgements

zaplex stands on two excellent projects:

- [**Warp**](https://github.com/warpdotdev/warp) — the terminal core: GPU renderer, blocks, the Rust foundation.
- [**Zap**](https://github.com/zerx-lab/zap) — the open-source, local-first fork that removed the mandatory cloud and wired CLI agents into blocks. From Zap's docs: [migrating from Warp](docs/migrate-from-warp.md) · [Zap roadmap](docs/roadmap.md).

zaplex diverges where they stop: subscription-account orchestration across providers, and a native persistence layer for remote sessions.

## Contributing

Issues and PRs are welcome — [open an issue](https://github.com/byte5ai/zaplex/issues). To see how features are planned here, start with the [concept](docs/zaplex-concept.md) (German) and the dated design docs in [`docs/superpowers/specs/`](docs/superpowers/specs/).

## License

AGPL-3.0 for the app ([LICENSE-AGPL](LICENSE-AGPL), inherited from upstream); the UI crates (`warpui`, `warpui_core`) are MIT ([LICENSE-MIT](LICENSE-MIT)).
