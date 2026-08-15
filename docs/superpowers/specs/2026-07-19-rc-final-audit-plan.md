# RC-Final-Audit & Plan — zaplex für externe Test-User (2026-07-19)

Quelle: 6 unabhängige codex-Linsen (Sol/xhigh, read-only) + eigene
Code-Verifikation. Ziel: **ein zaplex-Build, den man echten Test-Usern in die
Hand drücken kann** — nicht die volle Premium-Vision, sondern ein Build, der
**nicht abstürzt, keine Daten zerstört, nicht still lügt und in Woche-1-Flows
als *ein* kohärentes deutsches Produkt liest.**

Die zentrale Erkenntnis der Konzept-Linse zuerst, weil sie den Owner-Eindruck
am ehrlichsten erklärt: **Die integrierte app-weite „Spine" fehlt.** Das
Cockpit ist ein Toolbelt-Tab *neben* SSH-Manager/Files/Chats, kein integriertes
Werkzeug. Das ist ein **architektonischer** Rückstand (Kategorie 2, unten
Tier 3) — er lässt sich in dieser Woche nicht seriös schließen, und so zu tun
wäre der Fehler der letzten Woche. Der Test-Drop-Gate ist ein anderer: nicht
„erreicht die Vision", sondern „ist es sicher und ehrlich in der Hand eines
Fremden".

Deshalb die Tier-Trennung. Tier 0–2 = **diese Woche, Test-Drop-Gate**.
Tier 3 = **Post-Test-Roadmap**, ausdrücklich nicht dieses Gate.

---

## TIER 0 — Datenverlust / Absturz. Muss weg, kompromisslos.

Ein Test-User, der hier reinläuft, verliert Daten oder sieht die App
crashen. Das rangiert über allem.

### T0.1 Lokaler FM zerstört Dateien (Doppelpfad-Klasse, lokales Backend lahmt)
Das Remote-SFTP-Backend arbeitet sicher (Temp+atomarer Rename, `overwrite:
false`); das **lokale** Backend nicht — und genau das benutzt ein Test-User auf
dem eigenen Rechner. Alle gegen Code verifiziert:
- **C3** Umbenennen: `fs::rename` überschreibt ein existierendes Ziel
  kommentarlos ([sftp_backend.rs:329](app/src/sftp_manager/sftp_backend.rs:329)).
- **C4** Kopieren/Upload: `fs::copy` schreibt direkt auf den Zielpfad — Fehler
  mitten drin oder Selbst-Kopie vernichtet den alten Inhalt
  ([sftp_backend.rs:378](app/src/sftp_manager/sftp_backend.rs:378)).
- **C1** Lokal→remote Directory-MOVE plant nur reguläre Dateien/Dirs, schluckt
  `create_dir`-Fehler, löscht danach den **ganzen** Quellbaum → ausgelassene
  Symlinks/Sockets/leere Dirs sind dann weg ([browser.rs:1994/2041/3164](app/src/sftp_manager/browser.rs:1994)).
- **C5** Overwrite eines Symlinks-auf-Verzeichnis kann dessen Ziel rekursiv
  löschen (Remote nutzt `stat` statt `lstat`, [sftp_backend.rs:127](app/src/sftp_manager/sftp_backend.rs:127)).
- **C2** Path-Traversal: rekursiver Download fällt bei `strip_prefix`-Fehler auf
  ungeprüften Remote-Pfad → `..`/absolut kann außerhalb des Zielordners
  schreiben ([browser.rs:3194](app/src/sftp_manager/browser.rs:3194)).

**Fix-Richtung:** Lokales Backend an das sichere Remote-Muster angleichen —
Copy über Temp+Rename, Rename mit Konfliktprüfung (`overwrite:false`-Semantik),
Move rekursiv über **alle** Eintragstypen und Quelle erst nach verifizierter
Kopie löschen, Download-Ziel per validiertem `strip_prefix`. Das ist wieder
„das eine Backend macht es richtig, das andere zieht nach".

