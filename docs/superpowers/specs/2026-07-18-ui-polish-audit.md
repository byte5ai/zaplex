# UI-Polish-Audit — Cockpit/Sidebar/Daemon-Tab (2026-07-18)

Anlass: RC-Abnahme v0.rc2. Urteil des Users: „nicht professionell, nicht einer
Premium-Terminal-App würdig, wie semi-professionelle Shareware". Dieses Audit
benennt JEDEN gefundenen Verstoß gegen das eigene System (Spec v3 +
Sidebar-Spec §2.1 + vorhandene Komponenten) mit Ist/Soll und Beleg.

**Diagnose in einem Satz:** Die Defekt-Fixes der letzten Runden waren lokal
korrekt, aber niemand hat die Flächen gegen EIN verbindliches Bauteil- und
Interaktions-Vokabular abgenommen — es existieren sogar die richtigen
Komponenten (`SearchBar`, `EditorView::set_placeholder_text`, `zone_card`),
sie wurden nur nicht überall benutzt.

---

## P0 — sichtbare Verstöße, klein und eindeutig (ein Durchgang)

### P0.1 Suchfeld der Sessions-Tabelle ist ein nacktes Rechteck
- **Ist:** `pane.rs:1538` baut ein `text_input` ohne Placeholder, ohne Icon,
  ohne Affordanz — ein leeres Rechteck.
- **System-Beleg:** Es gibt `app/src/search_bar.rs` (SearchBar mit Lupe,
  Border, Styles) UND `EditorView::set_placeholder_text` (i18n-fähig, genutzt
  z. B. `enum_creation_dialog.rs:158`).
- **Soll:** Lupe + lokalisierter Placeholder („Sessions filtern…" /
  "Filter sessions…"), Höhe und Radius im Satz mit den Filter-Chips.
  *(Fokus-Akzent-Rahmen: nicht Teil dieses Passes — die SearchBar-Komponente
  stylt ihren Fokus selbst; als Follow-up erfasst.)*

### P0.2 Host-Zeilen-Hover ist winzig und eine eigene Grammatik
- **Ist:** `panel.rs:569-585` — Hover-Fläche ist NUR das Label, 3 px
  horizontal, 0 px vertikal gepolstert: ein texthoher Mini-Kasten. Die
  Konto-Cards direkt darunter hovern als volle Fläche.
- **Soll:** Der Glance-Span der Host-Zeile (Dot + Name + Fold-Count) ist EIN
  volle-Breite-Hover-Target mit derselben Row-Geometrie wie Session-Zeilen
  (v-Padding, Control-Radius, `fg_overlay_1`); ★ und ⋯ stehen DANEBEN —
  außerhalb des Klick-Targets, exakt die Session-Row-Grammatik (Glance
  klickt, Trailing-Verben handeln). Session-Zeilen bekommen dieselbe
  `hover_row`-Fläche (ihr Mouse-State wurde bisher ebenfalls verworfen).

### P0.3 Meter-Labels im Konto-Detail sind abgeschnitten („5-St…", „Woc…")
- **Ist:** `heat_bar` reserviert 24 px fürs Label (`pane.rs:986`); das
  Detail-Card übergibt die langen Spaltentitel (`pane.rs:2133-2151`).
- **Soll:** Detail nutzt dieselben Kurzlabels wie das Flotten-Card („5h"/„wk")
  — Meter-Labels sind Vokabeln, keine Sätze; die langen Titel gehören der
  Zahlen-Matrix. Keine Truncation in Labels, nirgends.

### P0.4 Nackte Reset-Zeiten ohne Kontext („3h40m", „40m")
- **Ist:** `pane.rs:2140-2155` rendert `format_reset` roh als eigene Zeile.
- **Soll:** Das Flotten-Card hat das Format bereits: `5h ↻ 3h40m · wk ↻ 40m`
  (`pane.rs:2270-2277`). Eine Quelle, beide Flächen.

### P0.5 Daemon-Tab ist bis zum Connect titellos
- **Ist:** `view.rs` übergibt `custom_tab_title: None` — der Tab heißt
  sekundenlang „" (Screenshots 2/4). Durch den Sofort-Tab-Fix jetzt sichtbarer.
