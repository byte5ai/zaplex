# Public README — holistic concept (positioning, story, voice, structure)

> **Status:** Design + first execution 2026-07-03 (the README itself ships with this doc; assets follow)
>
> **Quelle der Entscheidung:** User 2026-07-03 — "richtige README" via three steps: analyze top repos in the category → derive a holistic concept (audience, storytelling, wording, not just an outline) → fill it with the current state. Strategic frame: position zaplex toward the OSS-agent community (zero) as the natural terminal partner, never a competitor.
>
> **Analysis basis:** structure/tone/storytelling review of ghostty, waveterm, zellij, crush, opencode, zed, zero, and upstream Warp (2026-07-03). Key findings distilled in §2.
>
> **Goal:** A README that makes an unknown fork legible in 30 seconds, earns trust for its scariest feature (a daemon on your servers), and reads as a product — while staying strictly honest about pre-release status.

---

## 1. Audience — three readers, one page

| Reader | Arrives from | Must find within |
|---|---|---|
| **Remote-dev power user** running Claude Code/Codex on remote hosts (the concept-§1 Zielgruppe, incl. "vibecoders") | HN, GitHub search, word of mouth | 10 s: what it is + the one thing nobody else does; 60 s: how sessions survive + what the cockpit shows |
| **OSS-agent community member** (zero, opencode, …) who owns their agent and wants a better surface for it | zero ecosystem, cross-listing | the "bring your agent" stance + the zero paragraph: partner, not competitor |
| **Contributor / fork-watcher** (Warp/Zap ecosystem) | upstream repos | lineage honesty, license clarity, where the concept/specs live |

The README serves reader 1 first. Readers 2 and 3 get dedicated, short, honest sections — not the headline.

## 2. What the category analysis dictates (rules we adopt)

