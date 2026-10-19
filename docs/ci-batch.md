# CI nach mehreren Merges

Schwere Prüfungen laufen ausschließlich nach Christians Freigabe als ein
`ci-batch.yml`-Lauf auf `main`. Alle wiederverwendbaren Workflows verwenden dabei
denselben Commit des Dispatchs; weitere Merges verändern diesen Lauf nicht.

| Workflow | Art und Auslösung |
| --- | --- |
| `no-new-cjk` | Leichter Linux-Check auf PRs. Dateipfade werden im Job gefiltert, damit der gleichnamige Pflicht-Status auch bei Dokumentationsänderungen erscheint. |
| `dependency-security` | Advisory-Prüfung ohne App-Build; zuerst im Batch, zusätzlich manuell aufrufbar. |
| `pr-check` | Schwere Linux-/macOS-Compile- und Regressionstests; nur aus dem Batch. |
| `test-dispatch` | Schwere Rust-Tests auf Blacksmith Ubuntu 22.04 (16 vCPU / 64 GB); nur aus dem Batch. |
| `live-sftp-safety` | Schwere SFTP-Integrationstests; nur aus dem Batch. |
| `cockpit-parity-audit` | Schwere Paritätsprüfung; nur aus dem Batch. Echte Zwei-Host-Abnahme wird nur mit `require_manual_runtime=true` verlangt. Ein automatischer grüner Lauf ersetzt diese Abnahme nicht. |
| `test-dmg` | Optionaler signierter Test-DMG-Build nach grünen Prüfungen, einschließlich Linux-Daemon. |
| `build-remote-server` | Optionaler separater Daemon-Build nach grünen Prüfungen; alternativ zum Test-DMG. |
| `stale` | Nur manueller Verwaltungsjob. |
| `release-tag-guard` / `zap_release` | Bestehende Tag-/Release-Auslösung unverändert; keine Release-Freigabe durch diese Umstellung. |

Es gibt keine zeitgesteuerten Workflows mehr. Ein neuer Batch auf demselben Ref
bricht einen noch laufenden Batch ab. Eine Auslösung auf einem anderen Branch
scheitert vor den schweren Jobs.

### Nach einem Runnerfehler bei den Pakettests fortsetzen

`resume_run_id` kann einen abgeschlossenen, fehlgeschlagenen `main`-Batch angeben,
wenn Dependency-, Compile-/Regression-, SFTP- und Dokumentationstests erfolgreich
waren und nur die Pakettests scheiterten. Die Vorprüfung liest die tatsächlichen
Jobs/Schritte dieses Laufs und vergleicht seinen Quellstand mit dem neuen Commit.
Nur Änderungen an der begrenzten Runner-/Fortsetzungslogik und den Belegen zu #564
sind zulässig; geänderte App-, Cargo-, Toolchain- oder andere Prüfeingaben verweigern
die Wiederverwendung. Paketwahl, Testumgebung und Dokumentationstest-Kommando
müssen ebenfalls gleich bleiben.

Danach laufen nur die Pakettests und die angeforderten Artefaktstufen. Die bereits
erfolgreichen Jobs und Dokumentationstests werden übernommen; Quellcommit und
Ursprungslauf stehen im Job-Summary. Ohne `resume_run_id` bleibt der vollständige
Ablauf erhalten. Coverage-Läufe unterstützen diese begrenzte Fortsetzung nicht.

### Nach einem Runnerfehler beim App-Build fortsetzen

Ein direkter `test-dmg.yml`-Dispatch auf `main` mit `resume_run_id` setzt nur den
fehlgeschlagenen App-Build und seine DMG-/GUI-Folgeschritte fort. Der ursprüngliche
Batch muss alle Code-/Test-, Readiness- und Signaturprüfungen sowie den Linux-
Daemon-Build bestanden haben. Dessen eindeutiges, noch nicht abgelaufenes Artefakt
wird aus dem Ursprungslauf geladen und vor dem Bündeln per SHA-256 geprüft.

Die Vorprüfung verweigert andere Fehler, geänderte App-/Prüfeingaben, einen
abweichenden Versions-Tag und das Debug-Profil. App, Verpackung und GUI-Tests
checken ausdrücklich den ursprünglichen Quellcommit aus. Nur die Fortsetzungs-
Workflow-Datei stammt vom neuen `main`; ein gewöhnlicher GitHub-Rerun würde auch
die alte Workflow-Datei behalten. Das App-Build-Limit ist `CARGO_BUILD_JOBS=4`.
Erfolgreiche Compile-/Paket-/SFTP-Prüfungen und der Blacksmith-Daemon werden nicht
wiederholt. Der ursprüngliche Batch bleibt als fehlgeschlagener Lauf erhalten;
der Fortsetzungslauf verknüpft seine Belege mit ihm im Job-Summary.

## Freigabe und Ablauf

1. Mehrere PRs nach den leichten Checks mergen.
2. Christian gibt genau den gewünschten Lauf frei:
   `release-approve "ci-batch.yml --ref main"`.
3. Erst danach `gho workflow run ci-batch.yml --ref main` ausführen.
   Ohne weitere Inputs entstehen keine DMG-/Daemon-Build-Artefakte.
   `artifacts=test-dmg` bzw. `artifacts=remote-server` muss Teil der Freigabe sein.
4. Bei rotem Batch erst korrigieren oder zurücknehmen, bevor weitere Änderungen
   gemergt werden. Ein Release darf nur von einem `main`-Commit mit grünem Batch
   erfolgen; die gesonderte Release-Freigabe bleibt erforderlich.

## Ruleset 18082412

Einziger Pflicht-Check für Merges nach `main` ist `no-new-cjk`;
`strict_required_status_checks_policy` ist aus. Ein PR braucht keine
Freigabe durch Reviews und kann gemergt werden, sobald `no-new-cjk` grün ist.
