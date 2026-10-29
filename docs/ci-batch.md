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
| `publish-tested-dmg` | Manuelle Veröffentlichung des bereits gebauten und vollständig GUI-geprüften DMGs samt Linux-Daemon; kein neuer Build. |
| `stale` | Nur manueller Verwaltungsjob. |
| `release-tag-guard` / `zap_release` | Bestehende Tag-/Release-Auslösung unverändert; keine Release-Freigabe durch diese Umstellung. |

Es gibt keine zeitgesteuerten Workflows mehr. Ein neuer Batch auf demselben Ref
wartet auf einen noch laufenden Batch. Eine Auslösung auf einem anderen Branch
scheitert vor den schweren Jobs.

### Geprüftes DMG unverändert veröffentlichen

`publish-tested-dmg.yml` übernimmt einen abgeschlossenen Main-Batch. Hat dessen
vollständige GUI-Suite bereits bestanden, ist `gui_run_id` dieselbe Run-ID wie
`artifact_run_id`; eine erneute GUI-Ausführung ist nicht erforderlich. Alternativ
kann eine erfolgreiche vollständige `gui-resume.yml`-Fortsetzung den GUI-Nachweis
liefern. Alle Compile-, Regression-, SFTP-, Signatur- und Paketprüfungen müssen
erfolgreich sein. Nur bei einer solchen Fortsetzung darf die ursprüngliche
GUI-Stufe im Batch fehlen oder abgebrochen sein. Der Nachweis muss alle 15
tatsächlichen XCTest-Fälle ohne Fehler oder Auslassung enthalten. App-Commit,
Test-Commit, installierte Version und DMG-Prüfsumme werden verglichen; direkte
Batch-Nachweise müssen außerdem exakt aus demselben Run und Versuch stammen.

Die Veröffentlichung enthält das unveränderte `Zaplex-arm64.dmg`, den passenden
Linux-x86_64-Daemon und dessen Prüfsummen. Sie erzeugt zunächst einen privaten
Entwurf und veröffentlicht erst nach Prüfung der hochgeladenen Dateidigests.
Ein unterbrochener Upload kann denselben Entwurf mit den fehlenden Dateien
fortsetzen; abweichende Dateien oder bereits veröffentlichte Releases werden
nicht überschrieben. Prüfbelege bleiben getrennt von den Installern erhalten.
Das Workflow-Token löst durch die Tag-Erzeugung keinen zusätzlichen Build aus.
Die konkrete Release-Freigabe bleibt Voraussetzung für diesen manuellen Dispatch.

### Prüfungen und Artefakt-Build zeitlich trennen

Ein Batch mit `artifacts=none` führt alle Sicherheits-, Compile-, Test- und
Live-SFTP-Prüfungen aus, ohne App oder DMG zu bauen. Er enthält nur einen
macrunner-Job (macOS-Compile und native Regressionen); die übrigen Jobs laufen
auf Linux. Nach seinem Ende bleibt der Mac bis zum nächsten Dispatch frei.

Ein späterer Batch mit `artifacts=test-dmg` und `checks_run_id=<Prüflauf>`
übernimmt ausschließlich einen erfolgreichen Prüf-Batch auf **exakt demselben
Commit**. Die Vorprüfung kontrolliert jeden erforderlichen Job sowie Paket- und
Dokumentationstests, verweigert Coverage-/gemischte Fortsetzungsmodi und bereits
begonnene Artefakt-Builds. Danach laufen nur die Artefaktstufen. Der Abschluss
verlangt weiterhin alle ausgewählten Ergebnisse; übersprungene Prüfungen zählen
nur mit dieser validierten Herkunft. App-Build, Paketierung und GUI-Abnahme
bleiben getrennte Jobs. Die Trennung garantiert keine maximale Compilezeit.

`ci-checks-provenance` bindet den Prüflauf an den Quellcommit. App-Fortsetzung
und spätere Veröffentlichung prüfen den ursprünglichen Lauf erneut unabhängig,
statt einen übersprungenen Job pauschal als bestanden zu behandeln. Der Beleg
bleibt sieben Tage verfügbar; ohne ihn wird die Übernahme verweigert.

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

### Nach einem macOS-Setupfehler fortsetzen

Wenn der macOS-Compilejob scheitert, Sicherheitsprüfung und beide Linux-Jobs aber
erfolgreich sind, übernimmt `resume_run_id` genau diese bestandenen Gates.
macOS und alle bisher nicht ausgeführten Folgeprüfungen laufen anschließend.
Die Vorprüfung lehnt einen laufenden Ursprungslauf, fehlende Linux-Erfolge und
bereits ausgeführte Folgestufen ab. App, Lockfile und Toolchain müssen unverändert
sein. Auch die Linux-Kommandos, ihre Umgebung und der gemeinsame Setup-Teil
werden verglichen; im Setup darf sich ausschließlich der ausdrücklich auf macOS
begrenzte Rust-Bootstrap ändern. Quellcommit und Ursprungslauf bleiben im Summary
sichtbar. Die macOS-Einrichtung aktiviert den vollständigen Rust-Werkzeugpfad des
Runner-Images und prüft die Proxies, bevor Cache- oder Compile-Schritte beginnen.

