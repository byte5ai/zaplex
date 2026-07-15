# Cockpit-Redesign v2 — verbindliche Spec (User-abgenommen 2026-07-14)

> Ersetzt den Sidebar-Teil von `2026-07-11-cockpit-sidebar-redesign.md` und erweitert
> ihn um Konto-Panes, Sessions-Tabelle und Plexing. Konvergiert über ~10 Iterationen
> am Mockup (Artefakt v5). **Jede Regel hier ist Bau-Vorgabe, keine Empfehlung.**
> Referenzen: `2026-07-03-claudeplex-gap-analysis.md` (Plexing = C4-Launcher/Routing),
> `2026-07-08-integrated-ux-spine-design.md` (Objektbaum).

## 0. Leitprinzip — präattentive Wahrnehmung

Form und Farbe tragen die Information, **bevor** man liest. Keine Datenwüste, keine
gleich-großen Kleinstelemente. Das Auge wird durch Struktur geführt.

## 1. Encoding-System (eine Regel pro Datentyp — überall gleich)

| Datentyp | Kodierung | Verboten |
|---|---|---|
| **Session-Zustand** | Punkt-**Form**: Wartet = Punkt mit Halo (Doppelring) · Aktiv = gefüllt · Idle = hohl. In Tabellen zusätzlich das **Wort**. | Zustand NIE nur über Farbe (Rot-Grün-Schwäche). |
| **Auslastung** (Kontext-%, 5h-Meter) | Zahl + Balken, **immer grau**, erst **ab 90 % rot**. | Kein Amber/Grün für Auslastung. |
| **Attention** | **Amber, exklusiv** für „wartet auf Dich". Stille Punkt-Spur: Host-Punkt → Session-Punkt. | Amber für nichts anderes. Keine lauten Rand-Balken/„Klammern". |
| **Provider** | Farbkachel (Clay=Claude, Blau=Codex) **nur** in Konten-Cards + Tabelle. | Provider-Farben NIE im Tree. |
| **Auswahl** | Dezente Akzent-Fläche (Indigo `#6b74e8`). | Kein Balken, kein zweiter Akzent pro Zeile. |

Nichts codiert doppelt. Ein Akzent pro Zeile.

## 2. Sidebar

Keine Maximieren-Icons. Zwei Zonen, keine Verwaltungs-Icons in der Sidebar
(Ausnahme: Zahnrad, Stern).

### 2.1 Zone „VERBINDUNGEN" — der Tree (Host → Projekt → Session)
- **Header:** Kapitälchen-Label + Zähler + **Zahnrad rechts** (optische Größe = Favoriten-Stern,
  ~18 px). Zahnrad öffnet die SSH-Konfiguration → **ersetzt den separaten SSH-Config-Tab**.
  „+ Host hinzufügen" entfällt (läuft übers Zahnrad).
- **Host-Zeile:** Zustands-Punkt (Worst-Child: amber wenn ein Kind wartet) · Hostname (fett) ·
  Session-Zähler · ★-Favorit · ⋯ (nur bei Hover). Idle-Host = hohler Punkt.
- **Projekt-Ebene:** **typografischer Gruppen-Header** in Kapitälchen + **Disclosure-Chevron**
  (gezeichnet, ›/⌄, vertikal auf Textmittellinie, ~7 px Abstand zum Namen). **Kein Punkt vor dem
  Projekt.** Ein-/ausklappbar. Session-Zähler rechts.
- **Session-Zeile:** eingerückt, `[Zustands-Punkt] Branch(flex, truncatet) [Kontext-%]`.
  **Nur der Branch** — kein Projektpräfix (das trägt der Gruppen-Header). Kein Provider-Icon im Tree.
  Waiting-first innerhalb des Projekts.
- **Leerzustand:** „Keine laufenden Sessions" bündig unter dem Hostnamen (auf der Textkante).

### 2.2 Zone „KI-KONTEN" — Verbrauchs-Cards (separate Achse, NICHT der Tree)
- **Header:** Kapitälchen-Label + Zähler. Kein Icon rechts.
- **Card (klickbar):** Provider-Kachel · **Anzeigename** · Provider-Wort · Plan-Badge.
  **Ein Signal:** 5-Std-Meter (grau). Darunter „**N laufende Sessions**" (= aktiv + wartend,
  **ohne** idle). **Keine Mailadresse in der Sidebar** (Noise). Keine ⋯ in der Sidebar.
