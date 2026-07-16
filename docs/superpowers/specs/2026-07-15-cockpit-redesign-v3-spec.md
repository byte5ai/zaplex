# Cockpit-Redesign v3 — verbindliche Spec (ersetzt v2)

> **Ersetzt** `2026-07-14-cockpit-redesign-v2-spec.md` vollständig. v2 wurde adversarial
> gegen den Code reviewt (codex + grok, 2026-07-15) und enthielt Faktenfehler, innere
> Widersprüche und beschrieb an mehreren Stellen Daten/APIs, die nicht existieren — oder
> ignorierte Code, der bereits Besseres tut (Routing, Overrides).
>
> **Direktive des Eigentümers (2026-07-15, wörtlich sinngemäß):** Das Ziel bleibt
> unverändert auf Premium-Niveau. Fehlende Teile im Code sind **kein** Grund für
> Abstriche am Ziel — sie sind Arbeitspakete. Erst die Code-Basis solide und
> release-tauglich machen, dann die UI/UX-Änderungen umsetzen.
>
> **Methode dieser Spec:** Jede Aussage über den Ist-Zustand ist mit `file:line` belegt.
> Jede Lücke ist ein nummeriertes Arbeitspaket (F/E/S/P/X) mit Abnahmekriterium.
> Nichts hier ist Empfehlung — alles ist Bau-Vorgabe.
> Review-Regel: codex (zwei Linsen) + Selbstverifikation am Code. **grok nicht nutzen**
> (Security-Leak, User-Anweisung 2026-07-15).

---

## 0. Ziel & Leitprinzip (unverändert)

Präattentive Wahrnehmung: Form und Farbe tragen die Information, bevor man liest.
Keine Datenwüste, keine gleich großen Kleinstelemente, keine Regression bestehender
Funktionalität — **je Konflikt gewinnt das Produktziel, der Code wird nachgezogen.**

---

## 1. Vokabular (Encoding + Schwellen — EIN System, überall)

### 1.1 Zustand = Form (+ Wort in Tabellen)

| Zustand | Form | Farbe |
|---|---|---|
| Wartet | Fisheye/Halo (Punkt im Ring) | Amber — **exklusiv** für Warten |
| Aktiv / Monitor | gefüllter Punkt | Grün |
| Idle | hohler Ring | Gedämpft |

- `Monitor` (Tool-Lauf/Live-Job) kollabiert bewusst auf „aktiv" — der Blick
  unterscheidet arbeiten/warten/ruhen, nicht Busy-Substates. (`SessionState` hat vier
  Zustände: `crates/zaplex_cockpit/src/types.rs`.)
- **„laufend" = alle nicht-idle** (Aktiv + Wartet + Monitor). v2 definierte
  „aktiv + wartend" und widersprach dem Code (`panel.rs:352` zählt non-idle) — der
  Code hat recht, die Definition wird angepasst.
- **Rendering-Technik ist frei** (korrigiert 2026-07-16). Eine frühere Fassung
  verlangte „alle Status-Zeichen als SVG" und machte daraus ein Arbeitspaket — das
  war **erfunden**, nicht gefordert. Maßstab ist allein: **keine Emojis** (§7),
  farblich ruhig, konsistent. `●/◉/○` sind geometrische Glyphen, keine Emojis, und
  erscheinen auch **inline in Textbadges** (Attention-Badge, Inbox) — dort müssen
  sie identisch aussehen, was gegen SVG spricht. Sie bleiben.
  Affordanzen (Chevron, Zahnrad, Stern) sind SVG-Icons, weil sie ohnehin
  Icon-Elemente sind — kein Dogma, nur der praktikable Weg.
- **Eine Quelle für Form + Farbe: `session_glyph` + `status_dot_coloru`.**
  Jede Render-Stelle zieht daraus; niemand hartkodiert Glyphen. (pane.rs:732 tat
  genau das und rannte damit in exakt die Rot-Grün-Kollision, die §1.1 verbietet —
  behobene Lektion: „konsistent mit X" im Kommentar ist keine Konsistenz.)

### 1.2 Auslastung: EIN Schwellwert, EIN Rot

**Beleg des v2-Fehlers:** `attention_coloru() == heat_coloru(HeatLevel::Critical)
== #F97316` (style.rs:204/98) — v2s „rot ab 90 %" färbte Auslastung in **exakt der
Warte-Farbe**. Zusätzlich existierten drei Schwellen (85 Heat-Band / 90 Spec / 85
Plexing).

**Neu — ein *visuelles* Vokabular:**
- **„fast voll" = ≥ 85 %** — die eine Schwelle für alle **Anzeigen**: Kontext-%,
  5h-/Wochen-Meter. (Deckungsgleich mit dem bestehenden `HeatLevel::Critical`-Band,
  format.rs:17-29 — Code und Sichtbarkeit fallen zusammen.)
- **Kein „Plexing-Skip" bei 85 %.** Der Router (`zaplex_cockpit::routing`,
  `OVER_BUDGET_HEAT = 1.0`) **überspringt nicht hart**, sondern *depriorisiert*
  vollere/arbeitende Konten über seinen Bindungsfenster-Score. „fast voll" ist die
  *visuelle* Marke; die Routing-Logik ist ihr eigener Vertrag (§5/X1). (Das war eine
  Falschbehauptung der ersten v3-Fassung — Routing skippt nirgends bei 85 %.)
