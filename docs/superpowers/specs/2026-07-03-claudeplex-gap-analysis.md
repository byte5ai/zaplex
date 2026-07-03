# Gap-Analyse: claudeplex + claudeplex-desktop → zaplex (Stand 2026-07-03)

> Vollinventar beider Referenz-Repos (CLI `~/projects/zaplex/claudeplex`, Desktop
> `~/projects/zaplex/claudeplex-desktop`), gemappt gegen zaplex' Ist-Stand auf
> `feat/stage2-client-attach`. Legende: ✅ vorhanden (nativ adaptiert) · 🟡 teilweise ·
> ❌ fehlt. Prioritäten folgen dem Increment-Plan C3–C5 der
> Cockpit-Native-Integration (2026-07-01-cockpit-native-integration-design.md).

## (a) Account / Usage / Kosten

| Feature (Referenz) | zaplex | Anmerkung |
|---|---|---|
| Account-Auto-Discovery (`~/.claude*`, Prozesse, env) | ✅ | Spine `zaplex_cockpit::claude`; zaplex kann zusätzlich **Codex** (Referenz ist Claude-only) |
| Token-Accounting 5h/heute/Woche + Kosten | ✅ | Spine + Sidebar (C2) + Haupt-Pane (C2b, inkl. Wochen-Heat) |
| 5h-/Wochen-Reset-Timer | ✅ | |
| Soft-Budgets konfigurierbar | ✅ | Settings `cockpit.budget_5h/_week` |
| **Echte OAuth-Usage** (`api.anthropic.com/api/oauth/usage`, Token aus `.credentials.json`, 15-min-Cache, Fallback auf Schätzung) | ❌ | **Desktop-Juwel — höchster Genauigkeitsgewinn.** → C3/C4 |
| **Plan-basierte Budget-Schätzung** (Enterprise/Team/20x/Pro/Max-5x statt Flat-Default, „est"-Badge) | ❌ | klein, hoher Wert → C3 |
| Per-Modell-Sublimits (Opus-/Sonnet-Bars) | ❌ | Teil „per-model heat" in C3 |
| Manuelle Account-Overrides (`instances.json`: Label/Farbe/Order/hide) | ❌ | klein → C3 |
| Aggregat-Rollup-Header (live/working/waiting-Counts) | 🟡 | Kosten-Rollup ✅ (C2b); Status-Counts fehlen (brauchen (b)) |

## (b) Session-/Instanz-Monitoring — **größte Lücke, Herz von claudeplex** → C3

| Feature | zaplex | Anmerkung |
|---|---|---|
| Live-Registry lesen (`<config>/sessions/*.json`, PID-alive-Check) | ❌ | Quelle der Wahrheit für „läuft wirklich" |
| Status-Ableitung aktiv/**wartet**/monitor/stale (`stateOf`: busy → aktiv; `ended` aus Transcript-Endezeile → wartet; Tool-Lauf/bg → monitor) | ❌ | Kern-Algorithmus, pur portierbar in die Spine |
| Account-Status WORKING/LIVE/IDLE/OFFLINE | ❌ | |
| „Waiting on you"-Liste zuerst + Konsolen-Fokus | ❌ | UI über (b)-Daten |
| Live-Output-Tails (letzte Aktivität je Session) | ❌ | |
| Flash bei Statusübergang + **Notification aktiv→wartet** | ❌ | zaplex hat Notification-Infra — nur verdrahten |
| Kontext-Füllstand/Modell je Session | ❌ | |
| FS-Watcher statt Poll (Desktop) | 🟡 | CockpitModel nutzt HomeDirectoryWatcher + 45s-Reconcile — Registry-Pfade zusätzlich watchen |

**zaplex-Vorteil:** Sessions, die zaplex selbst fährt (Panes/Daemon), sind exakt bekannt —
die Registry-Ableitung braucht es nur für extern gestartete CLIs. Unified Inventory
(Design §3.4) = live Panes + Daemon-Sessions + Registry/Transcripts.

## (c) Agent-Steuerung (Cockpit-Drive)

| Feature | zaplex | Anmerkung |
|---|---|---|
| Agent in echtem interaktiven PTY (Subscription-Billing, kein `-p`) | ✅ | nativ: Agent läuft im Terminal-Pane |
| Nachricht senden / Raw-Keys / Menüs bedienen | ✅ | ist ein echtes Terminal |
| Bild anhängen (Paste/Drag) | 🟡 | Pfad-Paste geht; komfortables Clipboard-PNG→Datei→Pfad fehlt |
| **Adopt idle Session** („intake": tippen übernimmt in-place, `--resume <id>`) | ❌ | → C3/C4; zaplex-Analog: Tab öffnen mit `claude --resume <id>` im richtigen cwd |
| **Watch-Mode** (Session läuft woanders → read-only Transcript-Follow) | ❌ | → C3 (Transcript-Follow reicht) |
| **Fork „Branch a copy"** (`--fork-session`) | ❌ | klein, an Transcript-/Session-UI andocken |
| Restart (Fork-Resume: neue Plugins/MCP, Historie bleibt) | ❌ | klein |
| Rename (`--name` / Override) | ❌ | klein |
| Agent-Persistenz über App-Restart (Roster + resume) | 🟡 | zaplex-Panes persistieren; „resume der CLI-Session im Pane nach Restart" fehlt |

## (d) Multi-Host / Remote

| Feature | zaplex | Anmerkung |
|---|---|---|
| Persistente Remote-Sessions (tmux-RC-Fleet) | ✅ **besser** | nativer Daemon (überlebt Drop/Deckel), Auto-Install, Adopt-Sidebar |
| SSH-Host-Verwaltung + `~/.ssh/config`-Kandidaten | ✅ | on-demand beim Hinzufügen |
| **Tailscale-Peer-Discovery** | ❌ | klein: `tailscale status --json` in die Kandidaten |
| 2-Pane-Dateibrowser host↔host + Copy ohne scp-Dance | 🟡 | FM-Pane-Mode P1 ✅ (lokal+remote, in-slot); **host↔host-Copy = FM P2/P3** |
| Shell-Out in echtes PTY | ✅ | ist ein Terminal |
| **Conductor: Host▸Projekt▸Session-Baum mit „needs-me"-Bubbling** | ❌ | Leit-UI für C3/C5-Inventar |
| RAM-Governor (Ceiling, „N Sessions passen noch", opt-in Reaping) | 🟡 | per-Session Ring-Ceiling + Daemon-GC ✅; globales RAM-Budget + Anzeige fehlt |
| Remote-Fleet-Aggregation (Fleet-Daten fremder Hosts) | ❌ | → C5; zaplex-Analog: `list_sessions` über alle Hosts aggregieren |

## (e) Git / GitHub — komplett offen → C5

| Feature | zaplex | Anmerkung |
|---|---|---|
| Repo-Identität (owner/repo, Worktree-Kennung) | 🟡 | git-Infra vorhanden (code_review); Cockpit-Anzeige fehlt |
| **Quick-Issue** (Instanz entwirft engl. Issue → Review-Loop → `gh issue create`) | ❌ | |
| **PR-Review** (listen → freie Instanz analysiert → approve/comment/merge via `gh`) | ❌ | |
| **Issue-Triage** (Desktop: type/priority/actionable → comment/close) | ❌ | |
| „Freieste Instanz"-Picker für Hintergrund-Analysen | ❌ | Baustein auch für C4 |
| Lenientes JSON-Parsing der Modell-Antworten | ❌ | Hilfsbaustein |

## (f) Launch / Routing → C4 (Anforderungen bereits festgehalten)

| Feature | zaplex | Anmerkung |
|---|---|---|
| New-Agent-Wizard (Instanz×Folder, beide Richtungen) | ❌ | = **C4-Launcher** („on Host in Dir"), App-Level-Regel dokumentiert |
| Quick-Launch mit Folder-History + Pfad-Completion | ❌ | |
| **Account-Pinning** (`CLAUDE_CONFIG_DIR` je Account, Default ohne, API-Key-Scrub) | ❌ | Kern von C4-Routing |
| Launch-on-freest (5h-Last-Sortierung) | ❌ | Spine-Heat existiert — Picker fehlt |
| Fresh vs. Resume (`--session-id`/`--resume`, natives `--name`) | ❌ | |
| „Ask my agent"-Primitive (Kontext→Prompt→CLI) | ✅ **neu** | `AskAgent` (heute Nacht, dbb1628a) — P2: Command-Error/Block-Kontext |

## (g) Transcript / History → C3/C5

| Feature | zaplex | Anmerkung |
|---|---|---|
| Claude-CLI-Transcript-Parser (jsonl → Turns, Hook-Strip) | ❌ | pur → in die Spine |
| Transcript-Viewer (Markdown, Diffs/Tools als Blöcke; live/watch/intake) | ❌ | zaplex' ConversationListView deckt nur Agent-Mode-Konversationen |
| Folder-Historie je Account (recency) | ❌ | Baustein für C4-Wizard |

## (h/i) Shell/Sonstiges

- Command-Palette ✅ · Toolbelt ✅ · Themes ✅ (eigenes System) · Markdown ✅ ·
  Install/Update ✅ (DMG; Autoupdate-Flag) · Clipboard-PNG 🟡 · **i18n DE ❌**
  (Referenz hat DE/EN; zaplex nur EN — ftl-Infra vorhanden).

## Priorisierung (Vorschlag)

1. **C3a — Status-Spine** (Registry + stateOf + ended-Erkennung + Transcript-Parser in
   `zaplex_cockpit`; pur, testbar) → macht Sidebar/Pane „lebendig": waiting-first,
   Status-Counts, Notification aktiv→wartet.
2. **C3b — echte OAuth-Usage + Plan-Budgets + Account-Overrides** (Genauigkeit).
3. **C4 — Launcher/Routing** (Wizard „on Host in Dir", Account-Pinning, launch-on-freest,
   resume/fork/rename/adopt) — bringt die entlisteten Titelbar-Buttons korrekt zurück.
4. **C5 — GitHub-Flows** (Quick-Issue, PR-Review, Triage) + Fleet-Aggregation + Conductor-Baum.
5. Kleinkram parallel: Tailscale-Discovery, Clipboard-PNG, i18n-DE, RAM-Governor-Anzeige.
