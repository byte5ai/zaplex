# Zaplex 1.0 — Gate 0 Fortsetzung

Stand: 2026-07-23, nach angeordnetem Stopp wegen zu kleinem Temp-/Build-Volume.

## Verbindlicher Arbeitsstand

- Worktree: `/home/cwendler/projects/zaplex/repos/zaplex.worktrees/rc-master-plan`
- Branch: `rc/master-plan`
- HEAD: `35f5397e` (`docs: plan Zaplex 1.0 implementation and orchestration`)
- Der Haupt-Clone auf `main` wurde nicht verändert.
- Alle Gate-0-Änderungen sind uncommitted und unstaged im Worktree erhalten.
- Es wurde nichts gepusht, kein PR angelegt, kein Release/Publish ausgelöst und kein
  DMG- oder CI-Build gestartet.
- Git-Identität vor dem nächsten Commit erneut prüfen:
  `iret77 <63622643+iret77@users.noreply.github.com>`.

## Umgesetzter, noch nicht committeter Gate-0-Stand

### SFTP/Filemanager

- Klickziele für Breadcrumbs und Abbruch bleiben über Neurendern stabil.
- „Speichern unter“ ist an Remote-Pfad und Größe statt an einen flüchtigen
  Listenindex gebunden.
- Parallele Übertragungen verwenden eindeutige Zwischen- und Sicherungsziele.
- Bestehende Ziele werden erst nach vollständig erfolgreichem Kopieren ersetzt.
- Fehler und Abbruch räumen Zwischenstände auf und stellen vorhandene Ziele wieder
  her.
- Ein Move löscht die Quelle nur nach vollständig erfolgreichem Copy; bereits ein
  Skip macht den Vorgang unvollständig.
- Rekursive Uploads/Downloads prüfen vorab den vollständigen Baum, Pfadgrenzen,
  Symlinks und Spezialdateien und brechen unsichere Fälle geschlossen ab.
- `normalize_remote_path` akzeptiert allgemein `&Path`; alle 21 SFTP-Aufrufer
  wurden statisch geprüft.

### Agent-Sessions und Konten

- Terminal-Sessions tragen die Kontoidentität zusätzlich zu Anbieter, Session und
  Host.
- Ein bekanntes Konto kann nicht durch ein Terminal eines anderen oder unbekannten
  Kontos erfüllt werden.
- Die Konto-Zuordnung wird vor Start, Resume, Fork und Slash-Aktion gesetzt und
  bleibt bei der späteren Erkennung desselben Anbieters erhalten.
- Die zwei Kernregeln wurden nach allen abgebrochenen Rotläufen bytegenau wieder auf
  den grünen Stand gesetzt:
  - Kontoabgleich prüft gebundenen Anbieter plus E-Mail.
  - Eine bestehende Zuordnung wird nur bei einem Anbieterwechsel verworfen.
- Sicherungspatch der beiden Kernregeln hatte SHA-256
  `7864f485b62a0d913bcb20999fa0c410131e7718edc2cb7f92c1c3c40c6b5891`.

### Startbefehle und sichere Prozesssteuerung

- Startbefehle besitzen eine stabile ID und eine typisierte Bestätigung.
- Doppelte Zustellung führt nicht zu doppelter Ausführung; fehlgeschlagene
  Einreihung wird weder bestätigt noch als erledigt gespeichert.
- Verlorene Bestätigungen werden mit derselben ID wiederholt.
- Der begrenzte Verlauf bekannter IDs verhindert unbegrenztes Wachstum.
- Lokale Prozessaktionen prüfen PID und Prozess-Fingerabdruck unmittelbar vor dem
  Signal und brechen bei unbekannter oder wiederverwendeter Identität geschlossen
  ab.
- Für entfernte Prozesse gibt es eine typisierte Anfrage mit Session-ID, PID,
  Fingerabdruck und Signal. Es gibt keinen Shell-Befehl als Rückfall.
- Alte Gegenstellen ohne die neue Fähigkeit brechen geschlossen ab.

### Textqualität

- Status-Ellipsen verwenden `Starting…` statt `Starting ...`.
- Englische und deutsche Kataloge sowie ein eigenständig ausführbarer
  Katalogtest wurden ergänzt.

## Bereits belastbare Beweise

- SFTP Breadcrumb rot vor Fix:
  `breadcrumb navigation should fire exactly once across a rerender`,
  tatsächlich `0` statt `1`.
- Cockpit-Kontofallback rot vor Fix:
  `next_waiting_distinguishes_accounts_when_the_route_stamp_is_missing` und
  `session_key_uses_account_identity_when_the_route_stamp_is_missing`.
- `zaplex_cockpit` danach grün: 224 Unit-Tests plus 1 Snapshot-Test.
- Ellipsen-Test rot vor Fix bei `status = ✓ …`, danach alle 3 eigenständigen
  Katalogtests grün.
- `zaplex_remote_session` Startup-Fähigkeit grün: 1/1.
- Frühe Startup-/Prozess-API-Tests gingen erwartungsgemäß zunächst auf
  fehlenden Schnittstellen rot; die Implementierung ist vorhanden.