- **Farbe für „fast voll" = echtes Rot** = `HeatLevel::Over`-Palette (#EF4444/#B91C1C),
  **immer kontrast-adaptiert** über `heat_coloru_on(…, bg)` (style.rs:128) — nie die
  Dark-Palette hart (`heat_coloru`) auf themebaren Flächen.
- Unter 85 %: gedämpftes Grau. Kein Grün, kein Amber, kein Gradient.
- → Arbeitspaket **E1** (der bereits gebaute Code in style.rs:405ff und panel.rs
  heat_bar nutzt Critical/90 % und `heat_coloru` — beides falsch).

### 1.3 Übrige Encodings

| Datentyp | Kodierung |
|---|---|
| Attention | Amber als **Status** ausschließlich für „wartet auf Dich". Stille Punkt-Spur Host-Punkt → Session-Punkt. **Nichts codiert doppelt** → needs-me-Zähler-Badge am Host **entfällt**, Amber-Chevron eingeklappter Projekte **entfällt**; stattdessen färbt sich der **Zähler** des eingeklappten Projekts amber (ein Signal, am Ort der Verbergung). → **E2** · **Carve-out:** destruktive Verben (Stop/Kill/Stop-all) dürfen **beim Hover** amber werden — das ist eine transiente Gefahren-*Affordanz* („du bist dabei, etwas Destruktives zu tun"), kein Dauer-Status; sie kollidiert nicht mit dem Warte-Status, weil sie nur unter dem Cursor und nur momentan erscheint (`VerbKind::Destructive`, style.rs:281). |
| Provider | Farbkachel (Clay `#C8724A` = Claude, Blau `#3D9BF0` = Codex) **nur** in Konten-Cards; **kontrast-adaptiert** analog `heat_coloru_on` (Light-Variante definieren) → **S3**. Nie im Tree. |
| Auswahl | **`theme.accent()`-Overlay** — v2s Hex `#6b74e8` ist gestrichen (App ist themebar; Code macht es bereits richtig, panel.rs:397). |
| Verwaltungs-Icons Sidebar | Erlaubt sind genau: **Zahnrad** (Zonen-Header), **★** (Host), **⋯** (Host). v2s Ausnahmeliste war unvollständig. ⋯ ist **immer gerendert** (faint, Hover färbt) — „Hover färbt um, re-layoutet nie" gilt weiter; v2s „nur bei Hover" ist gestrichen (Code macht es bereits richtig, style.rs:310ff). |

---

## 2. Fundament — Code-Basis release-tauglich (VOR weiterem UI-Bau)

Bestandsfehler und fehlende Datenpfade. Ohne sie ist jedes UI darüber Fassade.

- **F1 · Host-Identity statt Label-Match (P0, Bestandsbug).**
  Registry-Hosts werden per **Anzeige-Label** in Live-Hosts gemerged; der erste
  Treffer bekommt die `node_id` (fleet.rs:214/230). Zwei gleichnamige Hosts →
  Verwalten/Favorit/Öffnen trifft den **falschen** SSH-Eintrag. Merge auf stabile
  Identität umstellen. *Abnahme:* zwei Registry-Hosts mit gleichem Label bleiben
  getrennt bedienbar (Test).

- **F2 · „Heute" = lokale Tagesgrenze (P0, Bestandsbug).**
  `today`-Fenster rechnet in UTC (windows.rs:83/89) — um Mitternacht Europe/Berlin
  zeigt das Cockpit falsche Tageswerte. Auf lokale Tagesgrenze umstellen.
  *Abnahme:* Test mit fixierter TZ.

