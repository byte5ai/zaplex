# Installierte Abnahme und Exploration von 1.2.49

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562). Geprüfte App- und Testquelle:
`67be2c9ac435e1b636cb8ea0b725b8a941d4abf7`. Alle drei Explorationen verwenden
das unveränderte, signierte und notarisierte DMG aus
[Batch 40970919530](https://github.com/byte5ai/zaplex/actions/runs/40970919530).
DMG-SHA256: `5f4429e3f38b128b6cfc079a1993fe16bd46846158bbca516a5ed31ecd622557`.
Kein Release oder Tag wurde erstellt.

## CI und installierte Pflichtfälle

Der Batch ist vollständig erfolgreich. Die qualifizierte Fortsetzung übernimmt
4.391 positive Mac-Paketfälle aus dem ursprünglichen 1.2.49-Lauf und ergänzt
genau den korrigierten Journal-Fixture-Fall in 0,127 Sekunden. Die vollständige
Abdeckung von 4.392 Paketfällen wird vor dem Artefaktbau geprüft. Die unveränderten
ursprünglichen Linux-/SFTP-, Compile-, Sicherheits- und weiteren Testgates behalten
ihre belegte Herkunft; die Fortsetzung wiederholt sie nicht.

Alle **16 unterschiedlichen installierten GUI-Pflichtfälle** bestehen im ersten
Lauf, ohne Fehler, erwartete Fehler oder übersprungene Fälle. Der originale
XCTest-Fallbaum wurde zusätzlich gegen den vor Ausführung gesicherten Pflichtbestand
geprüft. App- und Testquelle, installierte Version und DMG-Hash stimmen überein.
[Unabhängiger Nachweis](evidence/candidate-149-native/installed-gui.json).

## Tatsächliche SSH-Einrichtung und Persistenz

[Exploration 40980387555](https://github.com/byte5ai/zaplex/actions/runs/40980387555)
prüft EN / Zaplex Dark / 1024×678 Punkte. Die Vorbedingungen und Erwartungen standen
vor Ausführung fest. Der isolierte Testbenutzer und die Laufzuordnung wurden geprüft.

| Ablauf | Beobachtung | Originalaufnahmen |
|---|---|---|
| Klassische Verbindung mit vorhandenem Installationsverzeichnis | Fehlende passende Erweiterung wird automatisch nachinstalliert; frische Remote-Shell verarbeitet Eingaben. | 006–009 |
| Echte Erstinstallation | Das ausschließlich eigene Installationsverzeichnis wurde nach Verlassen der klassischen Verbindung reversibel verschoben. Bei der nächsten Verbindung erscheint die tatsächliche Erweiterungswahl. Zwei Aufnahmen liegen 85,375 Sekunden auseinander; die Wahl bleibt unverändert bedienbar. Return startet die Installation, anschließend verarbeitet die Shell frische Eingabe. | 014, 016–020 |
| Automatisch geladenes Inventar | SSH-Alias importiert, passende persistente Sitzung geöffnet; sie erscheint ohne zusätzliche Aktualisierung in der Sessionliste. | 021, 023–024 |
| Tatsächliche Transporttrennung | Genau einen eigenen ControlMaster mit `ssh -O check/exit` geprüft und beendet, beide Rückgabecodes 0. | 025–026 |
| Persistenz nach Wiederverbindung | Frische Ausgabe bestätigt dieselbe Remote-PID, denselben nicht exportierten Marker und dasselbe Arbeitsverzeichnis. | 024, 027–028 |
| Ursprüngliche lokale Shell | Nach Rückweg aus der klassischen Verbindung frisch dieselbe lokale PID, Marker und CWD nachgewiesen. | 002, 031–032 |

27 Originale wurden einzeln betrachtet; 49 lokal gespeicherte Aufnahmen wurden
zusätzlich bytegenau mit dem vor VM-Abschluss privat abgeholten Archiv verglichen.
Diese zweite Zahl ist keine Behauptung, alle 49 visuell geprüft zu haben.
[Fallurteile und Hashes](evidence/candidate-149-native/40980387555.json).

## Startkarte: DE/EN, Hell/Dunkel und Mindestgröße

[Exploration 40982095905](https://github.com/byte5ai/zaplex/actions/runs/40982095905)
prüft Englisch; [40983860453](https://github.com/byte5ai/zaplex/actions/runs/40983860453)
Deutsch nach vollständigem App-Neustart mit neuer Shell-Baseline. Die deutsche
Baseline ersetzt keinen Shellerhalt über den Neustart.

| Variante | 1024×678 | 480×320 | Rückweg und frische Shell |
|---|---|---|---|
| EN / Zaplex Dark | Vier Claude-Modelloptionen und fünf Effort-Stufen sichtbar (007). | Alle Modellnamen und Efforts innerhalb der Karte, vertikal erreichbar; Haiku/Opus und Max tatsächlich gewählt. Zusammenfassung bestätigt Opus/Max. Vollständiger realer Codex-Fehler; Start gesperrt, auch nach Klick (008–019). | Cancel, anschließend dieselbe PID/Marker/CWD (020–021). |
| EN / Light | Dieselben Modell- und Effort-Optionen sichtbar (031). | Optionen, vollständiger realer Codex-Fehler und untere Aktionen per Scrollen erreichbar; Start im Fehlerzustand gesperrt (032–036). | Escape, anschließend dieselbe PID/Marker/CWD (037). |
| DE / Zaplex Dark | Modell- und Effort-Optionen sowie deutsche Hülltexte sichtbar (008). | Lange Identitäten und Effort-Stufen erreichbar; vollständiger realer Codex-Fehler mit deutscher Einleitung; Start gesperrt, Abbrechen erreichbar (009–013). | Abbrechen, anschließend dieselbe PID/Marker/CWD (014). |
| DE / Light | Modell- und Effort-Optionen sichtbar (021). | Alle Optionen, Fehler und untere Aktionen erreichbar (022–027). Das unsichtbare obere Schließen-Symbol bleibt per tatsächlichem Klick wirksam (028). | Danach dieselbe PID/Marker/CWD frisch nachgewiesen (029). |

Damit ist die gezielte Korrektur des horizontalen Abschneidens aus #720 am
Minimum bestätigt. Der Codex-Fehler wurde nicht zusätzlich in jeder normalbreiten
Variante aufgenommen. Ein erfolgreicher angemeldeter Modellstart folgt daraus nicht.

**Neuer Fehler [#743](https://github.com/byte5ai/zaplex/issues/743):** Das obere
Schließen-Symbol ist in Light weiß auf weiß, in EN und DE, bei normaler Breite
und Mindestgröße. Die gemeinsame Komponente verwendet eine feste weiße Farbe.
Die unteren Abbruchwege funktionieren; der Klick auf die unsichtbare obere
Schaltfläche in DE/Light schließt ebenfalls nur die Karte und erhält die Shell.
Die vorbereitete Korrektur in 1.2.50 verwendet das vorhandene `PaneHeaderTheme`
unverändert. Der neue Test scheitert am ursprünglichen Button und besteht danach
in Hell/Dunkel jeweils für Ruhe und Hover. Das ist eine lokale Regression,
noch keine native Abnahme des korrigierten DMG.
[Getrennte Testherkunft](evidence/candidate-149-native/modal-close-contrast.json).

Für die Korrektur bestehen lokal `cargo check --locked`, der unveränderte
CI-Clippy für `--bin zaplex` mit `-D warnings`, Phase A und die Workflow-Prüfung.
Ein zusätzlich ausgeführter strikter Test-Clippy schlägt auf beiden Ständen mit
exakt denselben 159 Diagnosen in 43 unveränderten Dateien fehl. Der vollständige
Vergleich auf `main` ergibt keine neuen oder entfernten Diagnosen; dieser
zusätzliche Check wird ausdrücklich nicht als bestanden ausgewiesen.

34 englische und 51 deutsche Originale wurden einzeln betrachtet. 41 englische
Aufnahmen wurden gegen das abgeschlossene Workflow-Artefakt verglichen; 60 deutsche
gegen das vor Abschluss privat abgeholte Archiv. Spätere Prozessdiagnosen liegen
zusätzlich lokal mit Einzelhash vor. [EN](evidence/candidate-149-native/40982095905.json),
[DE](evidence/candidate-149-native/40983860453.json).

## SSH-Abbruch und Schlüssel-Einrichtungsweg

Die deutsche Sitzung ergänzt DE / Light / 1024×678:

| Ablauf | Beobachtung | Originalaufnahmen |
|---|---|---|
| Klassischen Aufbau abbrechen | Importierten Host auf Standard-Sitzung gestellt und gespeichert. Verbinden öffnet einen neuen Tab mit tatsächlicher deutscher Erweiterungswahl. Escape entfernt diesen Tab vollständig; vorhandene Host-Einstellungen und lokale Shell bleiben erhalten. | 040–046 |
| Frischer Shellerhalt | Nach SSH-Abbruch und nochmals am Ende bleiben PID, nicht exportierter Marker und CWD gleich. | 006, 047, 053 |
| Fehlende Schlüssel-Anmeldung | Im ausschließlich eigenen Testprofil Passwort-Authentifizierung und persistente Sitzungen eingestellt. Die Sessionliste zeigt den deutschen Schlüssel-Hinweis. „Verbindung einrichten“ fokussiert die vorhandenen passenden Host-Einstellungen, ohne weitere Pane. Keine Passwort-Verbindung oder neue Anmeldedaten verwendet. | 048–052 |
| Prozessabgrenzung | Nach dem Abbruch kein klassischer SSH-Master und kein Proxy beobachtet. Ein wiederverwendbarer Headless-ControlMaster bleibt in der isolierten VM. Der erste Diagnosefilter erfasste unqualifizierte Prozessnamen nicht; nur die korrigierte Abfrage wird gewertet. | 053, 055–056 |

Erste Add-host-Klicks nach dem Themewechsel erzeugten kein sichtbares Menü.
Nach Schließen/Öffnen der Sidebar und frischem Hover/Klick funktionierte der Einstieg.
Die Ursache ist nicht von der Operator-/Fokussteuerung getrennt; weder ein
App-Hänger noch eine uneingeschränkt bestandene Maus-Fokusfolge wird behauptet.

Alle Explorationen wurden ausdrücklich beendet. Die Remote-Bereinigung bestätigt
ausschließlich eigene Prozesse und reversible Ablage der eigenen Testdaten. Die
deutsche Sitzung hat keinen Daemon gestartet. Rohbilder mit privaten Remote-Details
bleiben privat; die versionierten Belege enthalten Fallurteile, Identitäten und Hashes.

## Grenzen der Produktabnahme

Die offiziellen Statusbefehle bestätigen auf dem Remote-Testbenutzer Claude
`api_key_helper` und Codex API-Anmeldung; die isolierte Mac-Umgebung ist bei beiden
CLIs unangemeldet. Diese Zustände werden weder als Subscription-Routing noch als
erfolgreicher verwalteter Agent-/Agenten-Worktree-Fork gezählt. Die entsprechende
Ressourcenfrage ist gestellt; die übrigen unabhängigen Prüfungen wurden fortgesetzt.

Ohne tatsächlich erkannte Konten konnten gefüllte Cockpit-Listen, Kontotab-Dedupe
und der davon erreichbare Cockpit-Haupttab nicht nativ geprüft werden. Die leeren
Sessions-/Kontenansichten sind zusätzliche Teilbelege, keine Ersatzkonten. Weitere
Host-/Providerkombinationen, Passwort-Anmeldung, kontinuierliche Animationen,
Reduced Motion und die vollständige Matrix bleiben getrennt unbestätigt.
