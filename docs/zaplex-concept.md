# Claudeplex × Zap — Konzept für den Zap-Fork

> **Status:** Konzept, festgelegt 2026-06-22 mit dem User. Umsetzung in eigener Session ab 2026-06-23 im Fork-Repo [iret77/zaplex](https://github.com/iret77/zaplex).
>
> **Was hier steht:** Mission, Designprinzipien, Architektur, UX, erste Schritte. Genug, dass eine kalt gestartete Claude-Code-Session loslegen kann, ohne die Diskussionshistorie zu kennen.
>
> **Was hier NICHT steht:** Implementierungs-Details der Rust-Crates. Die entstehen in der Session selbst — geleitet von den Prinzipien unten, nicht prescripted.

---

## 1. Mission

Aus claudeplex (Bun-TUI) wird ein nativer Teil von **Zap** (open-source Warp-Fork von [zerx-lab/zap](https://github.com/zerx-lab/zap)). Ergebnis: ein einziges Tool, das

- die **MC-Hälfte** liefert (Host-Zugriff, Dual-Pane-Files, Cross-Host-Copy) — kommt großteils von Zap selbst,
- die **Claude-Hälfte** liefert (Multi-Max-Account-Orchestrierung, Fleet, Conductor) — die fehlende Schicht, die wir beitragen,
- und **Session-Bedienung + Prompting** als first-class-Erlebnis vereint — der eigentliche Wow-Moment.

Es ersetzt das standalone claudeplex-TUI und Marcels Electron-App (`byte5ai/claudeplex-desktop`) — beide haben bisher einen Terminal-/SSH-Unterbau nachgebaut, den Zap fertig mitbringt. Diese Redundanz schneiden wir weg.

**Zielgruppe:** der User selbst. Kein Produkt für Dritte. Das heißt: kompromisslos „geil" statt „skalierbar". Wenn ein Feature den Workflow nicht spürbar verbessert, fliegt es raus.

---

## 2. Designprinzipien — nicht verhandelbar

### 2.1 Es muss sich nativ anfühlen

Dies ist die Kern-Akzeptanzregel. Wenn ein Reviewer einen Screenshot sieht und es als „angepappten Sidebar" erkennt, haben wir verloren.

**Konkret heißt das:**

- **Zaps visuelle Sprache vollständig erben.** Keine eigene Farbpalette, keine eigenen Border-Stile, keine eigene Typographie, kein eigenes Spacing. claudeplex' `theme.ts`/`ui.ts` waren Eigenleistung für ein Standalone-Tool — im Zap-Fork sind sie **Anti-Pattern**. Wir nutzen Zaps Theme-System, sonst nichts. Die Lumen-Themes werden NICHT mit übernommen.
- **Zaps Interaktionsmuster erben.** Cmd-K-Command-Palette, Block-System, Slash-Befehle, Block-Header-Konventionen, Notification-Center — alles wie es Zap macht. Wenn Zap einen Hotkey für „nächste Notification" hat, nutzen wir den auch für „nächste wartende Session". Wir erfinden keine parallelen Konventionen.
- **Zaps Icon-Family erben.** Falls Zap eine Icon-Sammlung hat (Lucide/eigene), übernehmen. Keine claudeplex-spezifischen Unicode-Glyphen mehr.
- **Sidebar-Patterns kopieren von `warp_ssh_manager`.** Wenn der SSH-Manager-Panel Akkordeons nutzt, wir auch. Wenn er einen bestimmten Selektions-Stil hat, wir auch.
- **Native Look ist wichtiger als Feature-Vollständigkeit.** Lieber 80% der Features mit perfektem Nativ-Gefühl als 100% mit Stilbrüchen.

### 2.2 Saubere Session-Bedienung & Prompting

Der explizit hervorgehobene Wunsch. Was „sauber und nice" konkret bedeutet:

- **Eine Session = ein Zap-Block.** Nicht ein eigenständiges Panel, nicht ein Tab, ein **Block** — wie Zaps existierende CLI-Agent-Integration (Claude Code, Codex, agy) sie verwendet. Block-Output ist Block-Output, Block-Eingabe ist Block-Eingabe.
- **Inline-Prompting im Block.** Eingabezeile direkt am Block-Ende. ⏎ schickt in die laufende Session. Slash-Commands gehen durch. Clipboard-Bilder per Cmd-V. Genauso wie wenn man `claude` direkt im Terminal startet — nur mit Multiplexer drumherum.
- **Mehrere Sessions parallel sichtbar.** Split-Pane / Tile-Layout, nicht „eine zur Zeit". Das ist das, was claudeplex' Cockpit-Mode konzeptionell wollte, aber als TUI nie wirklich geliefert hat. In Zap mit echten Blocks ist es selbstverständlich.
- **Cross-Session-Navigation per Tastatur.** Cmd-1..9 zu den ersten N Sessions, Cmd-Tab zur nächsten aktiven, ein dedizierter Hotkey („go to next waiting"). Maus optional, nie verpflichtend.
- **Adopt-by-session-id muss sich anfühlen wie „die Session war schon hier".** Wartende Session in Sidebar → Enter → öffnet als Block, history visible, ready to prompt. Kein „adoption ritual", kein Modal, kein „connect"-Schritt.
- **Account-Awareness im Block-Header.** Jeder Block zeigt, gegen welchen Account er läuft (kleiner Indicator, Zaps Stil, nicht aufdringlich). Beim Launch eines neuen Agenten wird der Account vorausgewählt (freester) — aber im Block-Header umstellbar, falls man bewusst einen anderen Account will.

### 2.3 Keine Krücken, keine Spar-Implementierungen

- Wenn etwas „eigentlich richtig" eine Woche braucht und „pragmatisch" einen Nachmittag, nehmen wir die Woche.
- Wenn ein Feature nur als visueller Glance Sinn ergibt, bauen wir es als visueller Glance — nicht als Tool-Aufruf, nicht als Slash-Command, nicht als MCP-Wrapper.
- Wenn die Wahl zwischen „eigene Konvention" und „Zaps Konvention erweitern" steht: Zaps Konvention erweitern, auch wenn es mehr Arbeit ist.
- MCP ist ergänzende Beigabe (siehe §7), niemals Ersatz für UI.

---

## 3. Ist-Zustand (Referenz)

### 3.1 Was Zap fertig liefert

Quelle: [zerx-lab/zap](https://github.com/zerx-lab/zap), Stand 2026-06-21.

- **Terminal-Engine** (GPU-rendered, Block-basiert, Historie)
- **`warp_ssh_manager`** — SSH-Hosts, tmux-Integration, Sessions
- **`warp_files`** — Terminal-File-Handling (Drag-Drop, Inline-Preview, File-URLs)
- **AI-Provider-Routing** (BYOP) — Anthropic, OpenAI, Gemini, DeepSeek, Ollama nativ + beliebige OpenAI-kompatible Endpoints
- **CLI-Agent-Adapter** — Claude Code, Codex, agy bereits als Blocks verdrahtet, OSC9/777-Routing in Notification-Center
- **MCP-Client**
- **`settings`/`warpui`/`warpui_core`** — UI-Framework (UI-Crates sind MIT-lizenziert, der Rest AGPL-3.0)
- **61 Crates insgesamt**, klare Trennung
- **Aktiv** (täglich Commits, 1.8k ⭐, ~34 offene Issues)

### 3.2 Was claudeplex einbringt (Lücken-Liste)

Die Schicht, die Zap aus konzeptionellem Grund weglässt (Roadmap: „single account/identity shared across surfaces").

- **Multi-`CLAUDE_CONFIG_DIR`-Discovery** — mehrere Claude-Max-Logins parallel
- **Per-Account-Budget-Tracking** — 5h- und Wochen-Fenster, Heat-Coloring, Reset-Countdown, Kosten
- **Account-Routing** — „launch on freest" als Default beim Agent-Start
- **Cross-Account-Session-Inventar** — alle laufenden/wartenden/recent Sessions über alle Accounts/Hosts
- **„Needs me"-Bubbling** — „● N waiting" + Hotkey-Jump
- **Persistente Remote-Fleet** — `claude remote-control`-Server in tmux auf SSH-Hosts (überlebt Lid-Close, bedient auch Claude-Mobile-App)
- **RAM-Governor für die Fleet** — ~330 MB/Session, harter Ceiling
- **Adopt-by-session-id** — Session, die in einer anderen Shell gestartet wurde, hier weiterführen
- **MC-style Dual-Pane File Manager** — Host↔Host-Copy ohne scp, weil `warp_files` Single-Pane ist

### 3.3 Datenschicht in Bun, schon getestet

claudeplex' `--json`-Endpoint liefert `{accounts, remote}` mit allen oben genannten Daten:

```bash
claudeplex --json
# → {accounts: [...], remote: {...}}
```

Diese Logik ist getestet, performant, läuft auf Linux + macOS. Wir bauen sie **NICHT** auf Anhieb nach Rust um (siehe §6.1).

---

## 4. Architektur

### 4.1 Schichten-Modell

```
┌──────────────────────────────────────────────────────────────┐
│  UI Layer (Rust, in warpui / warp_terminal)                  │
│  - Account Dock (Sidebar-Panel)                              │
│  - Agent Tree (unter dem Dock)                               │
│  - Launch Wizard (Modal)                                     │
│  - Block-Header-Extension (Account-Indicator)                │
│  - Hotkey Registration (next-waiting, switch-session)        │
│  - MC Dual-Pane View (separater Modus)                       │
└──────────────────────────────────────────────────────────────┘
              ▲                                  ▲
              │                                  │
┌─────────────┴──────────────┐    ┌──────────────┴─────────────┐
│  Action Layer (Rust)       │    │  Data Layer (Bun → Rust)   │
│  - Launch agent            │    │  v0: spawnt `claudeplex    │
│  - Adopt session           │    │      --json` und parst     │
│  - Steer (send to block)   │    │  v1: native Rust ports von │
│  - PR-review / quick-issue │    │      collect.ts /          │
│  - Fleet control           │    │      discover.ts /         │
└────────────────────────────┘    │      usage.ts              │
              │                   └────────────────────────────┘
              ▼
   Zap's existing block / agent infrastructure
   (claude wird gespawnt wie jeder andere CLI-Agent;
    nur mit dem richtigen CLAUDE_CONFIG_DIR und stdin-Pipe)
```

### 4.2 Crate-Layout

Eigene Crates mit klarem `claudeplex_`-Prefix. Vorteil: bei jedem Rebase mit Zap-Upstream ist offensichtlich, was „unseres" ist.

| Crate                  | Inhalt                                                      | Größe (geschätzt) |
|------------------------|-------------------------------------------------------------|-------------------|
| `claudeplex_accounts`  | Discovery, Usage-Parser, Heat-Logik, Routing                | mittel            |
| `claudeplex_sessions`  | Live-Inventar, Waiting-Detection, Adoption, Send-to-block   | mittel            |
| `claudeplex_fleet`     | Remote-control-Supervisor (tmux), RAM-Governor              | mittel            |
| `claudeplex_mc`        | Dual-Pane-File-Manager (SFTP-aware, Host↔Host-Copy)         | groß              |
| `claudeplex_ui`        | UI-Komponenten (Account-Dock, Agent-Tree, Launch-Wizard)    | mittel            |

**UI-Einhängung** passiert in **so wenig fremdem Code wie möglich**:
- `warpui` / `warp_terminal`: minimaler Patch, der unsere Panels registriert und Hotkeys bindet
- Alles andere lebt in unseren eigenen Crates

Diese Disziplin ist die Maintenance-Versicherung. Je weniger Zeilen wir in geerbten Crates ändern, desto weniger Rebase-Schmerz.

### 4.3 Datenfluss (v0 — hybrid)

```
claudeplex Binary (Bun, existiert)
  │
  ▼
claudeplex --json --watch       ← neuer Flag in Bun, streamt Updates als NDJSON
  │
  ▼ stdout
claudeplex_accounts (Rust)      ← spawnt Subprocess, parst NDJSON
  │
  ▼
Internal State (Rust structs)
  │
  ▼
UI updates via Zap's reactive system
```

**Warum hybrid?** v0 muss in Wochen, nicht Monaten laufen. Die Bun-Logik ist getestet. Die Prozess-Grenze ist sauber (kein FFI-Tanz, kein Memory-Sharing-Gefrickel). Zap hat schon Subprocess-Infrastruktur. Das ist **kein Hack** — das ist eine bewusste Schichten-Grenze.

**Wann v1 (nativ Rust)?** Wenn v0 sich bewährt UND der Bun-Hop spürbare Latenz/Bugs verursacht. Vorher portieren wir nichts. Wir portieren auch nicht „on principle" — wir portieren, wenn es weh tut.

### 4.4 Action-Layer

Aktionen rufen entweder existierenden claudeplex-Code auf (über CLI) oder nutzen Zap-Mechanismen direkt:

| Aktion                  | v0 Implementierung                                                |
|-------------------------|--------------------------------------------------------------------|
| Launch agent            | `claude` als Subprocess mit `CLAUDE_CONFIG_DIR=<acct>` → wird zu Block |
| Adopt session           | `claude --resume <session-id>` mit richtigem `CONFIG_DIR`         |
| Steer (prompt senden)   | stdin des Block-Subprocesses, exakt wie Zap es heute schon macht  |
| PR-review               | `claudeplex` CLI als Subprocess (existierende headless `-p` Logik)|
| Fleet control           | Bun-CLI als Subprocess (existierender `--json` Output)            |
| Remote-fleet-Server     | `ssh <host> claude remote-control ...` (existierender Code)      |

**Wichtig:** Wir bauen keinen eigenen `send-to-pty`-Layer. Zap hat den schon. Wir hängen uns dran.

---

## 5. UX-Design

### 5.1 Account Dock (Sidebar-Panel)

**Position:** linke Sidebar, oberster Bereich, oberhalb von Zaps existierender SSH-Host-Liste.

**Inhalt:** ein Eintrag pro entdeckter Account. Pro Eintrag:
- Account-Label (aus dem Account-Setup übernommen)
- Mini-Heat-Bar für 5h-Fenster (winzig, eine Zeile, Zaps Progress-Bar-Stil)
- Mini-Heat-Bar für Wochenfenster
- Reset-Countdown bei Hover oder im expanded state
- Aktueller Status (idle / working / waiting) als Farb-Indicator, NICHT als Text-Pille

**Stil:** wie Zap seine SSH-Hosts darstellt. Wenn Zap dort eine bestimmte Border, ein bestimmtes Spacing, ein bestimmtes Hover-Verhalten hat — kopieren wir es exakt.

**Aktion:** Click auf einen Account-Eintrag → öffnet ein Submenü/Akkordeon mit den Sessions auf diesem Account.

### 5.2 Agent Tree (unter dem Dock)

**Position:** linke Sidebar, unterhalb des Account Docks.

**Hierarchie:** Host ▸ Projekt ▸ Session. Aktive Sessions oben, wartende unter eigenem Header, kürzliche/idle weiter unten.

**Status-Anzeige:** keine Glyph-Soup. `[WORK]` / `[WAIT]` / `[IDLE]` als textuelle Badges in Zaps Badge-Stil, oder reine Farb-Indicator-Punkte — je nachdem, was Zap als Pattern hat.

**Top-Indicator:** „● N waiting" als kleiner Counter im Tree-Header (nicht in der App-Topbar — das wäre außerhalb unseres Scopes).

### 5.3 Launch Wizard

**Trigger:** Hotkey (Vorschlag: `Cmd-Shift-N`, falls Zap das nicht bereits belegt; sonst was Vergleichbares).

**Form:** Modal im Zap-Modal-Stil. Drei Felder:
1. **Account** — vorausgewählt mit freestem Account, dropdown alphabetisch
2. **Folder** — Combobox, gespeist aus History (claudeplex' bestehender `discover.ts` liefert das)
3. **Initial prompt** (optional) — Textarea, ⏎ sendet auch direkt

**Verhalten:** ⏎ launcht den Agenten in einem neuen Block, fokussiert den Block, scrollt zur Eingabezeile.

### 5.4 Block-Header-Extension

Jeder Session-Block bekommt im Header (oder unten als Status-Zeile, je nachdem wo Zap Header-Info platziert) zwei Mikro-Indicators:

- **Account-Badge** — welcher Account läuft hier
- **Budget-Mikro-Heat** — eine winzige Bar oder Punkt, der die Account-Last reflektiert

Beides klein, nicht aufdringlich. Hover gibt mehr Details. Click auf den Account-Badge öffnet einen Account-Switcher (wenn man bewusst auf einen anderen Account umstellen will — aber das ist Edge-Case, default ist beim Launch festgelegt).

### 5.5 MC Dual-Pane View

**Position:** ein eigener Mode/View. Zap kann Splits — wir nutzen einen Split, der explizit den MC-Modus aufmacht.

**Layout:** klassisch MC: linke Pane, rechte Pane, Funktionsleiste am unteren Rand (F1 Help, F5 Copy, F6 Move, F7 Mkdir, F8 Delete, F10 Quit) — oder Zaps Äquivalent davon. Wenn Zap eine Hotkey-Konvention hat, der sich daran halten lässt: gut. Wenn nicht: F-Keys, weil sie MC-User erwarten.

**Beide Panes können auf verschiedenen Hosts sein.** Das ist die Killer-Feature der MC-Hälfte — links macmini, rechts devhost, F5 copy → SFTP-Transfer.

**Verhalten zum Restsystem:** Wenn man in einem File-Block einen `.jsonl`-Claude-Transcript markiert und Enter drückt → öffnet als read-only Viewer mit claudeplex' Markdown-Renderer-Logik. Das ist die einzige Stelle, wo MC und Agent-Schicht direkt verzahnt sind.

### 5.6 Hotkey-Map (Vorschlag, an Zap anzupassen)

**Konvention:** wenn Zap einen Hotkey für eine semantisch ähnliche Aktion hat, übernehmen wir den. Was hier steht, sind Defaults, falls Zap keine Vorgabe macht.

| Aktion                          | Hotkey (Vorschlag)        |
|---------------------------------|---------------------------|
| Next waiting session            | `Cmd-Shift-W`             |
| Switch to next session block    | `Cmd-Tab` (oder Zaps Tab) |
| Switch to session 1..9          | `Cmd-1` .. `Cmd-9`        |
| New agent (Launch Wizard)       | `Cmd-Shift-N`             |
| Quick Issue                     | `Cmd-Shift-I`             |
| PR Review                       | `Cmd-Shift-P`             |
| Open MC Dual-Pane               | `Cmd-Shift-F`             |
| Adopt session under cursor      | `Enter` in Agent Tree     |

---

## 6. Roadmap

### 6.1 v0 — „funktioniert geil, hybrid intern" (Wochen)

**Scope:** Account Dock + Agent Tree + Launch Wizard + Inline-Prompting in Blocks. Datenschicht aus Bun via `claudeplex --json`. Noch keine MC-Pane, noch keine eigene Fleet-Steuerung im UI (Fleet existiert weiter via Bun-CLI).

**Definition of done:**
- Account Dock zeigt alle Max-Accounts mit korrektem Heat
- Agent Tree zeigt alle Sessions korrekt, „N waiting" stimmt
- Launch Wizard startet Agenten auf freestem Account, Block öffnet, Prompt geht durch
- Adopt-by-Enter funktioniert: Session aus Sidebar → Block, history visible, prompt funktioniert
- Visuelle Abnahme: 3 unbeteiligte Screenshots, niemand erkennt „angeflanscht"

### 6.2 v1 — „nativ und sauber" (Monate)

**Scope:** Bun-Datenschicht nach Rust portieren (`discover.ts`/`collect.ts`/`usage.ts` → `claudeplex_accounts`-internals). Eigene Fleet-Steuerung im UI (Start/Stop von remote-control-Servern aus dem Account Dock heraus). MC Dual-Pane.

**Trigger für den Start:** v0 läuft seit X Wochen ohne Krücken-Gefühl. Bun-Subprocess wird als spürbare Latenz/Fragility erlebbar.

### 6.3 v2 — „upstream contribution oder permanent fork" (offen)

**Optional:** `claudeplex_accounts` als Patch-Set an Zap anbieten. Multi-Identity ist auf Zaps Roadmap als Lücke benannt. Wenn der Maintainer annimmt: Rebase-Last weg.

Falls nicht: private Fork läuft weiter, kein Drama.

---

## 7. MCP — ergänzende Rolle

MCP ist **nicht** Ersatz für UI (siehe §2.3), aber sinnvolle Beigabe.

**Was als MCP-Server Sinn ergibt:**

- `claudeplex.list_accounts` → strukturierte Liste für den Agent
- `claudeplex.get_usage(account)` → Detail-Heat
- `claudeplex.list_sessions(filter)` → alles über alle Hosts
- `claudeplex.launch_agent(account, cwd, prompt)` → Agent startet (Block öffnet im UI)
- `claudeplex.adopt_session(id)` → öffnet als Block

Das macht claudeplex-Daten/Aktionen aus dem Chat heraus erreichbar — *zusätzlich* zur UI, nicht als Ersatz. Ein Slash-Command im Chat („starte einen neuen Agenten auf dem freisten Account") ruft das MCP-Tool auf, der Agent öffnet im UI als Block. Schöne Symmetrie.

**Implementation:** als eigener kleiner Rust-Binary (`claudeplex-mcp`), der wiederum auf `claudeplex_accounts`/`claudeplex_sessions` zugreift. Kein UI, nur stdio MCP server. Kommt nach v1.

---

## 8. Repo-Setup & Maintenance-Disziplin

### 8.1 Fork-Topologie

```
warpdotdev/warp (upstream)
  │
  └── zerx-lab/zap (Zap-Maintainer)
        │
        └── iret77/zaplex (unser Fork)  ← hier wird gebaut
```

Zwei-Stufen-Rebase. Beherrschbar **nur**, wenn unsere Änderungen 95%+ in eigenen `claudeplex_*`-Crates leben.

### 8.2 Branch-Strategie

- `main` — tracked `zerx-lab/zap:main` (regelmäßiger Rebase, alle 1-2 Wochen)
- `claudeplex` — unser Feature-Branch, der über `main` rebased wird
- Releases / Builds: vom `claudeplex`-Branch

### 8.3 Touchpoint-Disziplin

**Erlaubt** in fremden Crates:
- `warpui` / `warp_terminal`: Panel-Registrierung, Hotkey-Binding, ein Import-Block für unsere UI-Komponenten
- `settings`: Schema-Erweiterung für claudeplex-Settings (Account-Defaults, Hotkeys)

**Verboten** (würde Rebase-Hölle erzeugen):
- Logik in `warp_ssh_manager` ändern (auch wenn's verlockend ist) — stattdessen wrappen
- UI-Komponenten in `warpui_core` modifizieren — stattdessen eigene in `claudeplex_ui`
- Schema-Änderungen in `settings`, die existierende Settings beeinflussen

**Faustregel:** Wenn ein Patch in einem `warp_*`-Crate >20 Zeilen wird, ist es vermutlich falsch verortet — lieber ein neues Hook-Pattern im eigenen Crate vorschlagen.

### 8.4 Upstream-Sync-Disziplin

- Rebase-Termin im Kalender: alle 2 Wochen
- Vor jedem Rebase: `cargo test` muss grün sein
- Nach jedem Rebase: visueller Smoke-Test (Account Dock öffnet, Block startet, Prompt geht durch)
- Bei Konflikten in fremden Crates: lieber den eigenen Code anpassen als den upstream-Patch verformen

---

## 9. Erster Tag — konkrete Schritte für die neue Session

Die neue Session soll dieses Dokument lesen und dann **in dieser Reihenfolge**:

1. **Fork existiert bereits:** [iret77/zaplex](https://github.com/iret77/zaplex) (Fork von `zerx-lab/zap`)
2. **Lokal klonen** nach `~/projects/zaplex/iret77/zaplex/` (folgt der host-lokalen Projekt-Ordner-Struktur: `~/projects/<projekt>/<gh-org>/<repo>/`)
3. **Build-Voraussetzungen** klären: Rust toolchain, Zaps Build-Doku unter `docs/` und `CONTRIBUTING.md` im Zap-Repo lesen
4. **Lokalen Build** durchführen, App starten — sicherstellen, dass die Basis funktioniert
5. **`warp_ssh_manager` lesen** — das ist die Blaupause. Ziel: verstehen, wie ein Sidebar-Panel-Crate in Zap aussieht (Datei-Layout, Cargo.toml-Deps, Einhängung in `warpui`)
6. **Diesem Konzept folgen** für Architektur und UX
7. **Erste Crate anlegen**: `claudeplex_accounts`, sehr klein zum Start — nur Discovery (welche `CLAUDE_CONFIG_DIR`s gibt es). Ohne UI. Ohne Action-Layer. Pure Library mit einem Test.
8. **Erst dann** UI dazubauen: Account Dock als simpelster Sidebar-Eintrag mit Account-Liste, ohne Heat-Bars. Visuell verifizieren, dass es sich nativ anfühlt.

**Nicht im ersten Tag:**
- Nicht versuchen, alles auf einmal zu portieren
- Nicht versuchen, das Bun-Backend zu ersetzen
- Nicht in `warp_terminal` schnipseln, bevor klar ist, wie Zap Panels registriert
- Nicht „die ganze claudeplex-Logik" nach Rust kopieren

---

## 10. Referenzen

### 10.1 Bestehender Code (claudeplex)

- `/home/dev/projects/claudeplex/` — Bun-TUI, getestet
- `src/discover.ts` — Account-Discovery (`CLAUDE_CONFIG_DIR`-Enumeration)
- `src/collect.ts` — Session-Inventory, Usage-Parsing, PSS-Observer
- `src/usage.ts` — 5h-/Wochen-Fenster, Reset-Logik
- `src/agent.ts` / `src/agents.ts` — Spawn-Layer für `claude`-Subprocess (Vorlage für Action-Layer)
- `src/remote.ts` — Fleet-Supervisor mit RAM-Governor
- `src/hosts.ts` — Host-Discovery (`~/.ssh/config` + Tailscale)
- `src/pr.ts` / `src/issue.ts` — PR-Review + Quick-Issue (headless `claude -p`)
- `src/index.ts` — `--json`-Output, Format: `{accounts, remote}`

### 10.2 Zap

- Repo: [zerx-lab/zap](https://github.com/zerx-lab/zap)
- License: AGPL-3.0 (Client), MIT (`warpui`, `warpui_core`)
- Default branch: `main`
- Blaupause-Crate: `crates/warp_ssh_manager/`
- UI-Crates: `crates/warpui/`, `crates/warpui_core/`, `crates/ui_components/`
- Terminal: `crates/warp_terminal/`
- Settings: `crates/settings/`
- Doku: `docs/migrate-from-warp.md`, `docs/roadmap.md`
- Discussions: [warpdotdev/warp Discussion #9240](https://github.com/warpdotdev/warp/discussions/9240) (Open-Source-Ankündigung)

### 10.3 Verworfene Alternativen

Für Kontext, damit die neue Session nicht in dieselbe Diskussion zurückfällt:

- **MCP-only-Ansatz:** verworfen (siehe §2.3 — fehlende visuelle Permanenz)
- **Marcels Electron-App** (`byte5ai/claudeplex-desktop`): nicht weiterverfolgt (Electron baut Terminal-Unterbau redundant nach; Zap liefert ihn fertig)
- **Standalone claudeplex weiterführen:** ja, parallel, als Bun-CLI für Headless-/Server-Nutzung. Aber das Cockpit-UI lebt zukünftig im Zap-Fork.
- **Warp (upstream) forken statt Zap:** Zap gewinnt wegen Local-first + bereits verdrahteter CLI-Agent-Integration + Maintainer-Zugänglichkeit.

### 10.4 Vorarbeit-Memory (lokal beim Maintainer)

Frühere Session-Erkenntnisse liegen als lokale Memory-Snapshots beim Maintainer (claudeplex-Conductor-Reframe, Fleet-Design, Theme-Architektur, Electron-Entscheidung). Sie sind nicht öffentlich, aber alle relevanten Konzepte sind in diesem Dokument konsolidiert — eine neue Session braucht sie nicht zu lesen, um loszulegen.

---

## 11. Anti-Patterns — was die neue Session NICHT tun soll

Damit nichts in die falsche Richtung kippt:

1. **Kein eigenes Theme-System.** Nicht Lumen, nicht Truecolor-Gradients, nicht „aber claudeplex hatte das so schön". Zaps Theme. Punkt.
2. **Keine eigene Sidebar-Komponente von Null bauen.** Erst angucken, wie Zap Sidebars macht, dann das Pattern erweitern.
3. **Kein FFI/Memory-Sharing zwischen Bun und Rust.** Subprocess + NDJSON, sauber.
4. **Keine „weil claudeplex es so machte"-Argumente.** claudeplex-Konventionen sind reines Vorbild für die Datenseite. UI-Konventionen kommen von Zap.
5. **Keine TODO/FIXME für „MC-Pane macht v2".** Wenn etwas nicht im Scope ist, NICHT andeuten. Sauberer Code statt vorsichtshalber-Hook.
6. **Kein „schnell mal" Subprocess-Call von der UI-Schicht aus.** Action-Layer ist Action-Layer, UI ist UI. Trennung wahren auch beim Start.
7. **Keine vorbeugende Generalisierung.** Wenn es nur einen Account-Typ gibt (Claude Max), bauen wir nicht `enum AccountKind { Max, Pro, Team, Enterprise }`. Wir bauen `Account`. Erweitern, wenn es soweit ist.

---

## 12. Erfolgskriterien

Wie wir wissen, dass es geil geworden ist:

- Der User benutzt es täglich, das alte claudeplex-TUI nicht mehr.
- Marcels Electron-App ist obsolet (nicht aktiv gekillt — sie wird einfach nicht mehr gestartet).
- Ein Außenstehender, dem man einen Screenshot zeigt, fragt „seit wann hat Zap Multi-Account?" — nicht „was hast du da für eine Erweiterung?".
- Beim Multi-Tasking über 3 Accounts hat man jederzeit den Heat-Status im peripheren Blickfeld, ohne hinzuschauen.
- Eine wartende Session ist nie länger als 5 Sekunden unbemerkt.

Wenn diese fünf Punkte nach v1 stehen, hat sich der Fork gelohnt.