Hat eine solche Fortsetzung macOS und Paket-/Dokumentationstests bestanden,
Live-SFTP aber wegen eines Ablaufdefekts gar nicht ausgeführt, kann zusätzlich
`resume_base_run_id` den ursprünglichen Linux-/Sicherheitslauf nennen. Beide
Läufe müssen abgeschlossen sein. Die Vorprüfung kontrolliert die tatsächlichen
Ergebnisse und die unveränderten Eingaben über beide Quellstände; erst danach
laufen ausschließlich Live-SFTP und die gewählten Artefaktstufen. Ein grüner
Workflowstatus allein ist kein Nachweis für eine ausgeführte Stufe.

Die Bedingungen für Folgestufen berücksichtigen absichtlich übersprungene,
bereits belegte Vorgänger mit einer expliziten Statusfunktion. Der abschließende
Job „Verify all selected batch stages“ läuft immer und weist jede fehlende
Pflichtstufe ohne validierten Wiederverwendungsbeleg zurück. Das gilt auch für
eine angeforderte, aber nicht erzeugte DMG.

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

Falls ein erfolgreicher Test-DMG-Fortsetzungslauf die GUI-Abnahme übersprungen
hat, kann `gui-resume.yml` ausschließlich diese Prüfung nachholen. Der Dispatch
auf `main` verlangt `artifact_run_id` und den ursprünglichen `source_sha`.
Vor einem macOS-Job prüft er erfolgreiche App-/DMG-Jobs, den übersprungenen
GUI-Job, das verfügbare Artefakt und unveränderte App-/Testeingaben zwischen
Quellcommit und Artefakt-Workflow. Das vorhandene DMG wird installiert; weder
App noch Paket werden erneut gebaut. GUI-Tests und Quellnachweis verwenden
den angegebenen ursprünglichen Commit. Der Shell-Teststarter wird separat vom
Workflow-Commit geladen; `launcher-source-sha.txt` dokumentiert diese Revision,
während App- und XCTest-Quellstand unverändert bleiben.

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


## Fortsetzung nach fehlendem nativen Prüfwerkzeug

`resume_run_id` kann den fehlgeschlagenen nativen Policy-Job aus einem Main-Batch
fortsetzen, dessen native Sicherheits- und statische Schranken bestanden sind.
Die Vorprüfung bindet Quellstände, Plattform, unveränderte statische Befehle und
Testinventar; App-, Cargo-, Sicherheits- oder Asset-Änderungen schließen diese
Übernahme aus. Sie schreibt `ci-pre-policy-provenance` mit Ursprungs- und
Qualifikationsrevision. Nachfolgende Artefakt-/Releaseprüfungen lösen diesen
Nachweis selbstständig auf und prüfen den Ursprung erneut.

Nur die beiden belegten Schranken werden übernommen. Die native Policyprüfung,
Compile-, App-/Linux-Tests und Artefakte laufen regulär. Für kompakte Zeilenaktionen
stellt `prepare-native-checks` ripgrep mit PCRE2 bereit: vorhandene Fähigkeiten
werden geprüft, andernfalls wird ein natives Homebrew-Binärpaket mit seiner PCRE2-Abhängigkeit
verwendet. Der Aufruf prüft anschließend die tatsächliche Lookbehind-Funktion.

### Fortsetzung eines nativen Regressionstestblocks

Scheitert der Mac-Regressionsblock nach erfolgreichen nativen Sicherheits-,
statischen, Clippy- und Compile-/Layout-Gates, kann `resume_run_id` diese
unveränderten Gates und genau die belegten erfolgreichen Testschritte übernehmen.
`ci-native-checks-provenance.json` bindet Originallauf, beide Quellstände und die
exakte Schrittmenge. App-/Cargo-/Toolchain-/Werkzeugänderungen, veränderte
Testbefehle oder Testumgebungen sperren diesen Weg. Der spätere Artefaktvalidator
prüft Originalbelege und aktuelle Restergebnisse erneut; fehlende oder unerwartet
übersprungene Schritte sind Fehler. Frische VM-Einrichtung läuft weiterhin.
Unabhängige Restprüfungen laufen auch nach einem Fehler weiter, der Job bleibt rot.

Die Minidump-Adapterprüfungen gehören zum expliziten Linux-Teil: Adapter,
Abhängigkeiten und CLI-Worker existieren für Linux/Windows, nicht macOS. Der
Mac-Lauf 40924294137 führte beim alten Filter null Fälle aus; dieser Schritt wird
nie als Erfolg übernommen. Der Linux-Job verlangt mindestens die beiden
Lebenszyklustests und anschließend genau einen neu erzeugten, nicht leeren Dump.