- `git diff --check` war nach der letzten Quellenwiederherstellung grün.

## Noch zwingend offen in Gate 0

Diese Punkte nicht überspringen und nicht als erledigt darstellen:

1. Neues dauerhaftes Volume mit ausreichend Platz für Cargo-Target und Temp
   bereitstellen. Mount-Pfad nicht raten; vom User übernehmen.
2. Worktree, Branch, HEAD, Identität, Status und `git diff --check` erneut prüfen.
3. Auf dem neuen Volume einen persistenten Testcache verwenden. Richtwert:
   mindestens 30 GiB frei, weil das App-Testprogramm plus Abhängigkeiten und
   parallele Link-Zwischenstände mehrere GiB benötigen.
4. Konto-Rot/Grün abschließen:
   - nur die zwei dokumentierten Kernregeln temporär zurücknehmen,
   - die drei neuen Laufzeittests rot ausführen,
   - exakt wiederherstellen,
   - dieselben drei Tests grün ausführen.
5. SFTP-Green-Suite ausführen, insbesondere:
   - Breadcrumb und Abbruch über Neurendern,
   - parallele Tempziele,
   - Abbruch-/Fehler-Cleanup,
   - vorhandenes Ziel bei fehlgeschlagenem Überschreiben,
   - Move mit Skip,
   - Pfadgrenzen, Symlinks und Spezialdateien,
   - Save-As nach Listen-Neusortierung.
6. Startup-Exactly-Once und typisierte Fernprozesssignale in
   `zaplex_remote_session`, `remote_server` und den fokussierten App-Tests prüfen.
7. Gesamtes `zaplex_cockpit` erneut grün ausführen.
8. Formatprüfung, fokussierte Tests und `cargo check`.
9. Vor jedem logischen Commit unabhängiges kritisches Review:
   - SFTP-Datensicherheit: Claude max plus unabhängige begrenzte Zweitprüfung.
   - Session-/Startup-/Prozessidentität: Claude max plus unabhängige begrenzte
     Zweitprüfung.
   - Text/Workflow: angemessen kleineres unabhängiges Review.
10. Findings im Quelltext verifizieren, nötige Korrekturen wieder mit echtem
    Rot/Grün-Beweis prüfen.
11. Logische Commits erstellen, sauberen Status herstellen und erst dann
    `V1_BASE_SHA` festhalten.
12. Danach stoppen. Kein Push, CI-/DMG-Build, Release oder Publish ohne neue
    ausdrückliche Freigabe im Moment der Aktion.

## Cache-/Speicherstatus beim Stopp

- Keine Gate-0-Cargo-/rustc-Prozesse liefen beim Stopp.
- Gelöscht wurden ausschließlich erzeugte, nicht mehr verwendbare Artefakte:
  - veraltetes 718.687.728-Byte-App-Testprogramm im privaten SFTP-`/dev/shm`,
  - `/tmp/zaplex-process-safety-target` (264 MiB; abgeschlossene Cargo-Artefakte),
  - unvollständige Warp-App-Teilobjekte aus zwei gescheiterten Linkläufen
    (311 MiB und 574,2 MiB),
  - unvollständige Teilstände von `zbus`, `image`, `serde_json` und `rayon`.
- Erhalten blieb `/tmp/zaplex-gate0-sftp-target` mit wiederverwendbaren,
  vollständigen Abhängigkeiten. Beim Stopp belegte er ungefähr 4,3 GiB.
- Den alten Cache erst löschen, nachdem der neue Volume-Pfad bestätigt und die
  Fortsetzung dort erfolgreich angelaufen ist.

## UI-Anker für die spätere Lane

Der am 2026-07-23 gelieferte WARP-Screenshot des Tab-Dropdowns ist die visuelle
Referenz. Zu übernehmen ist das Ordnungsprinzip, nicht die geringere
Funktionsmenge:

- klare visuelle Hierarchie und ruhige Abstände,
- wenige gleichwertige Hauptaktionen,
- saubere Trennlinien,
- favorisierte Hosts als kompakte Haupteinträge,
- hostbezogene SSH-/Agent-Aktionen in einem seitlichen Untermenü,
- keine automatisch wachsende flache Agent-Liste,
- nahtlose Typografie, Maße und Zustände auf dem Niveau der geerbten
  WARP-Komponenten.

## Erster Prompt nach dem Neustart

> Setze Gate 0 anhand von
> `docs/2026-07-23-v1-gate0-resume.md` fort. Das neue Volume ist unter
> `<VOM USER GENANNTER MOUNT-PFAD>` gemountet. Verifiziere zuerst Worktree,
> Branch, HEAD, Identität, Status, freien Platz und `git diff --check`.
> Lege Cargo-Target und Temp ausschließlich auf dem neuen Volume an. Führe dann
> die offenen Rot/Grün-Prüfungen, Reviews, Formatprüfung und `cargo check` zu
> Ende. Kein Push, CI-/DMG-Build, Release oder Publish. Nach den Gate-0-Commits
> `V1_BASE_SHA` dokumentieren und stoppen.