- **Soll:** Tab trägt ab Sekunde 0 den Hostnamen (`add_tab_with_pane_layout`
  hat den Parameter bereits).

### P0.6 ~-Schätzwerte ohne Legende im Konto-Pane
- **Ist:** Die `~`-Erklärung (`cockpit-pane-provenance-legend`) rendert nur im
  Flotten-Zweig (HEAD `pane.rs:2621-2633`); das Konto-Pane zeigt „~0 %"
  unerklärt.
- **Soll:** Legende erscheint überall dort, wo ein `~` sichtbar ist.

### P0.7 Tabellen-Leerzustand
- **Ist:** „Keine Sessions für diesen Filter." klebt links oben im Card.
- **Soll:** Zentriert, muted, mit etwas vertikalem Raum — Leerzustände sind
  gestaltete Zustände, keine Debug-Ausgaben.

## P1 — Systematik erzwingen (gleicher Durchgang, mehr Sorgfalt)

### P1.1 Radius-/Padding-Wildwuchs im Cockpit
- **Ist:** Radius 3.0 (Alias-Editor, Provider-Kachel), 4.0 (Suchfeld,
  Host-Hover), 5.0 (Plan-Badge), 8.0 (Alt-Cards), 12.0 (`zone_card`) —
  fünf Radien in einem Modul.
- **Soll:** Ein Satz Konstanten in `style.rs`: Zone-Card = `CARD_RADIUS` (12),
  Block = `BLOCK_RADIUS` (8), Controls/Chips/Badges = `CONTROL_RADIUS` (4).
  Zwei benannte Ausnahmen außerhalb der Skala: Farb-**Swatches** (5/10/12 px
  Provider-Kacheln) skalieren mit ihrer eigenen Größe, und **Meter-Tracks**
  sind Pillen (Radius = halbe Track-Höhe).