### T0.2 Startup-/Interaktions-Panics (keymap-Geschwister; DEV-Profil = live)
Test-DMGs laufen im **dev-Profil**, also sind `debug_assertions` bei Test-Usern
scharf, und CI führt sie nie aus (genau die Falle des `cmd-shift-o`-Crashes).
- **E** Leerer Branch `PaneBranchTemplate { panes: [] }` → `nodes.len() - 1`
  Underflow-Panic ([tree.rs:247](app/src/pane_group/tree.rs:247)); im dev-DMG
  ein Crash beim Öffnen einer solchen Launch-Config.
- Der Lens-A-P0-Nachlauf ergänzt weitere Startup-Panic-Kandidaten (stale
  `~/.zaplex`-State aus älteren Versionen, Keymap/Theme/Settings-Parsing) —
  Liste wird eingearbeitet.

**Fix-Richtung:** Jeden Startup-Parser gegen leere/stale/hand-editierte
User-Daten härten (kein `len()-1` ohne Guard, kein `unwrap` auf
User-Einfluss). Diese Klasse ist die einzige, die *jeden* anderen Fix wertlos
macht, wenn sie zuschlägt.

### T0.3 Bootstrap-Dump ist nur in 1 von 3 bash-rcfiles gefixt
Mein „Wurzel-Fix" (`stty raw -echo` + PS2) sitzt nur in `bash_init_shell.sh`.
Verifiziert offen:
- `bash_init_subshell.sh:3` (nested `bash`) — `stty raw` ohne `-echo`.
- `bash_body.sh:1032` (eingebetteter rcfile beim Nested-Spawn) — dito.

**Fix-Richtung:** `-echo` + PS2-Behandlung in alle drei Pfade; Regressionstest
auf die Subshell-Route erweitern (der jetzige deckt nur `bash_init_shell`).

---

## TIER 1 — Kaputte / tote Session-Zustände. Session unbrauchbar oder falsches Ziel.

### T1.1 Version-Mismatch hinterlässt tote Daemon-Tab (Lücke in meiner Selbstheilung)
Classic-SSH-Fallback feuert nur bei Preflight/Install-Fehler, **nicht** beim
späten Handshake-Mismatch — dort bleibt eine tote Daemon-Tab statt der
funktionierenden Classic-Tab (**B4**, [view.rs:7364](app/src/workspace/view.rs:7364),
[manager.rs:850](crates/remote_server/src/manager.rs:850)). Der Enforcement-Zweig,
den ich baute, meldet den Fehler — aber niemand fängt ihn auf.

### T1.2 Stiller Freeze nach Reconnect-Erschöpfung
Der Daemon-EventLoop lässt `SessionDisconnected` in `_` fallen (**B3**,
[event_loop.rs:158](app/src/terminal/daemon_tty/event_loop.rs:158)); der lokale
Pfad beendet den Model-State sauber. Folge: Nach permanentem Netzverlust friert
der Bildschirm lautlos ein, Eingaben werden gepuffert/verworfen.

### T1.3 Adoptierte Session bleibt dauerhaft „not bootstrapped"
Adoption replayt nur den 4-MiB-Output-Ring ab 0; ist der initiale DCS evictet,
armen History/Autocomplete/Block-Parsing/Command-Ausführung **nie** (**B2**,
[event_loop.rs:226](app/src/terminal/daemon_tty/event_loop.rs:226)).

### T1.4 Remote-Aktion fällt in den lokalen Pfad → falscher Host/Checkout
- **B1** Ctrl/Cmd-Klick auf eine Remote-Datei kann in den lokalen Open-Path
  fallen; existiert derselbe Pfad lokal, wird die falsche Datei geöffnet/editiert.