- **F3 · Idle-Discovery bauen.**
  Es gibt heute **keinen** Produktionspfad, der `SessionState::Idle` erzeugt: Claude
  liefert nur PID-lebende Registry-Sessions (sessions.rs:252/259), Codex verwirft
  ruhende Rollouts (codex_sessions.rs:10/14). Das Produktziel (Idle-Filter §4.3,
  claudeplex „Adopt idle Session") **bleibt** → Discovery für wiederaufnehmbare
  ruhende Sessions bauen (Claude: Registry-Einträge mit totem PID + jüngste
  Transcripts; Codex: ruhende Rollouts), gedeckelt & sortiert nach Recency.
  *Abnahme:* beendete CLI-Session erscheint als Idle und ist über Resume adoptierbar.

- **F4 · Kosten pro Session (heute).**
  Existiert nicht als Feld — braucht Transcript-Fold pro `session_id` im Spine.
  *Abnahme:* Tabelle §4 zeigt $-Wert je Session; Summe ≈ Konto-Heute.

- **F5 · Fleet-Join Konto ↔ Sessions über Hosts.**
  `AccountUsage.sessions` ist nur lokale Discovery (lib.rs:100-117); die Konto-Tabelle
  braucht die Sessions **aller** Hosts (Join über config_dir-Pin im Inventory).
  *Abnahme:* Remote-Session des Kontos erscheint in dessen Pane mit Host-Spalte.

- **F6 · Capability-Semantik statt Provider-Ifs.**
  Codex-Sessions haben `pid == 0` (codex_sessions.rs:20/231) → Stop/Kill unmöglich;
  Slash nur Claude+resumebar; Review nur lokal. Ein `SessionCapabilities`-Modell
  (can_signal/can_fork/can_slash/can_review/can_resume) speist Menüs & Verbs.
  *Abnahme:* ⋯-Menü zeigt für eine Codex-Session kein Stop/Kill.

- **F7 · „Geprüft"-Markierung persistent + ehrlich benannt.**
  Heutiges „approve" togglet ein In-Memory-`HashSet` (pane.rs:1370/1419) — nach
  Pane-Schließen weg. Persistieren (Spine-State) und in der UI ehrlich als
  „Als geprüft markieren" führen — es approved nichts beim Agenten.

- **F8 · Fleet-Kollaps ersetzen.**
  Ab > 2 Hosts / > 8 Sessions degradiert jeder ruhige Host zu rohem Text ohne
  Klick/★/⋯/Status (conductor.rs:175/193, panel.rs:452-463) — Interaktion stirbt
  genau bei Skalierung. Neu: Header-Zeile bleibt **immer** voll interaktiv (Punkt,
  Name, Zähler, ★, ⋯); nur die Kinder falten.

- **F9 · Projekt = Repo, Worktree = Attribut.**
  `project_root` ist der Root **jedes Worktrees** (project.rs:32/40, fleet.rs:133) →
  parallele Worktrees erzeugen mehrfach denselben Projekt-Header. Neu: Gruppierung
  auf dem Haupt-Repo-Root (`git worktree`-Hauptpfad), Worktree wird Session-Attribut
  (die Tabelle §4 hat dafür die Spalte). *Abnahme:* 3 Worktrees von zaplex = 1
  Gruppe „zaplex" mit 3 Sessions.

---

## 3. Sidebar (Soll unverändert — Rest-Arbeitspakete)

Gebaut & abgenommen: 3-Ebenen-Tree (Host→Projekt→Session), einklappbare Projekte,
Session-Zeile = Branch + Punkt + ctx %, Konten-Cards mit 5h-Signal + Provider-Kachel.
**Nicht gebaut (v2 behauptete fälschlich „komplett") → Arbeitspakete:**

- **S1 · Zonen-Header.** `VERBINDUNGEN` / `KI-KONTEN` als Kapitälchen-Label + Zähler
  (heute: `cockpit-conductor-title = Hosts`, de/warp.ftl:37). Zahnrad (SVG,
  Stern-Größe) rechts im VERBINDUNGEN-Header → öffnet SSH-Manager (nie togglen).
  **KI-KONTEN-Header behält die Flotten-Summe** („heute $X" über alle Konten) und
  **die Summe ist selbst der Einstieg** ins Flotten-Pane — das Maximize-**Icon**
  entfällt, die Flotten-**Sicht** nicht.
  > **Korrigiert 2026-07-16.** Ein erster Anlauf entfernte Icon *und* Summe. Das war
  > eine Regression: das **geplante** Konto-Pane (P1) zeigt nur *sein* Konto, also
  > hätten kontenübergreifende Zahlen danach **keinen Ort** mehr gehabt (der Compiler
  > bewies es: `format_cost` wurde ungenutzt). Solange P1 nicht steht, ist das
  > aktuelle Pane (`OpenDashboardPane`) noch das **Flotten-Dashboard** (Aggregat +
  > Conductor + alle Konten-Cards, pane.rs) — die Summe öffnet genau dieses. Regel:
  > „Maximieren-Icon entfernen" heißt Chrome entfernen, **nicht** die dahinterliegende
  > Sicht. Fleet-weite Flächen (Summe, Aggregat, `⏹ stop all`) behalten immer einen
  > Ort — Konto-Panes sind ein *zusätzlicher* Zugriffspfad, kein Ersatz.
- **S2 · „+ Host hinzufügen" entfällt** (panel.rs:632ff) — läuft übers Zahnrad
  (SSH-Manager hat den Add-Flow bereits).
- **S3 · Karten-Feinschliff.** Provider-Wort + Plan-**Badge** als getrennte Slots
  (heute ein flacher String, panel.rs:336); `provider_color` kontrast-adaptiert
  (Light-Palette); bei 0 laufenden Sessions gedämpft „Keine laufenden Sessions"
  statt Zeile verstecken (Terminologie-Konsistenz, panel.rs:357).
- **Session-Identität (Klarstellung statt v2s „nur Branch"):** Anzeige-Kette
  Registry-Name → Branch → Worktree → Verzeichnis (wie gebaut, panel.rs:86-105).
  Der Projektname wird nie inline wiederholt.

---

## 4. Konto-Pane (eines pro Konto) + Sessions-Tabelle + Drive

### 4.1 Pane-Identität — **P1**
Heute: eine globale `selected_account` (model.rs:76), konto-lose `CockpitPaneView`,
Singleton-Dedup (view.rs:6423-6437). Neu: Konto-Key in Wrapper + View, Dedup per
`account.key`, Titel = **Konto-Anzeigename** (Alias→Label; kein Host-Präfix — Konten
sind nicht host-gebunden). Karten-Klick: erneuter Klick **fokussiert** das Pane
(heute: deselektiert, model.rs:147 — fixen). Keine Persistenz nötig
(`LeafContents::Cockpit => false`, app_state.rs:184).

### 4.2 Detail-Card — **P2**
Provider-Kachel · Anzeigename · Mail/Org (hier erlaubt) · Plan · Reset-Timer ·
⋯ (Alias ändern, §6). Meter 5h + Woche (grau, rot ab 85 %), 3-Spalten Heute/5h/Woche
× $/Tokens (Heute = lokale Tagesgrenze, F2).

### 4.3 Sessions-Tabelle — **P3/P4**
- **Fundament: `warpui::elements::Table`** (Sum-Tree-Virtualisierung,
  warpui_core/elements/table/mod.rs:262-542) — kein handgebautes Flex-Grid; bei
  „Hunderten Sessions" Pflicht. Sortierung/Filter/Gruppierung app-seitig (Table ist
  layout-only).
- Spalten: Session · Worktree · **Host** (F5) · Modell · Kontext (Balken + %) ·
  **Heute $** (F4) · Status (Punkt + Wort) · Zuletzt. Zahlen rechtsbündig,
  tabellarisch, einheitliche Gutter.
- Suche (Projekt/Branch/Worktree, live) · Filter-Chips Alle/Wartet/Aktiv/**Idle**
  (F3) · Gruppierung Projekt (Default, F9) umschaltbar Status/Modell, Gruppen
  einklappbar · sortierbare Spaltenköpfe.
- Zeilen-Klick → öffnet/fokussiert die Session (max. 1 Instanz).

### 4.4 Cockpit-Drive: das ⋯ pro Zeile — **P5** · KEINE REGRESSION
Das ⋯ trägt die vollständige Steuerung, **capability-gated (F6)**, destruktives
abgesetzt: Fork · Fork in neuen Worktree · /compact · /clear · Verlauf ·
Review ›(review / als geprüft markieren (F7) / redirect / commit / PR) · Stop · Kill.
Klickziele getrennt (Muster pane.rs:1186: Attach auf Info-Span, Verbs außerhalb).
`⏹ stop all`: **Command-Palette** (Confirm bleibt) — keine Chrome.
**Erst wenn jeder heutige Verb (Gegenprüfung `app/src/cockpit/pane.rs`) seinen Platz
hat, entfällt der Dashboard-Tree im Pane** (Entdopplung zur Sidebar) — **P6**.

---

## 5. Plexing — auf dem BESTEHENDEN Routing (kein Neubau, keine Regression)

**v2-Fehler:** §4 ignorierte, dass Routing existiert und **besser** ist als „meiste
5h-Reserve": `routing.rs:20/48/66` wählt nach dem **bindenden Maximum** aus
5h/Woche/Opus/Sonnet-Fenstern und **depriorisiert arbeitende Konten**; die Spawn-Card
defaultet bereits auf „Freest" und pinnt den Config-Dir (spawn_card.rs:273,
view.rs:17759). Pinning ist provider-spezifisch: `CLAUDE_CONFIG_DIR` **bzw.**
`CODEX_HOME` (types.rs:50/168).

**Soll (X1):** Der bestehende Algorithmus bleibt die Wahrheit. Die Spawn-Card zeigt
die **eine ruhige Auto-Zeile** (Kachel · Anzeigename · Plan · bindendes-Fenster-% ·
„Ändern"); „Ändern" öffnet die Kontoliste mit Auslastung. Skip-Regel = „fast voll"
≥ 85 % im bindenden Fenster (ein Vokabular, §1.2). Spawn-Card-Strings vollständig
DE („Konto", „Freiestes", nicht „Account"/„Freest" — de/warp.ftl:92-109 nachziehen).

---

## 6. Konto-Aliase — auf `instances.json` (kein zweites System)

**v2-Fehler:** verlangte Persistenz „in zaplex-Settings" und erklärte Farbe/Reihenfolge/
Ausblenden zur Zukunft — `overrides.rs:17-27` + model.rs:96 können **heute** Label,
Farbe, Ordnung, Hide über `instances.json`.

**Soll (A1):** Alias = **Label-Override** in `instances.json` (einzige Persistenz).
Bearbeiten über ⋯ im Konto-Pane. Anzeige-Fallback ohne Alias = bisheriges Label
(E-Mail → Org → Verzeichnisname — E-Mail ist optional, types.rs:39). Alias gilt
überall: Card, Pane-Titel, Tabelle, Spawn-Card.

---

## 7. Sprache & Typografie

- Konsequent Deutsch, korrekte Groß-/Kleinschreibung, FTL-Parity-Guard bleibt Gate.
- `42 %` mit schmalem Leerzeichen (heute `42%`, style.rs:445 → in E5 fixen),
  „4 h 00 min".
- **Keine Emojis** — zaplex soll nicht nach Discord aussehen. Das ist die Regel;
  **wie** ein Zeichen technisch gerendert wird (SVG-Icon oder geometrischer Glyph),
  ist *frei*. Maßstab: farblich ruhig, konsistent, guideline-konform (§1).
  > Korrigiert 2026-07-16: eine frühere Fassung schrieb „alle Status-/Disclosure-
  > Zeichen als SVG" vor. Das stand so nie in der Anforderung — es war eine
  > Erfindung der Spec und hätte Umbauten quer durch view.rs/attention_inbox
  > erzwungen, ohne dass jemand etwas davon sieht.
- UI-Verbs im ⋯-Menü deutsch („Verlauf", „Als geprüft markieren", „Fork in neuen
  Worktree").

---

## 8. Bau-Reihenfolge (Gates: cargo check --locked grün · codex 2 Linsen · Selbstverifikation · Parity-Guard)

1. **E1/E2/E5 + S1–S3** — Encoding-Korrekturen + Sidebar wirklich fertigstellen
   (klein, sichtbar, korrigiert die falsche „Schritt 2 komplett"-Meldung).
   **E3 ist gestrichen** (die „alles-SVG"-Regel war erfunden — §1.1/§7); was davon
   sinnvoll war (Chevron als Icon, eine Quelle für Glyph+Farbe), ist in E2 erledigt.
   **E4 existierte nie** — die Nummer war ein Zählfehler der ersten Fassung.
2. **F1 + F2** — die zwei P0-Bestandsbugs (Host-Identity, UTC-Heute).
3. **F3–F9** — Datenpfade & strukturelle Fundamente (einzeln committen).
4. **P1–P6** — Konto-Pane, Tabelle, Drive; Tree-Entdopplung zuletzt.
5. **X1** — Plexing-UI auf bestehendem Routing.
6. **FM** — Modus-Badge (Host bleibt im Tab-Titel) + FM-SFTP-Bug (Daemon-Browse).

**Ein RC-DMG + Daemon-Redeploy am Ende — nur auf explizite User-Freigabe.**
(Autocomplete-Fix `1e484697` fährt im selben Zyklus mit.)

## 9. Abnahme (Definition of Done je Paket)

Code gebaut + Abnahmekriterium des Pakets erfüllt + keine der §1-Regeln verletzt +
kein heute vorhandener Verb/Flow verloren (Gegenprüfung pane.rs) + i18n-Parity grün.
