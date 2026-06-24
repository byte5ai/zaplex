# Zaplex — Konzept für den Zap-Fork

> **Status:** Konzept, festgelegt 2026-06-22 mit dem User. Umsetzung in eigener Session ab 2026-06-23 im Fork-Repo [iret77/zaplex](https://github.com/iret77/zaplex).
>
> **Erweitert 2026-06-23:** Multi-Provider. Neben **Claude** wird **Codex** als gleichwertiger Agent-Provider unterstützt — beide primär über **Subscription-Auth** (Claude Max bzw. ChatGPT-Subscription). **Subscription-Support ist Must-Have.** Zaps bestehende API-Key-/BYOP-Pfade bleiben verfügbar, wo schon vorhanden — sie werden nicht entfernt, sind aber nicht der Fokus. Die Account-/Usage-/Routing-Schicht ist provider-symmetrisch und subscription-zentriert.
>
> **Was hier steht:** Mission, Designprinzipien, Architektur, UX, erste Schritte. Genug, dass eine kalt gestartete Claude-Code-Session loslegen kann, ohne die Diskussionshistorie zu kennen.
>
> **Was hier NICHT steht:** Implementierungs-Details der Rust-Crates. Die entstehen in der Session selbst — geleitet von den Prinzipien unten, nicht prescripted.
>
> **Namensregel (verbindlich):** Das Projekt heißt **einzig und allein `zaplex`**. Kein Artefakt dieses Projekts (Crate, Binary, MCP-Namespace, Settings-Key, UI-Komponente) darf „claudeplex" heißen — auch aus markenrechtlichen Gründen. Der Name „claudeplex" bezeichnet ausschließlich das **bestehende Bun-Referenz-Tool/-Repo**, aus dem wir die Datenschicht portieren; auf dieses verweisen wir bei Bedarf mit seinem realen Namen. Alles, was wir selbst bauen, trägt den Prefix `zaplex_*`.

---

## 1. Mission

Aus dem bestehenden Bun-TUI (claudeplex) wird **Zaplex** — ein nativer Teil von **Zap** (open-source Warp-Fork von [zerx-lab/zap](https://github.com/zerx-lab/zap)). Ergebnis: ein einziges Tool, das

- die **MC-Hälfte** liefert (Host-Zugriff, Dual-Pane-Files, Cross-Host-Copy) — kommt großteils von Zap selbst,
- die **Agent-Hälfte** liefert (Multi-Account-Orchestrierung über **mehrere Provider — Claude und Codex —**, Fleet, Conductor) — die fehlende Schicht, die wir beitragen,
- und **Session-Bedienung + Prompting** als first-class-Erlebnis vereint — der eigentliche Wow-Moment.

**Der Nordstern:** ein **integriertes Premium-Terminal** für anspruchsvolle Devs, die mit **Claude Code** und **Codex** auf einem oder mehreren Remote-Hosts entwickeln — oft mit **vielen Sessions/Agents parallel**. Es löst drei Versprechen ein:
1. **Kein Tool-Wechsel** — Sessions, Files, Accounts, Kosten an einem Ort; die CLIs bleiben voll direkt nutzbar, zaplex gibt den Überblick *obendrauf* (was braucht wo Aufmerksamkeit? Tokenverbrauch/Kosten? Subscription-Auslastung?).
2. **Nichts bricht ab** — Verbindungsabbruch, leerer Akku oder das Ausschalten des lokalen Rechners unterbrechen die Agenten **nicht** (nativer Session-Daemon, §3.5).
3. **Mentale Last runter** — das große Problem der „Vibecoder": sofort sichtbar, *was wo* gerade Eingreifen braucht; **Multi-Subscription-Balancing über Claude und Codex** gegen Rate-Limits; **kein Overwhelming** selbst bei vielen Projekten/Sessions/Agents.

Es ersetzt das standalone claudeplex-TUI und die Electron-App `iret77/claudeplex-desktop` (Fork der Team-App von Marcel) — beide haben bisher einen Terminal-/SSH-Unterbau nachgebaut, den Zap fertig mitbringt. Diese Redundanz schneiden wir weg. Beide bleiben **Referenzquellen** (`~/projects/zaplex/claudeplex`, `~/projects/zaplex/claudeplex-desktop`): alles, was sinnvoll passt, übernehmen wir.

**Provider-Gleichwertigkeit:** Claude (Claude Max) und Codex (ChatGPT-Subscription) sind keine Sonderfälle voneinander, sondern zwei Instanzen derselben Abstraktion. Discovery, Budget-/Heat-Tracking, „launch on freest" und das Session-Inventar funktionieren für beide identisch. Wo die zugrundeliegende CLI eine Fähigkeit nicht bietet (siehe Capability-Matrix §3.4), degradiert das Feature ehrlich — wir täuschen keine Parität vor, die das CLI nicht hergibt.

**Subscription zuerst (Must-Have):** Zaplex' Orchestrierung ist um **Subscription-Accounts** herum gebaut — genau deren rollende Rate-Fenster (5h + Woche) machen Heat-Tracking und „launch on freest" überhaupt sinnvoll. Subscription-Support ist nicht verhandelbar. API-Key-/BYOP-Nutzung wird pro Token abgerechnet und hat diese Fenster-Semantik nicht; sie ist deshalb nicht der Dreh- und Angelpunkt — aber Zap bringt sie bereits mit, und wir reißen sie **nicht** heraus. API-Key-Accounts dürfen koexistieren (z. B. im Account-Dock sichtbar, nur ohne Heat-Fenster); sie zu unterstützen kostet uns nichts, weil der Pfad schon da ist. Fokus von Discovery, Heat und Routing bleibt die Subscription-Seite.

**Zielgruppe:** **anspruchsvolle Devs, die mit Claude Code und Codex auf Remote-Hosts entwickeln oder vibecoden** — **nicht** auf den User oder sein Team beschränkt (auch wenn wir das Tool zunächst für uns selbst bauen). Das heißt: kompromisslos auf Erlebnis und Politur statt auf vorzeitige Breite optimiert — jedes Feature muss den Workflow spürbar verbessern, sonst fliegt es raus. **Erfolgskriterium:** schon die bisherigen claudeplex-/Desktop-User wechseln **freiwillig und gern** (vermissen nichts, gewinnen viel dazu) — und darüber hinaus jede:r anspruchsvolle Remote-Dev mit Claude/Codex (siehe §12).

---

## 2. Designprinzipien — nicht verhandelbar

### 2.1 Es muss sich nativ anfühlen

Dies ist die Kern-Akzeptanzregel. Wenn ein Reviewer einen Screenshot sieht und es als „angepappten Sidebar" erkennt, haben wir verloren.

**Konkret heißt das:**

- **Zaps visuelle Sprache vollständig erben.** Keine eigene Farbpalette, keine eigenen Border-Stile, keine eigene Typographie, kein eigenes Spacing. Die `theme.ts`/`ui.ts` des Bun-claudeplex waren Eigenleistung für ein Standalone-Tool — im Zap-Fork sind sie **Anti-Pattern**. Wir nutzen Zaps Theme-System, sonst nichts. Die Lumen-Themes werden NICHT mit übernommen.
- **Zaps Interaktionsmuster erben.** Cmd-K-Command-Palette, Block-System, Slash-Befehle, Block-Header-Konventionen, Notification-Center — alles wie es Zap macht. Wenn Zap einen Hotkey für „nächste Notification" hat, nutzen wir den auch für „nächste wartende Session". Wir erfinden keine parallelen Konventionen.
- **Zaps Icon-Family erben.** Falls Zap eine Icon-Sammlung hat (Lucide/eigene), übernehmen. Keine eigenen Unicode-Glyphen mehr. **Provider-Icons:** Zap verdrahtet Claude Code und Codex bereits als CLI-Agenten und hat dafür mit hoher Wahrscheinlichkeit schon Agent-Icons — die übernehmen wir für die Provider-Kennzeichnung (Claude vs Codex), statt eigene zu malen.
- **Sidebar-Patterns kopieren von `warp_ssh_manager`.** Wenn der SSH-Manager-Panel Akkordeons nutzt, wir auch. Wenn er einen bestimmten Selektions-Stil hat, wir auch.
- **Native Look ist wichtiger als Feature-Vollständigkeit.** Lieber 80% der Features mit perfektem Nativ-Gefühl als 100% mit Stilbrüchen.

### 2.2 Saubere Session-Bedienung & Prompting

Der explizit hervorgehobene Wunsch. Was „sauber und nice" konkret bedeutet:

- **Eine Session = ein Zap-Block.** Nicht ein eigenständiges Panel, nicht ein Tab, ein **Block** — wie Zaps existierende CLI-Agent-Integration (Claude Code, Codex, agy) sie verwendet. Block-Output ist Block-Output, Block-Eingabe ist Block-Eingabe. Der Block ist provider-agnostisch: ein Codex-Block bedient sich exakt wie ein Claude-Block.
- **Inline-Prompting im Block.** Eingabezeile direkt am Block-Ende. ⏎ schickt in die laufende Session. Slash-Commands gehen durch. Clipboard-Bilder per Cmd-V. Genauso wie wenn man `claude` oder `codex` direkt im Terminal startet — nur mit Multiplexer drumherum.
- **Mehrere Sessions parallel sichtbar.** Split-Pane / Tile-Layout, nicht „eine zur Zeit". Das ist das, was claudeplex' Cockpit-Mode konzeptionell wollte, aber als TUI nie wirklich geliefert hat. In Zap mit echten Blocks ist es selbstverständlich. Claude- und Codex-Sessions liegen gemischt nebeneinander.
- **Cross-Session-Navigation per Tastatur.** Cmd-1..9 zu den ersten N Sessions, Cmd-Tab zur nächsten aktiven, ein dedizierter Hotkey („go to next waiting"). Maus optional, nie verpflichtend. Provider-unabhängig.
- **Adopt-by-session-id muss sich anfühlen wie „die Session war schon hier".** Wartende Session in Sidebar → Enter → öffnet als Block, history visible, ready to prompt. Kein „adoption ritual", kein Modal, kein „connect"-Schritt. Gilt für beide Provider.
- **Provider- & Account-Awareness im Block-Header.** Jeder Block zeigt, **welcher Provider** (Claude/Codex) und **gegen welchen Account** er läuft (kleiner Indicator, Zaps Stil, nicht aufdringlich). Beim Launch eines neuen Agenten wird Provider (per Wizard-Wahl) und Account (freester des gewählten Providers) vorausgewählt — im Block-Header umstellbar, falls man bewusst einen anderen Account will.

### 2.3 Keine Krücken, keine Spar-Implementierungen

- Wenn etwas „eigentlich richtig" eine Woche braucht und „pragmatisch" einen Nachmittag, nehmen wir die Woche.
- Wenn ein Feature nur als visueller Glance Sinn ergibt, bauen wir es als visueller Glance — nicht als Tool-Aufruf, nicht als Slash-Command, nicht als MCP-Wrapper.
- Wenn die Wahl zwischen „eigene Konvention" und „Zaps Konvention erweitern" steht: Zaps Konvention erweitern, auch wenn es mehr Arbeit ist.
- **Keine vorgetäuschte Provider-Symmetrie.** Wo Codex und Claude dieselbe Fähigkeit haben, bauen wir sie symmetrisch. Wo eine CLI eine Fähigkeit nicht hat (z. B. ein remote-control-Serverprotokoll), bauen wir keine Fake-Schicht, sondern degradieren sichtbar und ehrlich (Capability-Matrix §3.4).
- MCP ist ergänzende Beigabe (siehe §7), niemals Ersatz für UI.

---

## 3. Ist-Zustand (Referenz)

### 3.1 Was Zap fertig liefert

Quelle: [zerx-lab/zap](https://github.com/zerx-lab/zap), Stand 2026-06-21.

- **Terminal-Engine** (GPU-rendered, Block-basiert, Historie)
- **`warp_ssh_manager`** — SSH-Hosts, tmux-Integration, Sessions
- **`warp_files`** — Terminal-File-Handling (Drag-Drop, Inline-Preview, File-URLs)
- **AI-Provider-Routing** (BYOP) — Anthropic, OpenAI, Gemini, DeepSeek, Ollama nativ + beliebige OpenAI-kompatible Endpoints. *(Zaps API-Key-Pfad — bleibt bestehen und nutzbar; unser Fokus liegt auf der Subscription-Orchestrierung, siehe §1. Beides schließt sich nicht aus.)*
- **CLI-Agent-Adapter** — **Claude Code, Codex, agy bereits als Blocks verdrahtet**, OSC9/777-Routing in Notification-Center. *(Wichtig: Beide von uns orchestrierten Provider sind als spawn-bare Blocks bereits vorhanden — wir bauen die Account-/Routing-Schicht darüber, nicht den Block-Unterbau.)*
- **MCP-Client**
- **`settings`/`warpui`/`warpui_core`** — UI-Framework (UI-Crates sind MIT-lizenziert, der Rest AGPL-3.0)
- **61 Crates insgesamt**, klare Trennung
- **Aktiv** (täglich Commits, 1.8k ⭐, ~34 offene Issues)

### 3.2 Was Zaplex einbringt (Lücken-Liste)

Die Schicht, die Zap aus konzeptionellem Grund weglässt (Roadmap: „single account/identity shared across surfaces"). Alles **provider-übergreifend** (Claude + Codex):

- **Multi-Account-Discovery** — mehrere Subscription-Logins parallel, pro Provider:
  - Claude: mehrere `CLAUDE_CONFIG_DIR`s (Claude-Max-Logins)
  - Codex: mehrere `CODEX_HOME`s (ChatGPT-Subscription-Logins)
- **Per-Account-Budget-Tracking** — 5h- und Wochen-Fenster, Heat-Coloring, Reset-Countdown, Kosten. Pro Provider eigener Usage-Parser, einheitliches Heat-Output-Format.
- **Account-Routing** — „launch on freest" als Default beim Agent-Start, **innerhalb des gewählten Providers**.
- **Cross-Account-Session-Inventar** — alle laufenden/wartenden/recent Sessions über alle Accounts/Provider/Hosts.
- **„Needs me"-Bubbling** — „● N waiting" + Hotkey-Jump (provider-gemischt).
- **Persistente Remote-Fleet** — Agent-Server laufen als Sessions im **nativen zaplex-Session-Daemon** (§3.5; überlebt Lid-Close **und** App-Restart, **kein** tmux). *Provider-abhängig:* für Claude zusätzlich `claude remote-control` (bedient auch die Claude-Mobile-App); für Codex liefert der Daemon die generische Persistenz (kein eigenes Serverprotokoll — siehe §3.4). **Eine** gemeinsame Persistenz-Primitive für interaktive Shells und Fleet, nicht zwei Mechanismen.
- **RAM-Governor für die Fleet** — harter Ceiling, pro Session (~330 MB für Claude; Codex-Footprint bei Umsetzung messen, eigener Wert).
- **Adopt-by-session-id** — Session, die in einer anderen Shell gestartet wurde, hier weiterführen (beide Provider).
- **MC-style Dual-Pane File Manager** — Host↔Host-Copy ohne scp, weil `warp_files` Single-Pane ist. *(Provider-unabhängig — gehört zur MC-Hälfte.)*
- **Multiplexer-kompatibles Zapify** (zaplex' Shell-Integration, Pendant zu Warps „Warpify") — den Mehrwert (Blocks, Prompt-Status, Command-Status, Completions) auch *innerhalb* eines vom User betriebenen `tmux`/`screen`/`byobu` liefern, statt ihn dort abzuschalten. *(Provider-unabhängig — Terminal-/MC-Hälfte; Details §3.5.)*
- **Native Session-Resilienz + mosh** — interaktive Remote-Sessions überleben Verbindungsabbrüche (Deckel zu, Akku leer, Netz-/Tailscale-Roaming), nahtloses Re-Attach mit erhaltener Historie, plus mosh-Eigenschaften (UDP-Roaming, predictive local echo). Macht externes `byobu` + `mosh` überflüssig. Pro Verbindung in den Settings schaltbar. *(Provider-unabhängig; Details §3.5.)*

### 3.3 Datenschicht — was getestet ist, was greenfield ist

- **Claude-Seite:** claudeplex' (Bun) `--json`-Endpoint liefert `{accounts, remote}` mit Discovery, Usage und Fleet-Status. Diese Logik ist getestet, performant, läuft auf Linux + macOS:

  ```bash
  claudeplex --json
  # → {accounts: [...], remote: {...}}   (nur Claude-Accounts)
  ```

  Wir bauen sie **NICHT** auf Anhieb nach Rust um (siehe §6.1) — in v0 spawnen wir das Bun-Binary als Subprocess.

- **Codex-Seite:** Es gibt **keine** getestete Bun-Vorlage. Codex-Discovery/Usage ist greenfield und wird **von Anfang an nativ in Rust** in `zaplex_accounts` gebaut. Das ist kein Widerspruch zur Hybrid-Strategie — es gibt schlicht keinen Bun-Code zum Wiederverwenden. Codex ist damit der erste vollständig native Provider; die Claude-Seite zieht in v1 nach (§6.2).

  **Codex-Subscription-Auth (Design-Annahme, bei Umsetzung verifizieren):** `codex login` → „Sign in with ChatGPT" (OAuth im Browser) → Token in `$CODEX_HOME/auth.json` (Default `~/.codex`). Mehrere Subscription-Logins = mehrere `CODEX_HOME`-Verzeichnisse, exakt analog zum `CLAUDE_CONFIG_DIR`-Trick. Rate-Limit-/Restkontingent-Daten (5h-/Wochen-Fenster) surfacet die Codex-CLI; die genaue lokale Quelle (gecachter Rate-Limit-State im `CODEX_HOME` bzw. Response-Header) ist beim Bau zu lokalisieren. **Kein API-Key** — `auth.json` hält den Subscription-Token.

### 3.4 Capability-Matrix (Provider-Parität, ehrlich)

| Fähigkeit | Claude (Claude Max) | Codex (ChatGPT-Sub) |
|---|---|---|
| Subscription-Login, mehrere Accounts | ✅ `CLAUDE_CONFIG_DIR`-Set | ✅ `CODEX_HOME`-Set |
| Discovery | ✅ (Bun-`--json`, v0) | ✅ (nativ Rust ab v0) |
| Usage/Heat (5h + Woche) | ✅ | ✅ (Datenquelle bei Bau verifizieren) |
| „launch on freest" | ✅ | ✅ |
| Als Zap-Block spawnbar | ✅ (Zap fertig) | ✅ (Zap fertig) |
| Adopt / Resume by id | ✅ `claude --resume <id>` | ✅ Codex-Resume (Flag bei Bau verifizieren) |
| Steer (stdin → Block) | ✅ | ✅ |
| Session-Persistenz (Remote) | ✅ nativer zaplex-Daemon (§3.5) | ✅ nativer zaplex-Daemon (§3.5) |
| Persistenter remote-control-Server (Mobile-App) | ✅ `claude remote-control` | ❌ kein Serverprotokoll → keine Codex-Mobile-App |
| RAM-Governor | ✅ (~330 MB) | ✅ (Footprint messen) |

Die rot markierte Zelle ist der einzige bewusste Asymmetrie-Punkt: Codex hat kein dem `claude remote-control` äquivalentes Serverprotokoll und damit **keine Mobile-App-Anbindung**. Die **Session-Persistenz** ist hingegen für beide Provider gleich — sie kommt vom **nativen zaplex-Daemon** (§3.5), nicht vom CLI: Codex-Sessions laufen also persistent und adopt-bar, nur ohne Mobile-App. Das wird in der UI nicht verschleiert.

### 3.5 Sicheres Remote-Entwickeln — Multiplexer-kompatible Shell-Integration + Session-Resilienz

> **Motivation:** Der User entwickelt täglich auf Remote-Hosts (devhost via Tailscale, macmini) und stützt sich heute auf **externes** `byobu` (Persistenz) + `mosh` (Roaming) *um* das Terminal herum. Ziel: zaplex vereint alles Notwendige zum sicheren Remote-Entwickeln **im integrierten Terminal**. Dies betrifft die interaktive User-Shell — verwandt mit, aber nicht identisch zur Agent-Fleet-Persistenz (§3.2/§4.4); beide sollten auf **einer** Resilienz-Primitive sitzen, nicht doppelt gebaut werden.

**Drei Lücken im heutigen Warp/Zap-Verhalten** (1–2 am Code verifiziert, 3 als Beobachtung):

1. **Warpify ⟂ Multiplexer.** Wer auf dem Host bereits in `tmux`/`screen`/`byobu` sitzt, verliert die Warpify-Features. Zaps SSH-Warpify startet einen **eigenen, privaten** `tmux -Lwarp -CC` (Control-Mode, eigener Socket — `app/assets/bundled/ssh/bash_zsh/warpify_ssh_session.sh`, `app/src/terminal/model/ansi/mod.rs`), der nicht mit dem Multiplexer des Users komponiert; der Ausfall ist sogar als Hook `RemoteWarpificationIsUnavailable` kodiert (`app/src/terminal/model/ansi/dcs_hooks.rs`).
2. **Keine Persistenz interaktiver Sessions.** SSH läuft über das native `ssh`-Binary + ControlMaster (`crates/warp_ssh_manager/`, `app/src/remote_server/ssh_transport.rs`). Reconnect ist nur für transiente Blips ausgelegt (`MAX_RECONNECT_ATTEMPTS = 2`, `RECONNECT_DELAY = 2s` — `crates/remote_server/src/manager.rs`); Deckel-zu/Akku-leer überschreiten das sofort. tmux dient im Code **nur** dem Warpify-Bootstrap, **nicht** der Persistenz (Feature-Flag `SSHTmuxWrapper`).
3. **Maus-Bedienung bricht im Multiplexer weg.** Beim Zugriff auf einen `byobu`-Host reagiert in Warp die Maus nicht mehr (User-Beobachtung). Ursache offen — entweder die deaktivierte Warpify-Integration oder fehlendes Mouse-Mode-Passthrough (Terminal sendet SGR-Mouse-Reporting nur, wenn der Multiplexer `mouse on` hat und die Mouse-Mode-Sequenzen durchreicht). Der Track-A-Spike klärt die Ursache. **Unabhängig davon gilt für zaplex als harte Anforderung: Maus-Bedienung (Klick, Selektion, Scroll) muss in jeder Remote-/Multiplexer-Session gewährleistet sein — auch wenn Zapify deaktiviert oder nicht verfügbar ist.** „Ehrlich degradieren" (§2.3) heißt hier: Komfort-Features (Blocks/Hooks) dürfen fehlen, die Maus nie.

**Architektur-Entscheidung (festgelegt 2026-06-24): Option 1 — nativer Persistenz-Layer.**
zaplex **besitzt** einen eigenen nativen, persistenten Remote-Session-Layer und komponiert **nicht** mit vom User eingerichteten Multiplexern (tmux/byobu/screen) — gleiche Grundhaltung wie Warp, aber mit *echter* Persistenz (Warps remote-server überlebt den SSH-Drop, hat aber keine persistente Session-ID über App-Restart). Damit ist **Track B das Rückgrat** (Must-Have); **Track A** („Zapify im User-Multiplexer") entfällt als Primärpfad und kommt höchstens *optional/später*, eng gescoped auf plain `tmux` — **nicht** byobu/screen.

*Begründung aus dem Track-A-Upstream-Spike (2026-06-24):* Upstream gibt es nichts zu übernehmen — zap-Issue [#132](https://github.com/zerx-lab/zap/issues/132) ist ein stale Research-Issue ohne Code, Warps tmux-Warpify-Pfad ist offiziell **deprecated** (zugunsten der remote-server-Binary), der Maus-in-tmux-Bug [warpdotdev/Warp #5541](https://github.com/warpdotdev/Warp/issues/5541) ist „not planned" geschlossen. Die Deprecation ist eine **Produkt-/Wartungs-Entscheidung, keine technische Unmöglichkeit** — iTerm2s `tmux -CC`-Control-Mode-Integration beweist die Lösbarkeit seit ~2011. Compose mit fremden Configs (byobu-UI, Oh-My-Tmux ist von Warp als inkompatibel gelistet, `screen` hat gar keinen Control-Mode) wäre eine dauerhafte Wartungssteuer und kämpft gegen die Architektur — deshalb besitzt zaplex den Layer selbst. Die **Maus-Garantie** (Lücke 3) wird über diesen besessenen Layer erfüllt: zaplex kontrolliert die Remote-Session-Struktur end-to-end, statt SGR-Mouse durch einen fremden Multiplexer durchzureichen (der Warp-Maus-Bug rührt von der tmux-Schachtelung, nicht vom Warpify-Status).

**Stoßrichtungen (unter dieser Entscheidung):**

- **Track A — *(optional / später, kein Primärpfad)* Zapify im User-Multiplexer.** **Begriff (festgelegt):** Zaps Shell-Integration heißt upstream „Warpify"; unsere namensregel-konforme Variante heißt **Zapify** (`zaplex_*`). Wo wir das Verhalten neu bauen/erweitern, sprechen wir von Zapify; „Warpify" bleibt nur als Name des bestehenden Zap/Warp-Mechanismus und in vorhandenen Code-Pfaden (`app/src/terminal/warpify/…`) stehen. — Inhaltlich: den DCS/OSC-Hook-Strom (OSC `9277`–`9280`, `dcs_hooks.rs`) durch einen **vom User betriebenen** Multiplexer transportieren, statt einen eigenen `-Lwarp`-tmux danebenzustellen. Offene Punkte: (a) ~~Upstream-Stand prüfen~~ — **vom Spike beantwortet:** nichts upstream zu übernehmen, also selbst bauen (falls überhaupt verfolgt). (b) Passthrough-Framing (`tmux allow-passthrough`, `screen` restriktiver) — passieren die Hook-Sequenzen ungeschädigt? (c) Pro-Pane-Zuordnung der Blocks (Pane-ID als Hook-Dimension). (d) Modus „bestehende `$TMUX`/`STY`/byobu-Umgebung erkennen und darin bootstrappen". (e) Maus-Mode-Passthrough — warum bricht die Maus im `byobu`-Host weg, und wie bleibt SGR-Mouse-Reporting (`1006`/`1002`/`1003`) durch den Multiplexer erhalten (Maus ist harte Anforderung, siehe Lücke 3).
- **Track B — *(Rückgrat / Primärpfad)* Native Session-Resilienz + mosh.** Eingebauter Mechanismus: mehrere Sessions pro Host, Überleben von Verbindungsabbrüchen mit nahtlosem Re-Attach, plus mosh-Eigenschaften (UDP-Transport mit State-Sync, predictive local echo, Roaming über IP-Wechsel). Umsetzungsoptionen, aufsteigend:
  - **B1: `mosh` orchestrieren** — zaplex ruft `mosh`/`mosh-server` am Host auf (setzt mosh am Host voraus; mosh allein persistiert nicht über Server-Restart). Schneller Latenz-/Roaming-Gewinn.
  - **B2: Eigener zaplex-Session-Daemon** — baut auf dem vorhandenen `remote-server`-Daemon auf (überlebt SSH-Drop bereits, `crates/remote_server/`), erweitert um persistente Session-IDs über App-Restart, Output-Replay-Buffer und Re-Attach-Protokoll (erweitert die `Connecting→Initializing→Connected→Reconnecting`-Zustandsmaschine).
  - **B3: B2 + nativer UDP-Transport (mosh-äquivalent)** — SSP-artiger State-Sync + predictive echo + Roaming nativ im zaplex-Transport. Kür; mosh' Sicherheitsmodell (AES-OCB, Schlüssel über initiale SSH-Verbindung) als Referenz.

  **Reihenfolge:** B2 zuerst (Persistenz ist der schmerzhafteste Mangel und das Must-Have, baut auf vorhandenem `remote-server`-Code auf) → B3 (nativer mosh-grade UDP-Transport, sauber integriert) → B1 (mosh-Orchestrierung nur, falls der eigene Transport später kommt). Track A ist **kein** Primärpfad mehr (siehe Architektur-Entscheidung) und kommt nur optional/später.

**Pro-Verbindung-Setting (beide Tracks).** Host-Profile liegen in `SshServerInfo` (`crates/warp_ssh_manager/src/types.rs`) ↔ Tabelle `ssh_servers` (`crates/persistence/src/model.rs`), CRUD in `repository.rs`. Additiv erweiterbar um z. B. `zapify_multiplexer: Off|UseExisting|Managed` (Track A) und `session_resilience: Off|PersistOnly|PersistPlusMosh` (Track B): Felder am Struct + Diesel-Migration + CRUD + SSH-Manager-Panel. Default konservativ (aus). **Ehrlich degradieren**, wo Host-Voraussetzungen fehlen (kein tmux/mosh, alte tmux-Version — vorhandene Hooks `TmuxNotInstalled`/`UnsupportedTmuxVersion`, Auto-Install-Ansatz `SshTmuxInstaller` wiederverwenden). Globaler Default bleibt über die Warpify-Settings steuerbar (`app/src/terminal/warpify/settings.rs`); der Per-Host-Toggle überschreibt ihn.

**Akzeptanz:** (A) zaplex liefert Blocks/Prompt/Completions in Remote-Sessions über den **eigenen** persistenten Layer — ohne dass der User `byobu`/`tmux` aufsetzen muss; ein optionaler Compose-Modus (plain `tmux`) wäre Kür, kein Akzeptanzkriterium. (B) Eine interaktive Session überlebt Deckel-zu/Akku-leer/Netzwechsel mit nahtlosem Re-Attach und erhaltener Historie; über Tailscale-Roaming bleibt das Tippgefühl latenzarm. (C) **Maus-Bedienung** (Klick, Selektion, Scroll) funktioniert in jeder Remote-/Multiplexer-Session zuverlässig — unabhängig vom Zapify-Status (harte Anforderung, nicht degradierbar). Ergebnis: kein externes `byobu` + `mosh` mehr nötig.

---

## 4. Architektur

### 4.1 Schichten-Modell

```
┌──────────────────────────────────────────────────────────────┐
│  UI Layer (Rust, in warpui / warp_terminal)                  │
│  - Account Dock (Sidebar-Panel, Provider-gruppiert)          │
│  - Agent Tree (unter dem Dock)                               │
│  - Launch Wizard (Modal, mit Provider-Auswahl)              │
│  - Block-Header-Extension (Provider- + Account-Indicator)    │
│  - Hotkey Registration (next-waiting, switch-session)        │
│  - MC Dual-Pane View (separater Modus)                       │
└──────────────────────────────────────────────────────────────┘
              ▲                                  ▲
              │                                  │
┌─────────────┴──────────────┐    ┌──────────────┴─────────────┐
│  Action Layer (Rust)       │    │  Data Layer                │
│  - Launch agent (Provider) │    │  Claude: Bun → Rust        │
│  - Adopt session           │    │    v0: spawnt `claudeplex  │
│  - Steer (send to block)   │    │        --json` und parst   │
│  - PR-review / quick-issue │    │    v1: native Rust ports   │
│  - Fleet control           │    │  Codex: nativ Rust ab v0   │
└────────────────────────────┘    │    (CODEX_HOME-Discovery,  │
              │                    │     Usage-Parser)          │
              ▼                    │  → vereinheitlicht hinter  │
   Zap's existing block / agent    │    Provider-Trait          │
   infrastructure                  └────────────────────────────┘
   (claude/codex werden gespawnt wie jeder andere CLI-Agent;
    nur mit dem richtigen CONFIG_DIR/CODEX_HOME und stdin-Pipe)
```

### 4.2 Crate-Layout

Eigene Crates mit klarem `zaplex_`-Prefix. Vorteil: bei jedem Rebase mit Zap-Upstream ist offensichtlich, was „unseres" ist.

| Crate                  | Inhalt                                                                         | Größe (geschätzt) |
|------------------------|--------------------------------------------------------------------------------|-------------------|
| `zaplex_accounts`      | Provider-Abstraktion, Discovery (Claude + Codex), Usage-Parser pro Provider, Heat-Logik, Routing | mittel–groß       |
| `zaplex_sessions`      | Live-Inventar, Waiting-Detection, Adoption, Send-to-block — provider-aware      | mittel            |
| `zaplex_fleet`         | Remote-control-Supervisor (Claude) + tmux-Session-Host (beide), RAM-Governor    | mittel            |
| `zaplex_mc`            | Dual-Pane-File-Manager (SFTP-aware, Host↔Host-Copy)                             | groß              |
| `zaplex_ui`            | UI-Komponenten (Account-Dock, Agent-Tree, Launch-Wizard mit Provider-Auswahl)   | mittel            |

**Provider-Abstraktion** lebt *innerhalb* von `zaplex_accounts`, nicht als eigenes Crate:

```rust
enum Provider { Claude, Codex }

// jeder Account trägt seinen Provider
struct Account { provider: Provider, label: String, config_dir: PathBuf, /* … */ }

// pro Provider eine Impl: Discovery + Usage
trait ProviderBackend {
    fn discover(&self) -> Vec<Account>;          // CLAUDE_CONFIG_DIR-Set bzw. CODEX_HOME-Set
    fn usage(&self, acct: &Account) -> Usage;    // 5h-/Wochen-Fenster → einheitliches Heat-Format
    fn launch_cmd(&self, acct: &Account) -> Command; // claude … bzw. codex … mit richtigem env
}
```

Das `Usage`/`Heat`-Output ist provider-agnostisch; die UI weiß nichts von der Provider-Verzweigung außer dem Badge.

**UI-Einhängung** passiert in **so wenig fremdem Code wie möglich**:
- `warpui` / `warp_terminal`: minimaler Patch, der unsere Panels registriert und Hotkeys bindet
- Alles andere lebt in unseren eigenen Crates

Diese Disziplin ist die Maintenance-Versicherung. Je weniger Zeilen wir in geerbten Crates ändern, desto weniger Rebase-Schmerz.

### 4.3 Datenfluss (v0 — hybrid Claude, nativ Codex)

```
Claude:                                  Codex:
claudeplex Binary (Bun, existiert)       (kein Bun — nativ Rust)
  │                                         │
  ▼                                         ▼
claudeplex --json --watch                CODEX_HOME-Discovery +
(NDJSON-Stream)                          Usage-Parser (in zaplex_accounts)
  │ stdout                                  │
  ▼                                         ▼
zaplex_accounts (Rust): ProviderBackend::Claude   ProviderBackend::Codex
  └──────────────────────┬───────────────────────┘
                         ▼
              Internal State (Rust structs, provider-getaggt)
                         │
                         ▼
              UI updates via Zap's reactive system
```

**Warum hybrid für Claude?** v0 muss in Wochen, nicht Monaten laufen. Die Bun-Logik ist getestet. Die Prozess-Grenze ist sauber (kein FFI-Tanz, kein Memory-Sharing-Gefrickel). Zap hat schon Subprocess-Infrastruktur. Das ist **kein Hack** — das ist eine bewusste Schichten-Grenze.

**Warum nativ für Codex?** Es gibt keinen Bun-Code zum Wiederverwenden. Eine eigene Bun-Implementierung nur um der Symmetrie willen zu bauen wäre eine Krücke. Codex wird direkt als Rust-`ProviderBackend` gebaut — und dient zugleich als Blaupause für den v1-Port der Claude-Seite.

**Wann v1 (Claude nativ Rust)?** Wenn v0 sich bewährt UND der Bun-Hop spürbare Latenz/Bugs verursacht. Vorher portieren wir nichts. Wir portieren auch nicht „on principle" — wir portieren, wenn es weh tut.

### 4.4 Action-Layer

Aktionen rufen entweder existierenden claudeplex-Code auf (über CLI, Claude-Seite) oder nutzen Zap-Mechanismen direkt. Der Launch verzweigt über `ProviderBackend::launch_cmd`:

| Aktion                  | v0 Implementierung — Claude                                       | v0 Implementierung — Codex                                  |
|-------------------------|-------------------------------------------------------------------|-------------------------------------------------------------|
| Launch agent            | `claude` als Subprocess, `CLAUDE_CONFIG_DIR=<acct>` → wird zu Block | `codex` als Subprocess, `CODEX_HOME=<acct>` → wird zu Block  |
| Adopt session           | `claude --resume <session-id>` mit richtigem `CONFIG_DIR`          | Codex-Resume by id (Flag bei Bau verifizieren)              |
| Steer (prompt senden)   | stdin des Block-Subprocesses, exakt wie Zap es heute schon macht   | identisch — stdin des Block-Subprocesses                    |
| PR-review               | `claudeplex` CLI als Subprocess (existierende headless `-p` Logik) | Codex headless (analog; bei Bedarf später)                  |
| Fleet control           | Bun-CLI als Subprocess (existierender `--json` Output)             | nativ über `zaplex_fleet`                                   |
| Remote-fleet-Server     | Agent-Session im nativen zaplex-Daemon (§3.5); zusätzl. `claude remote-control` für Mobile-App | Agent-Session im nativen zaplex-Daemon (§3.5)               |

**Wichtig:** Wir bauen keinen eigenen `send-to-pty`-Layer. Zap hat den schon. Wir hängen uns dran. Der Steer-Pfad ist für beide Provider identisch, weil beide ganz normale Block-Subprocesses sind.

**Persistenz-Substrat (verbindlich, Architektur-Entscheidung §3.5):** Die Remote-Fleet persistiert über den **nativen zaplex-Session-Daemon**, **nicht** über tmux. Aus claudeplex übernehmen wir das **Fleet-*Modell*** (Discovery, Reuse je Account×Projekt, RAM-Governor, „~N Sessions passen noch") — **nicht** den tmux-*Mechanismus*. Solange der Daemon (Plan `docs/superpowers/plans/2026-06-24-native-remote-session-layer.md`, Stufen B2→B3) noch nicht steht, laufen Remote-Agents **ohne** Persistenz-Garantie (ehrlich degradiert, §2.3) — wir ziehen **keine** tmux-Krücke ein.

---

## 5. UX-Design

### 5.1 Account Dock (Sidebar-Panel)

**Position:** linke Sidebar, oberster Bereich, oberhalb von Zaps existierender SSH-Host-Liste.

**Inhalt:** ein Eintrag pro entdeckter Account, **gruppiert oder gekennzeichnet nach Provider** (Claude / Codex). Pro Eintrag:
- **Provider-Indicator** — Zaps Agent-Icon für Claude bzw. Codex (klein, vorangestellt)
- Account-Label (aus dem Account-Setup übernommen)
- Mini-Heat-Bar für 5h-Fenster (winzig, eine Zeile, Zaps Progress-Bar-Stil)
- Mini-Heat-Bar für Wochenfenster
- Reset-Countdown bei Hover oder im expanded state
- Aktueller Status (idle / working / waiting) als Farb-Indicator, NICHT als Text-Pille

**Gruppierung:** Provider-Header (oder durchgehende Icon-Kennzeichnung — je nachdem, was sich in Zaps Sidebar nativer anfühlt; bei Umsetzung am `warp_ssh_manager`-Pattern entscheiden). Heat-Bars sehen für beide Provider gleich aus — die Fenster-Semantik (5h/Woche) ist äquivalent.

**Stil:** wie Zap seine SSH-Hosts darstellt. Wenn Zap dort eine bestimmte Border, ein bestimmtes Spacing, ein bestimmtes Hover-Verhalten hat — kopieren wir es exakt.

**Aktion:** Click auf einen Account-Eintrag → öffnet ein Submenü/Akkordeon mit den Sessions auf diesem Account.

### 5.2 Agent Tree (unter dem Dock)

**Position:** linke Sidebar, unterhalb des Account Docks.

**Hierarchie:** Host ▸ Projekt ▸ Session. Aktive Sessions oben, wartende unter eigenem Header, kürzliche/idle weiter unten. Jede Session-Zeile trägt ein kleines Provider-Icon — Claude- und Codex-Sessions stehen gemischt im selben Baum (nach Host/Projekt sortiert, nicht nach Provider getrennt).

**Status-Anzeige:** keine Glyph-Soup. `[WORK]` / `[WAIT]` / `[IDLE]` als textuelle Badges in Zaps Badge-Stil, oder reine Farb-Indicator-Punkte — je nachdem, was Zap als Pattern hat.

**Top-Indicator:** „● N waiting" als kleiner Counter im Tree-Header (provider-gemischt; nicht in der App-Topbar — das wäre außerhalb unseres Scopes).

### 5.3 Launch Wizard

**Trigger:** Hotkey (Vorschlag: `Cmd-Shift-N`, falls Zap das nicht bereits belegt; sonst was Vergleichbares).

**Form:** Modal im Zap-Modal-Stil. Vier Felder:
1. **Agent / Provider** — `Claude` | `Codex` (Segmented Control oder Dropdown). Default: zuletzt genutzt. Die Wahl filtert das Account-Feld und bestimmt das gespawnte Binary.
2. **Account** — vorausgewählt mit freestem Account **des gewählten Providers**, dropdown alphabetisch, nur Accounts dieses Providers.
3. **Folder** — Combobox, gespeist aus History (claudeplex' bestehender `discover.ts` liefert das für die Claude-Seite; Codex-Folder-History nativ).
4. **Initial prompt** (optional) — Textarea, ⏎ sendet auch direkt.

**Verhalten:** ⏎ launcht den Agenten des gewählten Providers in einem neuen Block, fokussiert den Block, scrollt zur Eingabezeile. Wechselt man oben den Provider, springt das Account-Feld auf den freesten Account des neuen Providers.

### 5.4 Block-Header-Extension

Jeder Session-Block bekommt im Header (oder unten als Status-Zeile, je nachdem wo Zap Header-Info platziert) Mikro-Indicators:

- **Provider-Badge** — Claude- bzw. Codex-Icon (Zaps Agent-Icon)
- **Account-Badge** — welcher Account läuft hier
- **Budget-Mikro-Heat** — eine winzige Bar oder Punkt, der die Account-Last reflektiert

Alles klein, nicht aufdringlich. Hover gibt mehr Details. Click auf den Account-Badge öffnet einen Account-Switcher (Accounts desselben Providers; einen anderen Provider wählt man durch einen neuen Launch, nicht durch In-Place-Umschalten — eine laufende Session bindet an ihren Provider).

### 5.5 MC Dual-Pane View

**Position:** ein eigener Mode/View. Zap kann Splits — wir nutzen einen Split, der explizit den MC-Modus aufmacht.

**Layout:** klassisch MC: linke Pane, rechte Pane, Funktionsleiste am unteren Rand (F1 Help, F5 Copy, F6 Move, F7 Mkdir, F8 Delete, F10 Quit) — oder Zaps Äquivalent davon. Wenn Zap eine Hotkey-Konvention hat, der sich daran halten lässt: gut. Wenn nicht: F-Keys, weil sie MC-User erwarten.

**Beide Panes können auf verschiedenen Hosts sein.** Das ist die Killer-Feature der MC-Hälfte — links macmini, rechts devhost, F5 copy → SFTP-Transfer. *(Provider-unabhängig.)*

**Verhalten zum Restsystem:** Wenn man in einem File-Block einen `.jsonl`-Transcript (Claude oder Codex) markiert und Enter drückt → öffnet als read-only Viewer mit dem Markdown-Renderer.

**Drag & Drop (User → Agent):** Eine vom Desktop/lokalen Filesystem in eine **aktive Session** gezogene Datei wird automatisch per scp auf den richtigen Host in deren cwd übertragen und der Pfad in die Prompt-Zeile eingefügt — ein Handgriff statt „scp-Kommando bauen". Bilder werden (analog claudeplex-desktop) inline angehängt. Zwischen den Panes (lokal↔lokal, lokal↔remote, remote↔remote) ebenfalls per Drag oder F5/F6.

**Agent → User (Rückkanal, mit Consent):** Das Gegenstück — ein Agent kann dem User eine Datei schicken oder etwas ins Clipboard legen. Das läuft über die MCP-Tools mit **Bestätigungs-Modal** (Vertrauensgrenze bei zaplex, nie beim Agenten) — Details §7. Diese und der Transcript-Viewer sind die Stellen, an denen MC- und Agent-Schicht direkt verzahnt sind.

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

Der Provider wird **nicht** per Hotkey gewählt — er ist erstes Feld im Launch Wizard. (Falls sich im Alltag ein Bedarf für „neuer Codex-Agent" / „neuer Claude-Agent" als direkte Hotkeys zeigt, später ergänzen — nicht vorbeugend.)

### 5.7 Mentale-Last-Reduktion & Premium-Politur (Calm by default)

Die UX-Direktive (§2.1) konkret: **absolut aufgeräumt, ästhetisch, intuitiv — kein „ugly dev design", kein Noise, keine hässlichen Elemente.** Optik erbt durchgängig Zaps Theme-System; aus claudeplex-desktop übernehmen wir die *Muster*, nicht das CSS.

- **Calm by default.** Die Standardansicht zeigt **nur, was Aufmerksamkeit braucht**; alles andere kollabiert. Anti-Overwhelm ist Designprinzip, nicht Filter-Option — auch bei vielen Projekten/Sessions/Agents bleibt der Screen ruhig.
- **„Needs-me"-Router.** Ein Hotkey (`Cmd-Shift-W`, §5.6) springt zur **nächsten Session, die auf *dich* wartet** — priorisiert über alle Hosts/Accounts/Provider. Die direkteste Antwort auf „mentale Last". Status-Glyphen (● aktiv · ◐ läuft-woanders · ◷ wartet · ○ stale) für Sofort-Scan (claudeplex/-desktop-Muster).
- **Ruhiges Cost/Heat-HUD.** Dezente, glanceable Kopfzeile: Sub-Auslastung (5h + Woche) je Claude/Codex mit Heat-Coloring, Gesamt-Spend, und *welcher Account die aktive Session fährt*. Reset-Countdown. Nie blinkend, nie aufdringlich.
- **Command-Palette (`Cmd-K`).** Universeller Fuzzy-Sprung: Session/Host/Account/Datei/Aktion. Maus optional, nie Pflicht (Tastatur-first, §2.2).
- **Watch & Adopt.** Read-only-Mitlesen einer woanders laufenden Session; Tippen in die Intake-Zeile adoptiert sie in-place (gleiche Session-id). Senkt „ist es schon fertig?"-Anspannung (Desktop-Muster).
- **Resume everywhere.** Laptop morgens auf → alle Agents laufen noch, sofort re-attached **mit Historie** (Persistenz-Layer §3.5 + Output-Replay). Kein „blank on reopen".
- **Per-Projekt-Gruppierung.** Sessions nach Projekt/Repo/Host gruppiert und kollabierbar — viele Sessions erdrücken nicht.

Jedes Element muss **wirksame Entlastung** bringen; was nur „nett" ist, fliegt (§2.3).

---

## 6. Roadmap

### 6.1 v0 — „funktioniert geil, hybrid intern" (Wochen)

**Scope:** Provider-Abstraktion + Account Dock + Agent Tree + Launch Wizard (mit Provider-Auswahl) + Inline-Prompting in Blocks. Claude-Datenschicht aus Bun via `claudeplex --json`; Codex-Datenschicht nativ in Rust. Noch keine MC-Pane, noch keine eigene Fleet-Steuerung im UI (Fleet existiert weiter via Bun-CLI für Claude).

**Definition of done:**
- Account Dock zeigt alle Accounts **beider Provider** mit korrektem Heat und Provider-Badge
- Agent Tree zeigt alle Sessions korrekt (provider-gemischt), „N waiting" stimmt
- Launch Wizard startet Agenten **auf dem gewählten Provider** auf dessen freestem Account, Block öffnet, Prompt geht durch
- Adopt-by-Enter funktioniert für beide Provider: Session aus Sidebar → Block, history visible, prompt funktioniert
- Visuelle Abnahme: 3 unbeteiligte Screenshots, niemand erkennt „angeflanscht"; Claude- und Codex-Account sind im Dock sichtbar

### 6.2 v1 — „nativ und sauber" (Monate)

**Scope:** Claude-Bun-Datenschicht nach Rust portieren (`discover.ts`/`collect.ts`/`usage.ts` → `zaplex_accounts`-internals, als zweite `ProviderBackend`-Impl neben der bereits nativen Codex-Impl). Eigene Fleet-Steuerung im UI (Start/Stop von remote-control-Servern aus dem Account Dock heraus, Claude). MC Dual-Pane.

**Trigger für den Start:** v0 läuft seit X Wochen ohne Krücken-Gefühl. Bun-Subprocess wird als spürbare Latenz/Fragility erlebbar.

**Sicheres Remote-Entwickeln (§3.5):** parallel zur Provider-Arbeit einplanen, da provider-unabhängig (Terminal-/MC-Hälfte). **Track A** (Zapify — Multiplexer-kompatible Shell-Integration) vorab als Spike — falls Upstream das schon gelöst hat, ggf. schon in v0 übernehmbar. **Track B-B2** (Persistenz auf Basis des vorhandenen `remote-server`-Daemons) in v1; **B1** (mosh-Roaming) und **B3** (nativer UDP-Transport) folgen.

### 6.3 v2 — „upstream contribution oder permanent fork" (offen)

**Optional:** `zaplex_accounts` als Patch-Set an Zap anbieten. Multi-Identity über mehrere Provider ist auf Zaps Roadmap als Lücke benannt. Wenn der Maintainer annimmt: Rebase-Last weg.

Falls nicht: private Fork läuft weiter, kein Drama.

### 6.4 Ausblick — Mobile Companion (iPhone/Android, **nicht eingeplant**)

> Zukunfts-Idee, **steht unmittelbar nicht an** — hier nur als möglicher Ausblick, damit die Architektur ihn nicht verbaut.

Ein **Companion für unterwegs**, um Sessions zu **überwachen und leichtgewichtig zu steuern**, wenn der Desktop nicht greifbar ist. Kein zweites Voll-Terminal — eine **stark angepasste UI** mit bewusst **reduziertem Funktionsumfang**: nur das, was ein Dev mobil wirklich braucht.

- **Sehen (Glance):** Welche Session braucht *jetzt* meine Aufmerksamkeit (Needs-me, §5.7)? Status aller Sessions/Agents; Sub-Auslastung/Heat + Rate-Limit-Warnungen je Claude/Codex; Kosten; der letzte Output/Transcript-Tail einer Session.
- **Tun (wenige, hochwertige Aktionen):** auf einen wartenden Prompt antworten (Approve/Deny, kurze Text-Antwort), eine Session pausieren/fortsetzen/killen, Subscription umschalten, einen **vordefinierten** Agenten starten. **Push-Notifications** für „braucht Input" / „fertig" / „nahe Rate-Limit" (gespeist aus `zaplex.signal_attention`, §7).
- **Bewusst NICHT mobil:** volles Terminal-Editing, MC-Dateimanagement, Multi-Pane — das bleibt Desktop. Mobil zählt *glanceable + ein paar wirksame Eingriffe*, kein Mini-Desktop.

**Warum es architektonisch trägt:** Der **native Session-Daemon** (§3.5) macht es erst möglich — die Sessions leben host-seitig persistent, also kann ein Telefon sich ein-/ausklinken, ohne dass etwas abbricht. **Provider-agnostisch** (deckt auch **Codex** ab — anders als `claude remote-control`, das nur die Claude-Mobile-App bedient). Anbindung über den `zaplex.*`-MCP-/Server-Layer (§7). Die Persistenz-Entscheidung (Option 1) ist damit zugleich die Voraussetzung, dass dieser Ausblick später überhaupt sauber baubar ist — wir verbauen ihn heute nicht.

---

## 7. MCP — ergänzende Rolle

MCP ist **nicht** Ersatz für UI (siehe §2.3), aber sinnvolle Beigabe. zaplex stellt einen eigenen MCP-Server bereit (Namespace `zaplex.*`, provider-aware), über den **Claude Code und Codex an zaplex gekoppelt** werden — **bidirektional**: der Chat erreicht zaplex, und der *im Agenten laufende* Claude/Codex koppelt sich an zaplex zurück.

**Read-mostly / Orchestrierung (aus dem Chat heraus):**
- `zaplex.list_accounts` → strukturierte Liste, jeder Eintrag mit `provider`
- `zaplex.get_usage(account)` → Detail-Heat
- `zaplex.list_sessions(filter)` → alles über alle Hosts/Provider
- `zaplex.launch_agent(provider, account, cwd, prompt)` → Agent startet (Block öffnet im UI)
- `zaplex.adopt_session(id)` → öffnet als Block

**Agent → zaplex (der im Agenten laufende CLI koppelt zurück):**
- `zaplex.signal_attention(session, reason)` → der Agent meldet „brauche Input / bin fertig" → speist den **Needs-me-Router** (§5.7)
- `zaplex.copy_path(from, to)` → Datei über den MC-Layer holen/schieben (lokal↔remote↔remote)

**Agent → User (Rückkanal, bestätigungspflichtig):**
- `zaplex.send_to_clipboard(content)` → der Agent legt Text/Snippet ins **Clipboard des Users**
- `zaplex.send_file_to_user(path)` → der Agent schickt eine **Datei** vom Remote-Host an den User (Gegenstück zum User→Agent-Drag&Drop, §5.5)

**Consent-Modell (verbindlich):** Die **Vertrauensgrenze sitzt bei zaplex, nicht beim Agenten.** Der Agent *bittet* nur; zaplex *entscheidet und zeigt das Modal*:
- Clipboard-Write → Toast/Bestätigung mit Vorschau („Agent *X* möchte ins Clipboard schreiben: «…» — übernehmen?").
- Datei-Eingang → Annahme-Modal („Agent *X*, Host *h* sendet `datei.ext` (Größe) — annehmen nach *~/Downloads* / in die aktive Pane?"); Transfer in eine **Staging-Area**, erst nach OK sichtbar.
- Gegen Reibung ohne Noise: optionales „für diese Session/diesen Agenten immer erlauben" — **Default bleibt fragen**. So ist es Entlastung, nicht Überrumpelung (§2.3).

Das macht Zaplex-Daten/Aktionen aus dem Chat heraus erreichbar — *zusätzlich* zur UI, nicht als Ersatz. Ein Slash-Command im Chat („starte einen neuen Codex-Agenten auf dem freisten Account") ruft das MCP-Tool auf, der Agent öffnet im UI als Block. Schöne Symmetrie.

**Implementation:** als eigener kleiner Rust-Binary (`zaplex-mcp`), der auf `zaplex_accounts`/`zaplex_sessions` (und für den Rückkanal auf den Session-/MC-Layer) zugreift. Kein UI, nur stdio MCP server; die Consent-Modals rendert die zaplex-App. Kommt nach v1.

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

Zwei-Stufen-Rebase. Beherrschbar **nur**, wenn unsere Änderungen 95%+ in eigenen `zaplex_*`-Crates leben.

### 8.2 Branch-Strategie

- `main` — tracked `zerx-lab/zap:main` (regelmäßiger Rebase, alle 1-2 Wochen)
- `zaplex` — unser Feature-Branch, der über `main` rebased wird
- Releases / Builds: vom `zaplex`-Branch

### 8.3 Touchpoint-Disziplin

**Erlaubt** in fremden Crates:
- `warpui` / `warp_terminal`: Panel-Registrierung, Hotkey-Binding, ein Import-Block für unsere UI-Komponenten
- `settings`: Schema-Erweiterung für zaplex-Settings (Provider-/Account-Defaults, Hotkeys)

**Verboten** (würde Rebase-Hölle erzeugen):
- Logik in `warp_ssh_manager` ändern (auch wenn's verlockend ist) — stattdessen wrappen
- UI-Komponenten in `warpui_core` modifizieren — stattdessen eigene in `zaplex_ui`
- Schema-Änderungen in `settings`, die existierende Settings beeinflussen

**Faustregel:** Wenn ein Patch in einem `warp_*`-Crate >20 Zeilen wird, ist es vermutlich falsch verortet — lieber ein neues Hook-Pattern im eigenen Crate vorschlagen.

### 8.4 Upstream-Sync-Disziplin

- Rebase-Termin im Kalender: alle 2 Wochen
- Vor jedem Rebase: `cargo test` muss grün sein
- Nach jedem Rebase: visueller Smoke-Test (Account Dock öffnet mit beiden Providern, Block startet, Prompt geht durch)
- Bei Konflikten in fremden Crates: lieber den eigenen Code anpassen als den upstream-Patch verformen

---

## 9. Erster Tag — konkrete Schritte für die neue Session

Die neue Session soll dieses Dokument lesen und dann **in dieser Reihenfolge**:

1. **Fork existiert bereits:** [iret77/zaplex](https://github.com/iret77/zaplex) (Fork von `zerx-lab/zap`)
2. **Lokal klonen** nach `~/projects/zaplex/iret77/zaplex/` (folgt der host-lokalen Projekt-Ordner-Struktur: `~/projects/<projekt>/<gh-org>/<repo>/`) — *bereits erledigt*
3. **Build-Voraussetzungen** klären: Rust toolchain (1.92.0, gepinnt), plus System-Libs (`libclang-dev`, `protobuf-compiler`/`protoc`, `libssl-dev`, `libfreetype-dev`, `libexpat1-dev`, `libgit2-dev`, `libdbus-1-dev`, `libfontconfig1-dev`, `libasound2-dev`) sowie `corepack enable` + `yarn install` für `crates/command-signatures-v2/js`. Referenz: `script/linux/install_build_deps`.
4. **Lokalen Build** durchführen, App starten — sicherstellen, dass die Basis funktioniert (`cargo check --workspace` grün)
5. **`warp_ssh_manager` lesen** — das ist die Blaupause. Ziel: verstehen, wie ein Sidebar-Panel-Crate in Zap aussieht (Datei-Layout, Cargo.toml-Deps, Einhängung in `warpui`). **Zusätzlich:** kurz ansehen, wie Zap die CLI-Agenten (Claude Code, Codex) als Blocks verdrahtet — das ist unser Action-Layer-Anker.
6. **Diesem Konzept folgen** für Architektur und UX
7. **Erste Crate anlegen**: `zaplex_accounts`, sehr klein zum Start. Zuerst die **Provider-Abstraktion** (`Provider`-Enum + `ProviderBackend`-Trait) und **eine** Discovery-Impl. Empfehlung: mit **Codex beginnen** (greenfield, nativ Rust, kein Bun-Hop — der sauberste erste Schnitt), d. h. `CODEX_HOME`-Discovery: welche Codex-Subscription-Logins gibt es. Ohne UI. Ohne Usage. Pure Library mit einem Test. **Dann** die Claude-Discovery-Impl (v0: shellt zu `claudeplex --json`) als zweite `ProviderBackend`-Instanz.
8. **Erst dann** UI dazubauen: Account Dock als simpelster Sidebar-Eintrag mit Account-Liste (beide Provider, mit Provider-Badge), ohne Heat-Bars. Visuell verifizieren, dass es sich nativ anfühlt.

**Nicht im ersten Tag:**
- Nicht versuchen, alles auf einmal zu portieren
- Nicht versuchen, das Bun-Backend zu ersetzen
- Nicht in `warp_terminal` schnipseln, bevor klar ist, wie Zap Panels registriert
- Nicht „die ganze claudeplex-Logik" nach Rust kopieren
- Nicht Usage/Heat vor der reinen Discovery bauen — erst muss die Account-Liste beider Provider stehen

---

## 10. Referenzen

### 10.1 Bestehender Code (claudeplex, Bun — Datenseite-Vorbild)

- `/home/dev/projects/claudeplex/` — Bun-TUI, getestet (nur Claude)
- `src/discover.ts` — Account-Discovery (`CLAUDE_CONFIG_DIR`-Enumeration)
- `src/collect.ts` — Session-Inventory, Usage-Parsing, PSS-Observer
- `src/usage.ts` — 5h-/Wochen-Fenster, Reset-Logik
- `src/agent.ts` / `src/agents.ts` — Spawn-Layer für `claude`-Subprocess (Vorlage für Action-Layer)
- `src/remote.ts` — Fleet-Supervisor mit RAM-Governor
- `src/hosts.ts` — Host-Discovery (`~/.ssh/config` + Tailscale)
- `src/pr.ts` / `src/issue.ts` — PR-Review + Quick-Issue (headless `claude -p`)
- `src/index.ts` — `--json`-Output, Format: `{accounts, remote}`

*(Für die Codex-Seite gibt es bewusst kein Bun-Vorbild — sie wird nativ in Rust gebaut, siehe §3.3.)*

### 10.2 Codex (Provider-Referenz)

- CLI: `codex` (OpenAI Codex CLI), als Zap-Block bereits verdrahtet
- Auth: `codex login` → „Sign in with ChatGPT" (Subscription, **kein API-Key**)
- Config-Home: `$CODEX_HOME` (Default `~/.codex`), Token in `auth.json`; mehrere Logins = mehrere `CODEX_HOME`-Dirs
- Subscription-Tiers mit Rate-Fenstern (5h + Woche), analog zu Claude Max
- **Bei Umsetzung zu verifizieren:** exakte Quelle der Rate-Limit-/Restkontingent-Daten; Resume-Flag für Adopt-by-id; tatsächlicher RAM-Footprint pro Session

### 10.3 Zap

- Repo: [zerx-lab/zap](https://github.com/zerx-lab/zap)
- License: AGPL-3.0 (Client), MIT (`warpui`, `warpui_core`)
- Default branch: `main`
- Blaupause-Crate: `crates/warp_ssh_manager/`
- UI-Crates: `crates/warpui/`, `crates/warpui_core/`, `crates/ui_components/`
- Terminal: `crates/warp_terminal/`
- Settings: `crates/settings/`
- CLI-Agent-Adapter (Claude/Codex/agy): im `app/`-Tree verdrahtet — Action-Layer-Anker
- Doku: `docs/migrate-from-warp.md`, `docs/roadmap.md`
- Discussions: [warpdotdev/warp Discussion #9240](https://github.com/warpdotdev/warp/discussions/9240) (Open-Source-Ankündigung)

### 10.4 Verworfene Alternativen

Für Kontext, damit die neue Session nicht in dieselbe Diskussion zurückfällt:

- **MCP-only-Ansatz:** verworfen (siehe §2.3 — fehlende visuelle Permanenz)
- **Electron-App als Produkt** (`iret77/claudeplex-desktop`, Fork der Team-App von Marcel): als *Auslieferungsform* nicht weiterverfolgt (Electron baut Terminal-Unterbau redundant nach; Zap liefert ihn fertig) — bleibt aber **Referenzquelle** für UI/UX-Muster und Implementierungsideen (§1, §5.7).
- **Standalone claudeplex weiterführen:** **Nein.** Den claudeplex-Fork (`iret77/claudeplex`) führen wir **nicht** als Fork weiter — er bleibt **reine Referenzquelle** (Ideen-/Code-Vorbild), wird aber nicht aktiv gepflegt. Ob das **Team** die **Original-Repos** (claudeplex / claudeplex-desktop) fortführt, hängt davon ab, **wie überzeugend zaplex wird** — das ist die Entscheidung des Teams, nicht Teil dieses Konzepts. Das Cockpit-UI lebt zukünftig **allein** im Zap-Fork (zaplex).
- **Warp (upstream) forken statt Zap:** Zap gewinnt wegen Local-first + bereits verdrahteter CLI-Agent-Integration (Claude **und** Codex) + Maintainer-Zugänglichkeit.
- **Eigene Bun-Implementierung für Codex:** verworfen — keine getestete Vorlage vorhanden, Symmetrie-um-der-Symmetrie-willen wäre eine Krücke. Codex wird direkt nativ in Rust gebaut (§3.3).
- **API-Key statt Subscription als Fokus:** Subscription-Support ist Must-Have und das Zentrum der Orchestrierung (§1). Zaps bestehender API-Key-/BYOP-Pfad (§3.1) wird aber **nicht** entfernt — er bleibt verfügbar, wo schon vorhanden; nur Heat/Routing sind subscription-zentriert.

### 10.5 Vorarbeit-Memory (lokal beim Maintainer)

Frühere Session-Erkenntnisse liegen als lokale Memory-Snapshots beim Maintainer (claudeplex-Conductor-Reframe, Fleet-Design, Theme-Architektur, Electron-Entscheidung). Sie sind nicht öffentlich, aber alle relevanten Konzepte sind in diesem Dokument konsolidiert — eine neue Session braucht sie nicht zu lesen, um loszulegen.

---

## 11. Anti-Patterns — was die neue Session NICHT tun soll

Damit nichts in die falsche Richtung kippt:

1. **Kein eigenes Theme-System.** Nicht Lumen, nicht Truecolor-Gradients, nicht „aber claudeplex hatte das so schön". Zaps Theme. Punkt.
2. **Keine eigene Sidebar-Komponente von Null bauen.** Erst angucken, wie Zap Sidebars macht, dann das Pattern erweitern.
3. **Kein FFI/Memory-Sharing zwischen Bun und Rust.** Subprocess + NDJSON, sauber. *(Gilt für die Claude-Seite; Codex hat ohnehin keinen Bun-Hop.)*
4. **Keine „weil claudeplex es so machte"-Argumente.** Die claudeplex-Konventionen sind reines Vorbild für die Datenseite. UI-Konventionen kommen von Zap.
5. **Keine TODO/FIXME für „MC-Pane macht v2".** Wenn etwas nicht im Scope ist, NICHT andeuten. Sauberer Code statt vorsichtshalber-Hook.
6. **Kein „schnell mal" Subprocess-Call von der UI-Schicht aus.** Action-Layer ist Action-Layer, UI ist UI. Trennung wahren auch beim Start.
7. **Provider-Enum ja — Spekulations-Enum nein.** Es gibt jetzt **zwei reale Provider** (Claude, Codex) → `Provider { Claude, Codex }` ist berechtigt und gewünscht. Aber NICHT vorbeugend um Tiers (`Pro`, `Team`, `Enterprise`) oder einen dritten, noch nicht existierenden Provider erweitern. Genau die zwei realen Fälle modellieren, nicht mehr.
8. **Nichts „claudeplex" nennen.** Jedes neue Artefakt heißt `zaplex_*` / `zaplex.*` / `zaplex-*`. „claudeplex" steht ausschließlich für das bestehende Bun-Referenz-Tool, wenn wir darauf verweisen. (Namensregel im Header.)
9. **Keine vorgetäuschte Codex-Parität.** Wo Codex eine Fähigkeit nicht hat (remote-control-Server, §3.4), keine Fake-Schicht bauen — ehrlich degradieren.

---

## 12. Erfolgskriterien

Wie wir wissen, dass es geil geworden ist:

- Der User benutzt es täglich, das alte claudeplex-TUI nicht mehr.
- Marcels Electron-App ist obsolet (nicht aktiv gekillt — sie wird einfach nicht mehr gestartet).
- Ein Außenstehender, dem man einen Screenshot zeigt, fragt „seit wann hat Zap Multi-Account über Claude UND Codex?" — nicht „was hast du da für eine Erweiterung?".
- Beim Multi-Tasking über mehrere Accounts **und beide Provider** hat man jederzeit den Heat-Status im peripheren Blickfeld, ohne hinzuschauen.
- Ein neuer Agent ist in unter 5 Sekunden gestartet — Provider wählen, freester Account ist vorausgewählt, Prompt rein, läuft.
- Eine wartende Session ist nie länger als 5 Sekunden unbemerkt — egal welcher Provider.
- Ein Verbindungsabbruch / zugeklapptes Notebook unterbricht **keinen** laufenden Agenten — beim nächsten Connect ist alles nahtlos wieder da.
- Ein anspruchsvoller Remote-Dev **außerhalb** des Teams, der zaplex zum ersten Mal sieht, will es sofort nutzen — nicht „nett", sondern „das nehme ich ab jetzt".

Wenn diese Punkte nach v1 stehen, hat sich der Fork gelohnt.