- **D1** Fork/Compact/Clear einer **Remote**-Session arbeitet lokal am
  Remote-CWD ([view.rs:4239](app/src/workspace/view.rs:4239)) → falscher lokaler
  Checkout adressiert.

### T1.0 SSH-„Test connection" scheitert mit dem eingegebenen Passwort (erste Aktion, wirkt kaputt)
Die **allererste** Aktion eines Test-Users ist: Host anlegen + „Test". Der
Test-Pfad kann das gerade eingegebene Passwort/die Passphrase auf macOS nicht
nutzen — ein gültiges Passwort meldet Offline/Timeout, ein verschlüsselter Key
scheitert, wenn er nicht schon im ssh-agent entsperrt ist ([ssh_command.rs:75/153](crates/warp_ssh_manager/src/ssh_command.rs:75)).
Folge: Das SSH-Setup wirkt komplett kaputt, bevor der User überhaupt verbindet.
Dieselbe Auth-Doppelpfad-Klasse wie beim FM (den ich schon gefixt habe) — der
Test-Pfad zieht nach.

### T1.5 FM-Refresh-Race + stale Indizes (Datenintegrität)
- **E5/A25** Überlappende Refreshes installieren die falsche Listung: Header
  sagt B, Zeilen sind A; Open/Download/Delete trifft A
  ([browser.rs:989](app/src/sftp_manager/browser.rs:989)).
- **E6** Kontextmenü/Save-As halten rohe `entry_index` über Refreshes → Aktion
  trifft die Datei, die jetzt an A's altem Index steht.
- **E7** Row-Klick/Refresh löschen Marks (Legacy-Single-Select-Verhalten) →
  Batch-F5/F6/F8 trifft nur die geklickte Zeile. Regression aus dem Mark-Umbau.

---

## TIER 2 — Peinlichkeiten & Vertrauensverlust. Blamiert, „lügt", geerbtes Fremd-Branding.

### T2.1 Tote Buttons & leere/Upstream-URLs (geerbtes Warp leckt durch)
- ~10 Settings-Info-Icons dispatchen `OpenUrl("")` ([features_page.rs:4903](app/src/settings_view/features_page.rs:4903)).
- Resource-Center: zwei „View documentation" mit leerer URL; „How Zaplex uses
  Zaplex" öffnet `warp.dev/blog/how-warp-uses-warp` ([sections.rs:59](app/src/resource_center/sections.rs:59)).
- Autoupdate-Failure-„More info" → `""` ([view.rs:12842](app/src/workspace/view.rs:12842)).
- Shell-Terminated-Banner: „File issue" → `zerx-lab/warp` ([shell_terminated_banner.rs:213](app/src/terminal/view/shell_terminated_banner.rs:213)).
- **Zaplex-Launch-Modal**: primäre „Visit repo"-CTA → `warpdotdev/warp` — wird
  onboarding-Testern prominent gezeigt ([zap_launch_modal/view.rs:27](app/src/workspace/view/zap_launch_modal/view.rs:27)).
- **Help/Feedback/Docs/Slack/Privacy** (App-Menü): tot oder → `zerx-lab/warp`;
  „GitHub Issues"/„Send Feedback" öffnen das Upstream-Repo ([links.rs:6](app/src/util/links.rs:6),
  [app_menus.rs:842](app/src/app_menus.rs:842)). **Kritisch für ein Test-Programm:**
  genau die Feedback-Wege, die Test-User brauchen, führen ins Leere oder falsch.
- Agent-Inbox-Onboarding „Learn more" → leere URL ([hoa_onboarding_flow.rs:474](app/src/workspace/hoa_onboarding/hoa_onboarding_flow.rs:474)).

**Fix-Richtung:** Jede leere/Upstream-URL entweder auf ein echtes Ziel setzen
oder den Button entfernen. Kein Fremd-Branding in der Test-User-Hand. Die
Feedback-Route (Issues/Feedback) auf byte5ai/zaplex richten — sonst kann das
Test-Programm sein eigenes Ziel nicht erfüllen.

