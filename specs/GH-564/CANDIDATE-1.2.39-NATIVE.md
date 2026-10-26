# Native Abnahme des Kandidaten 1.2.39

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).

App- und Testquellstand: `8a569cc4844a1b3b04668d727d184b294e07b310`.
[PR #712](https://github.com/byte5ai/zaplex/pull/712) ergänzt für #711 den
vollständigen lokalisierten Worktree-Titel als Tooltip oberhalb des Menüeintrags.
Das seitliche Untermenü darf ihn nicht verdecken. PR #713 ergänzt ausschließlich
die bereits bestandene native Prüfung der einreihigen deutschen Dateileiste
auf dem unveränderten 1.2.38-DMG.

## CI und bestehende Nachweise

Der ausdrücklich freigegebene [CI-Batch 40318905418](https://github.com/byte5ai/zaplex/actions/runs/40318905418)
wurde am 26.10.2026 um 11:57 UTC mit Test-DMG `v1.2.39` gestartet und um
14:08 UTC erfolgreich abgeschlossen. Vorprüfung,
Abhängigkeitssicherheit, statische Prüfungen, Linux-Compile und gezielte
Linux-Keymap-/Settings-/Sitzungsprüfungen sowie macOS-Compile und native
Layoutprüfungen sind bestanden. Die Linux-Gesamtsuite meldet **4438 bestanden,
13 übersprungen**; Dokumentationstests **4 bestanden, 4 ignoriert**.
Übersprungene und ignorierte Fälle werden nicht als ausgeführte Erfolge gezählt.
Live-SFTP, Phase A, App-/Daemon-Bau und Signieren/Paketieren sind bestanden.
Die Notarisierung wurde akzeptiert; Anheften und Validieren des Tickets sind
ebenfalls erfolgreich. Der Lauf dauerte einschließlich der Runner-Warteschlangen
**2 Stunden 10 Minuten 47 Sekunden**. Insbesondere zwischen fertigem Daemon
um 13:15 UTC und Beginn der Paketierung um 13:57 UTC lag zusätzliche Wartezeit.

Alle **16/16 installierten GUI-Fälle** sind im ersten Lauf bestanden. Der
[validierte Nachweis](evidence/candidate-139-native/verified-full-gui.json) prüft
Testbaum, Zusammenfassung, Prozess-Exit, Version, App-/Testquellstand und
DMG-Hash gemeinsam gegen den Produktionsvalidator. Keine Fortsetzung war nötig.
Die Original-JSONs sind daneben gespeichert.

Das unveränderte DMG aus Artefakt `12656692394` hat 195.244.936 Bytes und SHA256
`7e23137b6df9b6ad7e728bae4649ad1375ad340733f055ea732b88f3a1c75e9e`.
Das heruntergeladene ZIP stimmt mit dem GitHub-Digest
`e1c7d2aa420c4f20fbeb7987cb390756e186519adbcbfa10a71383aef8191652` überein.
Kein Release oder Tag wurde erstellt. Die direkte Produktnachprüfung ist unten getrennt von diesem vollständigen
CI-/GUI-Ergebnis dokumentiert.

Die [lokalen Prüfungen der Korrektur](evidence/menu-711-local.json) sind bereits
bestanden: Compile-Gate, vier gezielte bestehende Menü-/Tooltiptests, App-Clippy
und 13 Phase-A-Prüfungen. Sie werden nicht erneut ausgeführt.
Die [1.2.38-Nachweise](CANDIDATE-1.2.38-NATIVE.md) bleiben ihrem ursprünglichen
DMG zugeordnet; sie sind kein Nachweis des geänderten 1.2.39-Quellstands.

## Vorab festgelegte native Nachprüfung

Unabhängige Sollquellen sind [Startmenü L1–L6](../../docs/ui/pane-launch-and-tab-join.html#spec),
die [UI-Referenz](../../docs/ui/README.md) und die dokumentierte Soll-/Ist-Abweichung
in [#711](https://github.com/byte5ai/zaplex/issues/711). Der Referenzstand wird mit
Quellrevision und Datei-Hashes festgehalten, bevor native Ergebnisse bewertet werden.

| Umfang | Erwartung |
|---|---|
| E04 / #711, EN/Dark und EN/Light | Vollständiger Worktree-Titel während geöffnetem Untermenü lesbar; Tooltip nicht vom Untermenü verdeckt. Normale und tatsächlich gemessene Mindestbreite getrennt bewerten. |
| L01, vier Split-Richtungen | Gemeinsame Startmenü-Einträge entsprechen dem Plusmenü; Tab-Konfigurationen und Wiederöffnen erscheinen nur bei Plus. Tatsächlich verfügbare Agent-/Hosteinträge berücksichtigen. |
| L05, Außenklick | Abbruch erzeugt keine Pane/Session. Frische Shellausgabe erhält PID, nicht exportierten Marker und Arbeitsverzeichnis. |
| E03/A03, soweit erreichbar | Einstieg ohne angemeldetes Konto zeigt einen ehrlichen Zustand und einen gangbaren Rückweg. Leere, ladende und fehlerhafte Inventare nicht gleichsetzen. |

Nach jeder Aktion wird der sichtbare Zustand neu bewertet. Fehlgeschlagene
Operatoraktionen werden von Produktfehlern getrennt; historische Shellausgaben
zählen nicht als frischer Zustandsnachweis. Bereits bestandene F02-/F03-Varianten
werden nicht wiederholt. Standbilder ersetzen keine Bewegungsnachweise.

## Native Layout-Nachweise

Der macOS-Job bestätigt sechs Plattformtests, einen Agenten-Layouttest,
sechs Workspace-Fälle und einen Header-Einstiegstest ohne Fehler oder
übersprungene Fälle. Die beiden Artefakt-ZIPs stimmen mit ihren GitHub-Digests
überein; alle 41 Workspace-Bildhashes zusätzlich mit dem Manifest des Jobs.

Das [Bildverzeichnis](evidence/candidate-139-native/manifest.json) enthält alle
**44 Originale**, ihre Maße, Herkunft, Hashes und Beobachtungen.
**36 Bilder wurden einzeln betrachtet; acht sind bytegenau identisch** mit
bereits geprüften Originalen aus dem 1.2.38-Verzeichnis, auf die jeweils genau
verwiesen wird. Sie werden nicht als erneut betrachtet gezählt.
Die unveränderten [Agenten-](evidence/candidate-139-native/native-agent.zip) und
[Workspace-Archive](evidence/candidate-139-native/native-workspace.zip) sind
dauerhaft gespeichert; jedes der 44 Bilder wurde auch gegen diese gespeicherten
Archive gehasht.

| Ansicht | Beobachtung und Grenze |
|---|---|
| Agenten-Fixtures | Sitzungsende und Fehlerzustand behalten ihre sichtbaren Rückwege und umgebrochenen Aktionen. Lange Metadaten sind gekürzt. Kein Nachweis einer echten Provider-Sitzung. |
| Verbindungen/Favoriten | Vollständige Hosthinweise, feste Zeilenaktionen, umgebrochene Leerzustandsanleitung und sichtbares Elternmenü neben dem Flyout. Einige Leerzustands-Standbilder zeigen die Ordner-/Plus-Symbole nicht; deren tatsächliche Erreichbarkeit wird separat interaktiv bewertet. |
| Worktree-Menü | Der Zeilentitel bleibt gekürzt. Die Fixture aktiviert seinen Hover nicht und belegt deshalb weder Erfolg noch Fehler des neuen Tooltips. |
| Dateimanager DE/EN, hell/dunkel | Alle acht Aktionen in zwei Reihen bei 1180 und vier Reihen bei 760 Punkten. Aktive/inaktive Pane, Neuer-Ordner-Dialog und vollständiger Dateinamen-Tooltip sind unterscheidbar. Der Terminalentwurf bleibt im Rückkehrbild sichtbar. Kein neuer F03- oder frischer Shell-PID-Nachweis. |
| Pane/Header | Vorher/Nachher erhalten unterscheidende Identitäten und Entwurf. Header-Einstieg öffnet das ursprüngliche Verzeichnis neben dem unveränderten Geschwisterverzeichnis. Kein Bewegungsnachweis. |

Die [Sollreferenzen mit Hashes](evidence/candidate-139-native/design-reference.json)
sind unabhängig von den nativen Aufnahmen festgehalten.

## Direkte native Nachprüfung am unveränderten DMG

[Exploration 40338712200](https://github.com/byte5ai/zaplex/actions/runs/40338712200)
ist erfolgreich abgeschlossen und ausdrücklich vor dem Zeitlimit beendet.
App und Harness entsprechen dem oben genannten Quellstand, der DMG-Hash ist
unverändert. Artefakt `12660361804`, ZIP-SHA256
`49e8c86a62ed4b9452e44474fb30526202c9f12568a1b0b84001572a573d2701`.
Alle **38 Originalaufnahmen** wurden einzeln betrachtet und bytegenau mit dem
CI-Artefakt verglichen. [Manifest und Beobachtungen](evidence/candidate-139-native/exploration-40338712200/manifest.json),
[Aktionen einschließlich Fehlern](evidence/candidate-139-native/exploration-40338712200/actions.json)
sowie Abschlussmarker sind dauerhaft gesichert.

| Kriterium | Ergebnis |
|---|---|
| E04 / #711, EN/Dark und EN/Light, 1024×678 | Bestanden: vollständiger Worktree-Titel oberhalb der Zeile und rechtes Untermenü gleichzeitig lesbar (04/15). |
| E04, tatsächliche Mindestbreite 480×678 | Fehlgeschlagen: Worktree-Untermenü überdeckt seine Elternzeile; vollständiger Titel nicht sichtbar. Dark zweimal (07/08), Light ebenfalls (17). Die Größenanforderung 200 wurde auf480 geklemmt; dies ist kein Größenfehler der App. |
| L01, rechts/unten | Gemeinsame Einträge Terminal, New agent und Favorites mit Leerzustand; keine Tab-Konfigurationen oder Wiederöffnen im Split-Menü (24–26). Links/oben in diesem Lauf nicht geprüft: der versuchte Shortcut öffnete die Sitzungssuche (28/29). |
| L05, Außenklick rechts/unten | Keine zusätzliche Pane. Frisch ausgeführte Ausgabe nach beiden Abbrüchen bestätigt PID1279, nicht exportierten Marker `native139` und dasselbe Arbeitsverzeichnis (02/31). |
| Leere Connections, EN/Light1024 | Ordner/Plus ohne Hover sichtbar (34), Plus mit vollständigem Tooltip (35), öffnet Hostauswahl (36). Keine Verbindung angelegt. Die fehlenden Icons einzelner Layout-Fixtures sind hier nicht reproduziert. |
| A03 / echter Agent ohne Konto | Nicht geprüft. Settings- und Konversations-Leerzustände sind kein Nachweis einer echten Provider-Interaktion. |

Die zunächst wirkungslosen Pane-Menüklicks waren durch macOS-Mitteilungen
abgefangen worden: Desktop20/21 zeigt die Überlagerung,23 deren Entfernung,
24 das danach funktionierende Produktmenü. Operator-Tippfehler meldeten
`mutation_dispatched`; der tatsächliche Text wurde jeweils vor Return betrachtet.
`hotkey` ist in der Operatorversion entfernt; der Aufruf führte keine Aktion aus.
Keine dieser Operatorabweichungen wird als Produktfehler gezählt.

Der neue Mindestbreitenfehler ist von der bei Normalbreite erfolgreichen
Tooltip-Korrektur getrennt: `should_render_sidecar_left` fällt auf rechts zurück,
wenn300 Punkte auf keiner Seite passen. Die Fensterbegrenzung schiebt das
Untermenü dann über die Elternzeile. Soll vor Korrektur: Elternzeile und vollständiger
Hinweis lesbar, Suchfeld und Aktion erreichbar, richtiger Fokus und Abbruch ohne
Sessionanlage bei Mindestbreite und nach Größenwechsel. Standbilder beweisen
weiterhin keine kontinuierliche Hover-Bewegung.