### P1.2 Heat-Farb-Audit (Spec v3 E1)
- **Memory-Behauptung:** Critical (#F97316) kollidiert mit Attention-Amber.
  `utilisation_coloru` (grau→rot ab 85 %) ist das Soll-System für Meter.
- **Auftrag:** Verifizieren, wo die 5-Band-Palette (`heat_coloru_on`) noch
  Auslastung färbt statt nur Status; jede solche Stelle auf
  `utilisation_coloru` umstellen. (Nur verifizierte Stellen anfassen.)

### P1.3 Truncation-Regeln der Tabelle
- **Ist:** Modell-Spalte schneidet hart ab („codex-auto-revie…"), ohne Tooltip.
- **Soll:** Spaltenraum statt Tooltip (Modell-Spalte Flex 1.0 → 1.4): die
  vorhandenen Tooltip-Bausteine sind Button-gebunden, ein Zell-Tooltip wäre
  ein neues Primitive — siehe „Verifiziert"-Abschnitt.

### P1.4 „Connecting…"-Moment im Block-UI
- **Ist:** Die Progress-Zeile aus dem Sofort-Tab-Fix rendert als Terminal-Zeile
  zwischen Blocks; der sichtbare Zustand ist ein nacktes „Starting …".
- **Soll:** Prüfen, wie die Zeile im Block-UI ankommt; Zustand lesbar machen
  (Host + „Verbinde…"), OHNE den Connection-Pfad selbst anzufassen.

## P0-FM — Filemanager (Nachtrag 2026-07-19, User: „sieht FURCHTBAR aus")

### FM.1 Cursor-Zeile ist ein greller Accent-Slab mit invertiertem Text
- **Ist:** `file_list.rs:50` — `is_cursor` füllt die ganze Zeile mit
  `theme.accent()` und invertiert die Textfarben (MC-Look). Im dunklen Theme
  ein leuchtender Fremdkörper.
- **Soll:** Cursor = Accent-**Tint** (Accent mit niedriger Deckkraft), Text
  normal; Multi-Selektion = `fg_overlay_2`; Hover = `fg_overlay_1` (der
  Mouse-State wird heute verworfen — `|_|`). Cursor gewinnt über Selektion,
  Selektion über Hover. Semantik (Cursor ≠ Selektion) bleibt MC-treu.

### FM.2 Spalten wandern mit der Namenslänge
- **Ist:** `file_list.rs:129` — die Zeilen-Flex hat kein
  `MainAxisSize::Max`, der Name füllt nicht: Size/Modified kleben am Namen
  und stehen in jeder Zeile woanders.
- **Soll:** Zeile auf `Max`, Name flext, Size/Modified sind feste rechte
  Spalten (Size rechtsbündig). Header bekommt den 16+8-px-Icon-Spacer, den
  die Zeilen haben (`file_list.rs:210` fälscht ihn heute mit spacing 24 —
  der Name-Header steht 24 px neben den Namen).

### FM.3 Chrome-Strings hart englisch in der deutschen App
- **Ist:** „Name/Size/Modified" (`file_list.rs:175-198`), „Search files..."
  (`browser.rs:453`), F-Leisten-Captions „View/Edit/Copy/…" (`browser.rs:58`).
- **Soll:** ftl-Keys de/en für die sichtbare Chrome (Spalten, Placeholder,
  F-Leiste). Tiefere FM-Strings (Dialoge, Fehler) = eigener Follow-up, hier
  nur erfasst.

### FM.4 F-Tasten-Leiste im Norton-Commander-Look
- **Ist:** `browser.rs:2375` — rohe „F3 View"-Paare, Accent-Keys, engl.
- **Soll:** Ruhige Keycap-Leiste im Stil der App-Key-Hints: Key als kleine
  Chip-Kachel (surface_2, Hairline, Control-Radius), Caption muted, oben
  eine Hairline zur Liste. Funktionalität und Tasten unverändert.

## Verifiziert, KEIN Handlungsbedarf (Stand 2026-07-19)
- **E1 Heat-Farben:** `ctx_pct_element` und alle Meter laufen über
  `utilisation_coloru` (grau → echtes Rot ≥ 85 %, style.rs:282/488); der
  einzige `heat_coloru_on`-Aufrufer außerhalb style.rs färbt den
  WORKING-Status-Punkt (pane.rs:2209) — Status, nicht Auslastung. Die
  Memory-Warnung „E1-Farbfehler im gebauten Code" ist veraltet.
- **P1.3 Tooltip:** Tooltip-Bausteine existieren (`ui_components::tooltip`,
  `icon_verb_button_tooltip`), sind aber Button-gebunden — einen generischen
  Zell-Tooltip-Wrapper gibt es nicht. Lösung in diesem Pass über Spaltenraum
  (Modell-Spalte 1.0 → 1.4), kein neues UI-Primitive nebenbei.
- **P0.5 Tab-Titel:** `custom_tab_title` ist sticky (`set_title` →
  `custom_title`, pane_group/mod.rs:4250) und würde die dynamische
  „devhost:~"-Mechanik einfrieren. Stattdessen schreibt der
  Daemon-Eventloop beim Start eine OSC-0-Titelsequenz mit dem Hostnamen
  durch den normalen Parser (Control-Zeichen im Label werden gestrippt) —
  sofort benannt, später vom Titel-Mechanismus der laufenden Session
  (precmd-Metadaten → Titel-Events) natürlich überschrieben.

## P2 — eigenes Arbeitspaket (nicht in diesem Durchgang)

### P2.1 F9: Projekt = Repo, Worktree = Attribut
Worktrees zersplittern die Projektgruppen in Sidebar und Tabelle (Spec v3 F9,
Memory). Strukturelle Änderung am Join — eigener Branch-Abschnitt, eigenes
Review.

---

## Abnahme-Definition für diesen Polish-Pass
1. Jede sichtbare Cockpit-/Sidebar-Fläche benutzt ausschließlich Bausteine aus
   `style.rs`/bestehenden Komponenten (SearchBar, zone_card, verb_buttons).
2. Kein abgeschnittenes Label, kein leeres Eingabefeld ohne Placeholder, kein
   unbenannter Tab, kein unerklärtes `~`.
3. Hover/Fokus: EINE Grammatik (volle Zeile/Card, `fg_overlay_1`,
   Control-Radius) — nachweisbar identisch zwischen Hosts, Sessions, Cards.
4. codex-Review (Korrektheit + Faktentreue) VOR Commit; pane.rs/panel.rs haben
   keinen Test-Harnisch.
5. EIN DMG erst nach User-Freigabe.