### T2.2 Irreversibles Löschen ohne Rückfrage
Host/Ordner-Löschen im Kontextmenü entfernt DB-Node + 3 Keychain-Secrets sofort,
ohne Bestätigung/Undo ([panel.rs:700](app/src/ssh_manager/panel.rs:700)). Ein
Fehlklick zerstört einen Host oder Ordner-Subtree.

### T2.3 Cockpit zeigt „lädt"/„fehlgeschlagen" als Faktum „leer"
Cockpit startet mit leerem Snapshot, scannt async, Fehler werden nur geloggt;
reale Accounts/Sessions erscheinen als „Keine … gefunden", ununterscheidbar von
echtem Leer (**A28/D-P0**, [model.rs:121](app/src/cockpit/model.rs:121)). Lokale
I/O-/Parse-Fehler werden zu leeren Collections → **freest-account-Routing kann
das falsche Konto wählen** ([claude.rs:210](crates/zaplex_cockpit/src/claude.rs:210)).

**Fix-Richtung:** Loading/Error-Zustand im Snapshot halten; ungültige Snapshots
vom Routing ausschließen; echter Zero-Zustand bekommt eine Aktion
(Login/Install/Rescan) statt einer Sackgasse.

### T2.4 Gemischte Sprache in Woche-1-Flows
Der DE-Katalog ist da, aber viel geerbte UI ist hart englisch. **Nicht** die
ganze App — nur die Woche-1-Flächen schließen:
- **FM** (du hast ihn getestet): Kontextmenü, Dialoge (Copy/Move/Delete/Rename/
  Overwrite/Details), Transfer-/Connection-Toasts, `SFTP:`-Titel.
- **SSH-Manager**: Test/Progress/Error-Text ([server_view.rs:1000](app/src/ssh_manager/server_view.rs:1000)).
- **Connect-Moment**: „Connecting to …" hart englisch ([view.rs:7150](app/src/workspace/view.rs:7150)).
Tiefe Settings (Agent-Provider, MCP, Drive) = dokumentierte Bekannt-Limitation.

---

## TIER 3 — Konzept-Tiefe. Ehrlich: NICHT dieses Gate. Post-Test-Roadmap.

Real, aber architektonisch/feature-tief — Wochen, nicht Tage:
1. **Integrierte Spine** — das Cockpit als Rückgrat, nicht als Tab neben
   Toolbelt. Der #1-Konzeptbruch.
2. Plan-first-Aktivitätsfläche (genehmigt, existiert nicht).
3. Alternative Gruppierungen (Status/Modell) + Session-Peek.
4. Remote-Account-Aggregate im Cockpit (D3/D4: Remote-only-Accounts ohne Card).
5. Modal-Chrome-Vereinheitlichung M1/M4/M5/M6 (kosmetisch).
6. FM-Tiefe: Symlink-als-Datei behandeln, ProxyJump im SFTP, chmod/mtime beim
   Copy erhalten (C7), großer F5-Copy async mit Progress (C8).

Diese Liste wird der Test-User-Feedback-Backlog. Sie hier zu benennen ist der
Punkt: Der Test-Drop misst nicht sie.

---

## Reihenfolge & Prozess
1. **Tier 0** zuerst (Datenverlust + Crash — jeder Fix ohne diese ist wertlos).
2. **Tier 1**, dann **Tier 2**.
3. Jeder Fix: codex-reviewt (Sol/xhigh, 2 Linsen) VOR Commit; wo Verhalten
   prüfbar ist, ein Test, der **ohne** den Fix rot wird.
4. **Ein** DMG erst, wenn Tier 0–2 grün — und nur auf ausdrückliches OK.
5. Test-User-DMG muss als echtes GitHub-Release existieren (Auto-Update zieht
   aus byte5ai/zaplex-Releases; Bug „revertet auf Upstream" ist bereits gefixt).