1. **One noun.** Every strong README sells exactly one noun (ghostty=terminal, zellij=workspace, zero=agent). Ours: **cockpit** — anchored in the category word "terminal". Daemon, file manager, multiplexer-replacement are features *under* it, never co-headliners.
2. **Ghostty's trilemma frame** is the highest-leverage sentence structure in the sample: name the forced trade-off, refuse it. Ours: *real terminal vs. surviving sessions vs. agent overview — zaplex refuses to make you choose.*
3. **Substantiated confidence, zero hype adjectives.** Claims get mechanisms ("replay ring on the host"), not superlatives. "Premium" is demonstrated (calm UI, honest tables), never said.
4. **Honesty as a trust device.** Ghostty ends its roadmap with ❌; zero documents its own install bug. For a pre-release fork this is our strongest available signal: an explicit status table (✅ built / 🔍 in review / 📋 designed / 🔭 outlook) replaces vaporware suspicion with credibility.
5. **≤ 7 feature bullets**, bold keyword + one clause (crush's format). Everything else lives in docs/.
6. **A trust section is mandatory** in this category: zaplex installs a daemon on the user's hosts — the category's scariest ask. Adopt zero's "Safety Model" pattern: what runs where, what is stored, what phones home (nothing), how it degrades.
7. **Name agents, not terminal competitors.** Warp's move ("bring your own CLI agent: Claude Code, Codex, …") signals openness. tmux/mosh may be named — they are the incumbent *workflow* being replaced, and the reader's mental model.
8. **Fork lineage up top, gratefully.** "Built on Zap, the open-source Warp fork" converts a criticism into inherited credibility (proven GPU renderer, Rust, native macOS).
9. **Front-door, not manual.** No docs site exists; depth lives in `docs/` (concept + specs). The README links into it instead of inlining configuration prose.

## 3. Storytelling & voice

- **Narrative arc (top to bottom):** *the pain* (agents die with your SSH connection; ten sessions, no overview) → *the refusal* (trilemma) → *the mechanism* (daemon → real PTYs → cockpit, 3 steps) → *proof of intent* (honest status table) → *the invitation* (bring your agent; build with us).
- **The emotional core is promise #2 of the concept ("Nichts bricht ab"):** close the laptop mid-run, reopen, everything is still there *with history*. Every visual asset and the tagline sub-line serve this scene.
- **Voice:** declarative, technically precise, second person for benefits ("your agents", "your hosts"). Confident but never salesy; the crush-style playfulness does **not** fit a trust-first infrastructure product. English throughout (repo language rule).
- **Positioning stances, stated plainly:** toward agents — "zaplex is not an agent; bring yours." Toward zero — partner sentence + protocol adoption (see `2026-07-03-zero-agent-integration-design.md` Z3: cross-listing needs user confirm). Toward Warp/Zap — grateful lineage, clear divergence statement (subscription-orchestration + persistence layer are ours).

## 4. Structure (the schema)

```
[centered]  # zaplex  (text-only H1 until a logo exists)
            tagline  — "The terminal cockpit for AI coding agents on remote machines."
            sub-line — the lid-close scene in ≤ 14 words
            nav row  — About · How it works · Features · Status · Install · Docs
            2 badges — license · platform (no CI badge until pipelines are steady-green)
            [HERO SLOT — GIF placeholder, see §5]
About        — trilemma ¶ + what-it-is ¶ (lineage sentence here) + the three promises (concept §1)
How it works — numbered 3-step mechanism + "What runs on your hosts" trust block (rule 6)
Features     — exactly 7 bullets (rule 5)
Agents       — Claude Code + Codex first-class (subscription orchestration); inherited block
               support list; the zero partnership paragraph (honest: designed, not shipped)
Status & roadmap — the honest table; PR #16 named for in-review items
Install      — pre-release honesty: no releases yet; source build + CI DMG path; daemon
               auto-install note ("the app installs/updates the daemon itself")
Fork lineage & acknowledgements — Warp → Zap → zaplex, one line each + thanks
Contributing — issues/discussions; pointer to docs/zaplex-concept.md (German) + specs
License      — AGPL-3.0 app, MIT UI crates, one plain sentence
```

## 5. Visual & asset plan (currently: zero usable assets)

No zaplex logo exists (`assets/` holds only Zap's). Interim (decided 2026-07-03): an ASCII wordmark (`<pre>`, ANSI-shadow style) serves as the logo, and an ASCII architecture diagram opens "How it works" — terminal-native, dark/light-safe, zero asset dependency. Real asset backlog, in impact order — all require the macOS app, i.e. the maintainer's machine:

1. **Hero GIF (the money shot):** cockpit visible, agent running → lid closes / network drops → reopen → session alive with replayed history. ~10 s loop, dark theme.
2. **Cockpit screenshot** — accounts with heat bars + `✋ waiting` sessions.
3. **Logo** — separate design task; not blocking (text H1 until then).

The README ships with an HTML-comment placeholder in the hero slot so the structure is ready for the drop-in.

## 6. Repo-metadata follow-ups (each needs explicit user confirm — remote mutations)

- GitHub **description** still describes Zap → proposal: "The terminal cockpit for AI coding agents on remote machines. Native macOS, persistent remote sessions, agent cockpit — built on Zap/Warp."
- **Homepage** points to `zap.zerx.dev` → clear (no zaplex site yet) or point to the concept doc.
- **Topics** are empty → propose: `terminal`, `macos`, `rust`, `ai-agents`, `claude-code`, `codex`, `remote-development`, `warp`.
- **Stale translations:** `README.ja.md` / `README.zh-CN.md` were Zap's content and contradicted the new README → **decided & removed 2026-07-03** (User; history keeps them, upstream owns the originals).

## 7. Maintenance rules

- The status table is updated **in the same PR** that changes a feature's state — a stale "honest table" is worse than none.
- Tagline and the seven bullets change only with a deliberate positioning decision, not feature-by-feature.
- Anything longer than one clause per feature moves to `docs/` — the README never becomes the manual.
