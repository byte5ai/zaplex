# Codebase-Audit: historischer Wiederaufnahmeplan vom 10. September 2026

Erfassungsstand: 2026-09-10 · Historisch eingeordnet und ins Repository übernommen: 2026-09-17

Tracking: [Issue #245](https://github.com/byte5ai/zaplex/issues/245)

## Einordnung bei der Übernahme

Dieses Dokument bewahrt den bisher uncommitteten Entwurf vom 10. September.
Alle Mengen, offenen Punkte, Dateipfade und Empfehlungen unten beziehen sich
ausschließlich auf den damaligen Vergleich der beiden genannten Commits. Sie
sind weder ein aktueller Abdeckungsbericht für `main` noch der Nachweis eines
vollständigen Codebase-Audits. Einzelne genannte Dateien können inzwischen
geändert oder entfernt worden sein.

Bei der Übernahme am 17. September wurden die historischen Ergebnisse nicht
neu erhoben. Spätere Audit-Arbeiten und ihr Status sind anhand der Evidenz und
der verknüpften Issues in Issue #245 zu bewerten; die damaligen Restmengen
dürfen nicht als heutige offene Arbeit übernommen werden.

## Historische Kurzfassung

Der Audit war am Erfassungsstand **nicht vollständig**. Die ursprüngliche
Abdeckung bezog sich auf `f845a15038138e9800885c15a844435b386c83ee` (v1.0.6);
der damalige Wiederaufnahme-Stand verglich sie mit `7329496939f58cc4f2d21f01fdd06998a7c217c6` (v1.0.10).
Zwischen beiden Ständen liegen 271 Commits und 318 geänderte Dateien
(`+25.667/-5.301` Zeilen). Deshalb dürfen die bisherigen 40 gelesenen Einheiten
nicht pauschal als aktuelle Abdeckung gezählt werden.

## Abdeckungsmatrix am 10. September

| Status am Zielstand | Einheiten | Zeilen im alten Inventar | Tier-Verteilung | Evidenz / Konsequenz |
|---|---:|---:|---|---|
| Unverändert seit dem vollständigen Review | 15 | 43.659 | 15 × T3 | Kein Textdiff zum Audit-Stand; die alte Zeilenabdeckung bleibt erhalten. |
| Vollständig gelesen, danach geändert | 25 | 121.145 | 6 × T1, 1 × T2, 18 × T3 | 89 Dateien, `+5.186/-2.431`; erst nach Review des Deltas wieder aktuell vollständig. |
| Noch nie vollständig gelesen | 316 | 1.155.882 | 108 × T1, 205 × T2, 3 × T3 | Muss am neu inventarisierten Zielstand vollständig gelesen werden. |
| Geändert, aber im alten Chunk-Inventar nicht enthalten | 53 Dateien | — | 40 neu, 13 bereits vorhanden | Vor dem nächsten Finder-Lauf neuen oder bestehenden Einheiten zuordnen. |

Die Mengenprüfung der alten Artefakte ist konsistent: 356 Einheiten insgesamt
= 40 Ergebnisdateien für gelesene Einheiten + 316 Einträge in
`remaining_chunks.json`. Für alle 40 gelesenen Einheiten ist
`coverage.files_not_fully_read` leer. Die acht Querschnitts-Audits weisen
dagegen bewusst nur regionsweise gelesene Dateien aus und zählen deshalb nicht
als vollständige Einheiten.

### Artefaktkonsistenz

| Prüfung | Ergebnis |
|---|---|
| JSON-Lesbarkeit | 103 von 103 JSON-Artefakten sind syntaktisch lesbar. |
| Finder-Ergebnisse | 50 Dateien: 40 Einheiten, 8 Querschnitts-Audits und 2 mechanische Läufe. |
| Finder → Verifikation | Die drei Dedupe-Sätze mit 59, 39 und 25 Befunden entsprechen den drei verifizierten Sätzen; zusammen 123 Verdicts in 25 Verifier-Ausgaben. |
| Verifikation → Issues | 80 finale Issue-Pakete und 80 gespeicherte Erstellungsresultate; Batch 2 und 3 enthalten zusätzlich 9 bzw. 4 als Kommentar behandelte Einträge. |
| Batch-Protokoll | `rounds.json` enthält genau die drei in Issue #245 beschriebenen Batches. |

Diese strukturelle Konsistenz belegt die damalige Verarbeitungskette, nicht die
Aktualität der inhaltlichen Befunde am neuen Zielstand.

### Am damaligen Zielstand unveränderte, weiterhin abgedeckte Einheiten

`c245`, `c247`, `c254`, `c271`, `c290`, `c302`, `c304`, `c313`, `c321`,
`c322`, `c325`, `c338`, `n016`, `n017`, `n018`

Diese Aussage betrifft nur die zugeordneten Quelldateien. Sie ist kein Beleg,
dass Änderungen in aufgerufenen Modulen keine neue Querschnittswirkung haben.

### Bereits gelesene Einheiten mit offenem Delta

| Einheit | Tier | Bereich | Geänderte Dateien | Delta |
|---|---:|---|---:|---:|
| `c063` | 1 | `app/src/cockpit` | 6 | `+152/-261` |
| `c119` | 1 | `app/src/remote_server` | 1 | `+499/-133` |
| `c121` | 1 | `app/src/remote_server` | 3 | `+230/-143` |
| `c330` | 1 | `crates/zaplex_cockpit` | 7 | `+749/-143` |
| `c331` | 1 | `crates/zaplex_cockpit` | 11 | `+696/-301` |
| `c332` | 1 | `crates/zaplex_cockpit` | 13 | `+661/-417` |
| `c327` | 2 | `crates/warpui_extras + crates/watcher` | 1 | `+46/-43` |
| `c250` | 3 | `crates/editor` | 1 | `+3/-5` |
| `c260` | 3 | `crates/field_mask + crates/fuzzy_match + crates/handlebars` | 2 | `+131/-208` |
| `c274` | 3 | `crates/markdown_parser + crates/natural_language_detection` | 1 | `+36/-36` |
| `c286` | 3 | `crates/simple_logger` | 2 | `+202/-76` |
| `c288` | 3 | `crates/string-offset + crates/sum_tree` | 2 | `+111/-6` |
| `c291` | 3 | `crates/vim` | 4 | `+47/-10` |
| `c293` | 3 | `crates/voice_input` | 1 | `+360/-138` |
| `c314` | 3 | `crates/warpui` | 9 | `+72/-85` |
| `c315` | 3 | `crates/warpui` | 2 | `+47/-26` |
| `c318` | 3 | `crates/warpui_core` | 4 | `+85/-17` |
| `c319` | 3 | `crates/warpui_core` | 3 | `+112/-8` |
| `c320` | 3 | `crates/warpui_core` | 2 | `+80/-1` |
| `c323` | 3 | `crates/warpui_core` | 1 | `+4/-1` |
| `c324` | 3 | `crates/warpui_core` | 1 | `+16/-0` |
| `c326` | 3 | `crates/warpui_extras` | 7 | `+533/-151` |
| `c335` | 3 | `lib/rust-genai` | 2 | `+167/-130` |
| `c336` | 3 | `lib/rust-genai` | 2 | `+144/-92` |
| `c337` | 3 | `lib/rust-genai` | 1 | `+3/-0` |

### Dateien ohne Zuordnung im alten Inventar

Die 40 neuen Dateien müssen einer aktuellen Einheit zugeordnet werden. Die 13
bereits vorhandenen, aber nicht inventarisierten Dateien zeigen zusätzlich,
dass das alte Inventar nicht nur wegen neuer Dateien unvollständig ist.

<details>
<summary>40 neue Dateien</summary>

- `app/build_git.rs`
- `app/src/ai/agent_providers/chat_stream_tests.rs`
- `app/src/ai/agent_providers/content_tool_calls.rs`
- `app/src/ai/agent_providers/content_tool_calls_tests.rs`
- `app/src/ai/subscription_agent/presentation.rs`
- `app/src/ai/subscription_agent/presentation_tests.rs`
- `app/src/ai/subscription_agent/process_lifecycle_tests.rs`
- `app/src/remote_server/embedded_tests.rs`
- `app/src/remote_server/ssh_transport_tests.rs`
- `app/src/remote_server/unix/proxy_tests.rs`
- `app/src/settings_view/mcp_servers/edit_page_tests.rs`
- `app/src/settings_view/mod_tests.rs`
- `app/src/ssh_manager/secret_injector_tests.rs`
- `app/src/terminal/grid_renderer/unicode_placeholder.rs`
- `app/src/terminal/local_tty/event_loop_tests.rs`
- `app/src/terminal/local_tty/unix_tests.rs`
- `app/src/terminal/model/kitty_test.rs`
- `app/src/terminal/model/kitty_tests.rs`
- `app/src/terminal/model/session/command_executor/shared_tests.rs`
- `app/src/terminal/ssh/install_tmux_tests.rs`
- `app/src/terminal/view/agent_view_tests.rs`
- `app/src/terminal/view/ssh_file_upload_tests.rs`
- `app/tests/build_git_tests.rs`
- `crates/warpui/src/platform/mac/ime_abi_tests.rs`
- `crates/warpui/src/platform/mac/window_tests.rs`
- `crates/watcher/src/lib_tests.rs`
- `crates/zap_sftp/src/error_tests.rs`
- `crates/zap_sftp/src/session_tests.rs`
- `crates/zap_sftp/src/sftp_tests.rs`
- `crates/zaplex_cockpit/src/account_key.rs`
- `crates/zaplex_cockpit/src/lib_tests.rs`
- `crates/zaplex_cockpit/src/routing_tests.rs`
- `crates/zaplex_cockpit/src/snapshot_health_tests.rs`
- `docs/ui/agent-conversation-lifecycle.html`
- `lib/rust-genai/src/adapter/adapters/anthropic/streamer_tests.rs`
- `lib/rust-genai/src/adapter/adapters/gemini/adapter_impl_tests.rs`
- `lib/rust-genai/src/adapter/adapters/openai/streamer_tests.rs`
- `script/test-macos-bundle`
- `specs/gh-153/PRODUCT.md`
- `specs/gh-153/TECH.md`

</details>

<details>
<summary>13 geänderte Bestandsdateien ohne alte Zuordnung</summary>

- `Cargo.lock`
- `app/i18n/de/warp.ftl`
- `app/i18n/en/warp.ftl`
- `crates/remote_server/proto/remote_server.proto`
- `crates/remote_server/src/install_remote_server.sh`
- `crates/warpui/src/platform/mac/objc/host_view.m`
- `crates/warpui/src/platform/mac/objc/window.m`
- `crates/warpui/src/platform/mac/rendering/metal/shaders/shader_types.h`
- `crates/warpui/src/platform/mac/rendering/metal/shaders/shaders.metal`
- `crates/warpui/src/rendering/wgpu/shaders/image_shader.wgsl`
- `docs/release/1.0-open-issue-tests.json`
- `docs/superpowers/specs/2026-06-30-cockpit-increment1-account-usage-design.md`
- `specs/parity/cockpit-matrix.json`

</details>

## Querschnitts-Audits am 10. September

| Status am ursprünglichen Audit-Stand | Audits | Status am Zielstand |
|---|---|---|
| Gelaufen | `unsafe-app-and-zaplex`, `unsafe-framework`, `process-spawning`, `terminal-model-lock`, `async-blocking`, `secrets-and-logging`, `path-and-transfer-safety`, `ssh-bootstrap-injection` | Ergebnisse bleiben historische Evidenz; alle acht benötigen eine Delta-Reconciliation gegen den Zielstand. |
| Nicht gelaufen | `persistence-and-migrations`, `ci-release-security`, `upstream-drift`, `error-swallowing-and-cancellation`, `feature-flags-i18n-dead-code` | Vollständig offen. |
| Mechanisch gelaufen | Clippy und cargo-deny | Logs stammen vom alten Stand. Frische Läufe gehören in CI; aus ihnen darf vorher kein aktuelles Bestehen abgeleitet werden. |

Der am 10. September erfasste `upstream/main`-Snapshot stand bei
`5d874456a51cb7c412cfac7c21250f08ba5017f9` und enthielt 30 Commits
nach dem Merge-Base
`f4b04d58691e4ee12ae757152b321eea9c132942`. Vor dem eigentlichen
Upstream-Drift-Audit muss der Remote-Stand erneut authentifiziert aktualisiert
und als SHA festgehalten werden.

## Damals festgehaltenes Methoden-Delta vor einer Fortsetzung

| Alte Methode | Belegter Zustand | Erforderliche Korrektur |
|---|---|---|
| `chunks.json` / `remaining_chunks.json` | Mengen sind für v1.0.6 konsistent, enthalten aber 53 geänderte Zielstand-Dateien nicht. | Inventar am Ziel-SHA neu erzeugen; jede relevante Datei genau einer Einheit oder einem dokumentierten Split zuordnen. |
| `find_small.js` | Richtiger Batch-Mechanismus für ausgewählte 3–5 Einheiten/Audits; Schema verlangt Coverage und konkrete Failure-Pfade. | Repo, Ziel-SHA, Artefaktverzeichnis und Dedupe-Snapshot parameterisieren; keine alten absoluten Pfade oder den alten SHA weiterverwenden. |
| `find_v2.js` | Würde alle übergebenen Rest-Chunks plus alle Audits parallel starten. | Nicht für die Wiederaufnahme verwenden; der ungebremste Fan-out widerspricht dem quota-schonenden Batch-Verfahren. |
| `aggregate.py` | Normalisiert Ergebnisse und dedupliziert primär über Datei, Zeilennähe und Titel-Tokens. | Nur als Vorfilter behandeln; Root-Cause-Duplikate über mehrere Dateien müssen im adversarischen Review zusammengeführt werden. |
| `run_codex_verify*.sh` / `verify_ctx*.md` | Read-only Leaf-Verifikation mit Widerlegung, Impact und Reproduktion/Fix ist fachlich passend. Kontext und Pfade sind auf v1.0.6 eingefroren. | Ziel-SHA und vollständige Liste **offener und geschlossener** Issues je Batch frisch einbetten; jede Aussage am Zielstand neu prüfen. |
| `file_issues.py` | Alte Authentifizierung und feste lokale Pfade entsprechen nicht mehr der Repo-Regel. | Nicht unverändert ausführen; GitHub-Aufrufe müssen die für das Repository konfigurierte Authentifizierung verwenden. Erst verifizierte, nicht duplizierte Befunde schreiben. |
| `rounds.json` / `gen_tracking_body.py` | Drei Batches sind erfasst; 40 Einheiten und acht Audits werden korrekt aus den Ergebnisdateien abgeleitet. | Erst nach erfolgreicher Coverage- und Verdict-Prüfung ergänzen; generierten Tracker gegen diese Matrix plausibilisieren. |

Der damals vorhandene Dedupe-Snapshot war ebenfalls veraltet. Von den 80 aus
den drei Audit-Batches angelegten Finding-Issues waren am 2026-09-10 76
geschlossen und vier offen: `#215`, `#242`, `#243`, `#259`. Geschlossene Issues bleiben für die
Deduplizierung relevant; ein nachweislich erneut vorhandener Fehler ist als
Regression mit Verweis auf das frühere Issue zu behandeln, nicht als unbekannter
Neubefund.

## Wiederaufnahmeplan vom 10. September

### Voraussetzung

1. Einen neuen Ziel-SHA festschreiben; während eines Batches nicht wechseln.
2. Das Datei-/Zeileninventar für diesen SHA neu erzeugen und die 53 bisher
   unzugeordneten geänderten Dateien aufnehmen.
3. Aus allen GitHub-Issues und bereits verifizierten Befunden einen aktuellen
   Dedupe-Kontext erzeugen.
4. Alte Finder-/Verifier-Kontexte auf Ziel-SHA und portable Pfade umstellen.

### Empfohlene erste Batches

| Batch | Aufgaben | Warum zuerst |
|---|---|---|
| A: CI/Release | `ci-release-security`, `n001`, `n002`, `n004`, `n013` | Sicherheitskritischer Hebel; Audit und vollständige Einheiten liefern gegenseitige Coverage, ersetzen einander aber nicht. |
| B: Persistenz | `persistence-and-migrations`, `c112`, `c113`, `c278`, `n009` | Datenintegrität, Schema/Migrationskette und Laufzeitnutzung gemeinsam prüfen. |
| C: Lifecycle | `error-swallowing-and-cancellation`, `c117`, `c118`, `c120`, `c280` | Die stark geänderten Remote- und Task-Lifecycle-Pfade zuerst schließen. |
| D: Flags/i18n | `feature-flags-i18n-dead-code`, `c299`, `c300`, `c050`, `c051` | Gating, tote Pfade und UI-Kataloge zusammenführen; neue FTL-Dateien explizit zuordnen. |
| E: Upstream | `upstream-drift` | Alle Upstream-Commits gegen einen frisch geholten und festgehaltenen Upstream-SHA einzeln klassifizieren und Bug-/Security-Fixes im Zielcode prüfen. |

Danach folgen die übrigen Tier-1-Einheiten, die 25 Delta-Reviews, Tier 2 und
zuletzt Tier 3. Ein Querschnitts-Audit ersetzt niemals das vollständige Lesen
einer Einheit.

## Damals festgelegte Abschlusskriterien

Für den Abschluss von Issue #245 wurden folgende Kriterien festgehalten:

- Das Inventar ist an einen dokumentierten Ziel-SHA gebunden und weist keine
  relevante Datei ohne Zuordnung, keine Lücke und keine unbegründete
  Doppelzuordnung auf.
- Für jede Einheit existiert ein Ergebnis am Zielstand mit
  `coverage.files_not_fully_read = []`; die 25 geänderten Alt-Einheiten sind
  durch Delta- oder vollständiges Re-Review aktualisiert.
- Alle 13 Querschnitts-Audits sind am Zielstand gelaufen; für die acht alten
  Audits ist mindestens die seit v1.0.6 geänderte Scope-Fläche reconciled.
- Clippy- und cargo-deny-Evidenz stammt vom Zielstand und wurde in CI erzeugt.
- Jeder Rohbefund wurde adversarisch gegen den realen Zielcode verifiziert;
  `cannot-determine`-Verdicts sind aufgelöst oder ausdrücklich als offene
  Verifikationsarbeit gezählt.
- Duplikate wurden gegen offene und geschlossene Issues geprüft. Nur bestätigte,
  konkrete Failure-Pfade werden als Issue angelegt; Root-Cause-Duplikate werden
  gebündelt.
- `rounds.json`, die aggregierte Abdeckung und der Body von Issue #245 stimmen
  nach einer abschließenden Mengenprüfung überein.

Der am 10. September dokumentierte Zwischenstand lautete: **15 Einheiten am
damaligen Zielstand textuell aktuell abgedeckt; 341 Einheiten bzw.
Delta-Einheiten plus 53 noch zuzuordnende geänderte Dateien, fünf ungestartete
Querschnitts-Audits und acht Audit-Deltas offen.** Diese historische Bilanz
trifft keine Aussage über die Abdeckung späterer Commits.