- **Klick auf Card** → öffnet/fokussiert das Konto-Pane (§3).

## 3. Konto-Pane (großes Cockpit) — eines pro Konto

- **Max. ein Pane pro Konto.** Zweites Konto = zweites Pane. Dedup per Konto-Identität
  (gleiche Mechanik wie Cockpit-/SSH-Pane-Dedup).
- **Titel:** „`<Host> — <Konto-Anzeigename>`" (z. B. „devhost — Hauptkonto").
- **Oben:** Konto-Detail-Card — Provider-Kachel · Anzeigename · **Mail (hier erlaubt, Platz)** ·
  Plan · Reset-Timer · **⋯ (Konto verwalten / Alias ändern — nur hier, nicht in der Sidebar)**.
  5h- + Wochen-Meter (grau), 3-Spalten-Tabelle Heute/5h/Woche × $/Tokens.
- **Sessions-Tabelle** (der Mehrwert gegenüber der Sidebar):
  - **Suchfeld** — Live-Filter über Projekt, Branch, Worktree (skaliert auf Hunderte Sessions).
  - **Filter-Chips:** Alle / Wartet / Aktiv / **Idle** (Idle listet pausierte, wiederaufnehmbare Sessions).
  - **Gruppierung** umschaltbar (Default: Projekt; auch Status/Modell). Gruppen-Header = Kapitälchen +
    Chevron, einklappbar (gleiches Vokabular wie der Tree).
  - **Sortierbare Spaltenköpfe.** Spalten: Session (Branch) · Worktree · Modell · **Kontext (Balken+%)** ·
    **Kosten heute** · Status (Punkt+Wort) · Zuletzt. Zahlen rechtsbündig, tabellarisch, **einheitliche
    Spaltenabstände** (nichts angeklebt).
  - **Zeilen-Klick** → öffnet/fokussiert die Session (max. eine Instanz).

### 3.1 Agent-Steuerung („Cockpit-Drive") — das ⋯ pro Zeile · **KEINE REGRESSION**

> **Nachgetragen 2026-07-14 (User-Entscheidung).** Die erste Fassung dieser Spec kannte nur
> „Hover-Aktionen: Fokussieren · Fork · Fortsetzen" und hätte damit die halbe Agent-Steuerung
> des alten Dashboard-Panes **gelöscht**. Das war ein Fehler der Spec, nicht ein Grund, Funktion
> zu streichen. **Eiserne Regel: der Umbau darf keine einzige heute vorhandene Agent-Steuerung
> verlieren.** Wer diese Spec umsetzt, prüft das aktiv gegen `app/src/cockpit/pane.rs`.

- **Zeilen-Klick öffnet/fokussiert die Session** (siehe oben) — er wird *nicht* für Auswahl/Detail
  verbraucht.
- **Das ⋯ am Zeilenende trägt die vollständige Steuerung** der Session, **kontext-gated** (nur was
  zutrifft wird gezeigt), destruktives am Ende abgesetzt:
  - `Fork` · `Fork in neuen Worktree` (nur mit Fork-Mechanismus; Worktree nur im Git-Repo)
  - `⚙ /compact` · `⌫ /clear` (nur Claude **und** resumebare Session)
  - `Verlauf` (Transcript) · `Review ›` (Untermenü: review · ✓ approve · ↻ redirect · ⎙ commit ·
    ⬈ PR — **nur lokale** Sessions)
  - `⏸ Stop` · `⨯ Kill` (destruktiv, abgesetzt, amber/rot)
- **Warum Menü statt Hover-Verbs:** es sind bis zu 8+ kontextabhängige Verbs — in die Zeile
  gehovert wären sie exakt die Noise-Explosion, die §0/§1 verbieten. Ein ⋯ ist ein Vokabular mit
  den Host-Zeilen der Sidebar.
