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
wurde am 26.10.2026 um 11:57 UTC mit Test-DMG `v1.2.39` gestartet. Vorprüfung,
Abhängigkeitssicherheit, statische Prüfungen, Linux-Compile und gezielte
Linux-Keymap-/Settings-/Sitzungsprüfungen sowie macOS-Compile und native
Layoutprüfungen sind bestanden. Die Linux-Gesamtsuite meldet **4438 bestanden,
13 übersprungen**; Dokumentationstests **4 bestanden, 4 ignoriert**.
Übersprungene und ignorierte Fälle werden nicht als ausgeführte Erfolge gezählt.
Live-SFTP läuft noch. Ein vollständiger CI- oder GUI-Pass ist zu diesem
Zwischenstand noch nicht belegt.

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