- **Fleet-weites `⏹ stop all`: KEINE Chrome.** Selten + destruktiv → **Command-Palette**
  (bestehender Confirm-Dialog bleibt). Es gehört **nicht** in die Zone „Verbindungen" (die trägt
  *Hosts*, nicht Agents — Kategoriefehler) und in kein einzelnes Konto-Pane (es ist fleet-weit).
  Der häufige Fall („dieser eine Agent nervt") liegt im ⋯ als Stop/Kill.
- **Der Conductor-Tree im Pane entfällt** — die Sidebar trägt ihn (§2.1); das ist Entdopplung,
  keine Regression. Er darf aber **erst** entfallen, wenn seine Verbs über das ⋯ ihren Platz haben.

## 4. Plexing (Konto-Routing beim Spawn) — Auto ist Default

- **Session ist fest an ein Konto gebunden** (`CLAUDE_CONFIG_DIR`-Pinning) — nicht verschiebbar.
  Verteilung passiert daher **beim Start**.
- **Auto-Modus = Default.** zaplex wählt automatisch das Konto mit der meisten 5h-Reserve;
  fast volle Konten (**≥ 85 %**) werden übersprungen. Die Spawn-Card zeigt **eine ruhige Zeile**
  (Kachel · Anzeigename · Plan · 5h-% · „Ändern") — **null Extra-Klicks**.
- **Manuell** über „Ändern" → Kontoliste mit Auslastung, für bewusste Abo-Wahl.
- Komfort: „gleiches Projekt auf Konto B starten" (Parallel-Session) · Cap-Warnung nahe 5h-Limit ·
  freiestes Konto auch als Ziel für Hintergrund-Analysen. KI-Konten-Zone macht die Lastverteilung
  präattentiv sichtbar.

## 5. Konto-Aliase (= claudeplex Account-Overrides)

- Optionaler **Alias** wird zum Anzeigenamen (löst gleiche Mail bei Claude+Codex).
  Ohne Alias bleibt die Mail der Anzeigename.
- Alias gilt **überall konsistent**: Sidebar-Card, Pane-Titel, Sessions-Tabelle, Spawn-Card.
- Bearbeiten **nur im Konto-Pane** (⋯). Persistenz in zaplex-Settings.
- Fundament für spätere Overrides (Farbe/Reihenfolge/Ausblenden) auf demselben Mechanismus.

## 6. Terminologie & Sprache (Anti-AI-Slop)

- **„laufend"** = aktiv + wartend. Idle/pausiert wird separat benannt, nie in „N laufende Sessions".
- **Konsequent Deutsch**, konsistente Groß-/Kleinschreibung am Satzanfang (auch in Kommentaren/Tooltips).
- Deutsche Typografie: „42 %" (schmales Leerzeichen), „4 h 00 min".
- Keine Emojis in der UI (nicht premium). Icons = SVG in Standard-Icongröße, keine Font-Glyphen.
- **i18n:** die deutsche `warp.ftl` hat nur ~12 % der Keys → fehlende Keys fallen auf EN zurück
  (das ist der DE/EN-Mix). Fix = DE-Keys ergänzen + **FTL-Parity-Check** (jeder EN-Key braucht DE),
  damit es nicht zurückkommt. Bekannte Leaks: `workspace-new-worktree-config`/`-new-tab-config`/
  `-reopen-closed-session`, der `terminal-zero-state-*`-Block, `terminal-input-agent-hint-*`,
  Literal `"Refresh"` in `server_file_browser.rs`.

## 7. Bau-Reihenfolge

1. **i18n-Vervollständigung** (fehlende DE-Keys + Parity-Check) — risikoarm, voll gemappt.
2. **Sidebar/Tree + KI-Konten-Cards** — Encoding-System, Chevrons, Terminologie.
3. **Konto-Pane + Sessions-Tabelle** (Suche/Gruppierung/Sortierung/Filter, Dedup pro Konto) —
   **inkl. §3.1 Cockpit-Drive (⋯-Menü)**: erst wenn jeder Verb des alten Panes dort einen Platz
   hat, darf der Pane-Tree entfallen. Vorher gegen `app/src/cockpit/pane.rs` gegenprüfen.
4. **Plexing** (Auto-Routing + Spawn-Card) — baut auf 5h-Heat + C4-Account-Pinning.
5. **FM-Modus-Badge** (Host bleibt im Tab-Titel) + **FM-SFTP-Bug** (Daemon-Browse statt Key-SFTP).

Jeder Schritt: eigener Commit, `cargo check --bin zaplex --locked` grün, codex+grok-Review.
**Ein RC-DMG + Daemon-Redeploy am Ende — nur auf explizite User-Freigabe.**

## 8. Offen (kein Blocker)

- Provider-Kachel: neutraler Farbpunkt (aktuell) vs. selbstgezeichneter, markenfreier Glyph.
- Header-Namen final: „VERBINDUNGEN" / „KI-KONTEN" (so abgenommen).
