# Native Abnahme des Kandidaten 1.2.38

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).

App- und Testquellstand: `2434f6cd84ec3db40769c5b351e28279b6010d7c`,
[PR #710](https://github.com/byte5ai/zaplex/pull/710). Dieser Stand korrigiert
#709: Dateizeilenklicks aktivieren ihre Pane und geben der Liste nach Suchfokus
den Tastaturfokus zurück, ohne Markierungen zu löschen.

## Lauf und Aussagegrenze

Der freigegebene [CI-Batch 40242122433](https://github.com/byte5ai/zaplex/actions/runs/40242122433)
wurde am 25.10.2026 um 22:51 UTC mit Test-DMG gestartet und am 26.10.2026
um 00:08 UTC erfolgreich abgeschlossen. Die Vorprüfung,
Abhängigkeitsprüfung und Compile-Jobs für Linux und macOS sind bestanden.
Die gezielten Plattform-/Keymapprüfungen und Live-SFTP sind ebenfalls bestanden.
Die Linux-Gesamtsuite meldet **4438 bestanden, 13 übersprungen**; die
Dokumentationstests **4 bestanden, 4 ignoriert**. Diese Ausnahmen werden nicht
als ausgeführte Erfolge gezählt. Phase A, der App-Bau und der Bau des gebündelten
Linux-Daemons sind bestanden. Signieren/Paketieren einschließlich akzeptierter
Notarisierung und der Job für die installierte GUI-Abnahme sind bestanden.
Die direkte native Produktnachprüfung ist davon getrennt.

Alle **16/16 installierten GUI-Fälle** sind im ersten Lauf bestanden. Testbaum,
Zusammenfassung, Prozess-Exit, Version, App-/Testquellstand und DMG-Hash wurden
gegeneinander geprüft; es war keine Fortsetzung erforderlich. Der
[geprüfte Nachweis](evidence/candidate-138-native/verified-full-gui.json) verweist
auf beide Artefakte. Die zugehörigen Original-JSONs sind daneben gespeichert.

Das unveränderte Test-DMG aus Artefakt `12618983528` hat 195.245.336 Bytes und
SHA256 `7f5643fa1b24bf52e844417bdc231cc31125ff81b1e0777a021a9ea8be25a05c`.
Sein heruntergeladenes ZIP stimmt mit dem GitHub-Digest
`befe73087b97fcae06319f490bbf118a87db8b24a8d828dc4cc6405a0981a906` überein.
Es wurde kein Release und kein Tag erstellt.

Die neun gezielten lokalen Regressionstests, Compile-Gate und App-Clippy sind
bereits im [lokalen Nachweis](evidence/focus-709-local.json) dokumentiert und
werden nicht erneut lokal ausgeführt. Der vorherige F03-Fehler auf 1.2.37 ist
historisch; die unten genannten nativen Kombinationen bestehen auf 1.2.38.

## Native Layoutbilder

Das abgeschlossene macOS-Jobprotokoll bestätigt sechs Plattformtests, einen
Agenten-Layouttest, sechs Workspace-Fälle und einen Header-Einstiegstest,
jeweils ohne Fehler oder übersprungene Tests.

Die beiden heruntergeladenen Artefakt-ZIPs stimmen mit ihren GitHub-Digests
überein. Alle 41 Workspace-Bildhashes stimmen zusätzlich mit dem Manifest des
Laufs überein. Das [Bildverzeichnis](evidence/candidate-138-native/manifest.json)
enthält Herkunft, Hash, Maße und Beobachtung für alle **44 Originale**.
Die unveränderten ZIPs sind mit diesem Bericht dauerhaft gesichert:
[Agentenbilder](evidence/candidate-138-native/native-agent.zip) und
[Workspace-Bilder](evidence/candidate-138-native/native-workspace.zip), zusammen
4,2 MB. Jeder Bilddatensatz nennt Archiv und darin enthaltenen Originalpfad;
alle 44 Bildhashes wurden auch gegen diese gespeicherten Archive geprüft.

**27 Bilder wurden einzeln betrachtet; 17 sind bytegenau identisch** mit einzeln
geprüften Originalen aus 1.2.37. Für diese 17 nennt das Verzeichnis die genaue
vorherige Bildreferenz; sie werden nicht als erneut visuell betrachtet bezeichnet.

| Umfang | Beobachtung und Aussagegrenze |
|---|---|
| Agentenansichten | Sitzungsende und Wiederherstellung zeigen erreichbare Aktionen; lange Identitäten sind gekürzt. Keine echte Provider-Sitzung. |
| Verbindungen und Leerzustände | Feste Aktionsplätze, vollständiger Hosttooltip und umgebrochene Anleitung zu Ordner/Plus bleiben sichtbar. Der englische Eintrag zur neuen Worktree-Konfiguration ist im Favoritenmenü sichtbar gekürzt; seine direkte Bedienung gehört zur weiteren E04-Exploration. |
| Dateimanager DE/EN, hell/dunkel | Alle acht Aktionen sind bei 1180 Punkten in zwei und bei 760 Punkten in vier Reihen sichtbar. Aktive/inaktive Pane sind unterscheidbar. Neuer-Ordner-Dialog, vollständiger Dateiname im Tooltip und zurückgekehrter Terminalentwurf sind belegt. Diese Fixtures beweisen keinen Dateizeilen-Fokuswechsel und keine einreihige Leiste. |
| Pane/Header | Vorher/Nachher zeigen erhaltene Pane-Identitäten und Entwurf, beim Header-Einstieg das ursprüngliche linke Verzeichnis neben dem unveränderten Geschwisterverzeichnis. Standbilder sind kein Bewegungsnachweis. |

## Festgelegte direkte Nachprüfung

Am neuen installierten DMG werden zwei Dateipanes mit `same.txt`, aber unabhängig
bekannten Inhalten/Größen verwendet. Klick auf die linke Zeile und F3 muss die
linke Datei zeigen; die umgekehrte Klickrichtung muss die rechte Datei zeigen.
Nach Fokus im Filter muss ein Zeilenklick den Listenfokus wiederherstellen,
sowohl innerhalb der aktiven Pane als auch nach einem Wechsel. Vorhandene
Markierungen bleiben erhalten; Rechtsklick verwendet das angeklickte Ziel.
Schmale Panes und eine zusätzliche deutsche/helle Variante werden geprüft.

Unabhängige Datei-Hashes vor/nach den nichtdestruktiven Aktionen und frische
Shell-Nachweise trennen tatsächlich erhaltenen Zustand von historischer Ausgabe.
Weitere bisher ungeprüfte Varianten der Produktmatrix bleiben offen.

## Direkte Bedienung am installierten DMG

[Exploration 40247974777](https://github.com/byte5ai/zaplex/actions/runs/40247974777)
verwendete genau das oben geprüfte DMG und denselben App-/Harness-Quellstand.
Alle **64 Originalaufnahmen** wurden einzeln betrachtet und gegen die archivierten
CI-Originale gehasht. [Manifest](evidence/candidate-138-native/exploration-40247974777.json),
[Originalbilder als ZIP](evidence/candidate-138-native/exploration-40247974777-originals.zip),
[Aktionsfolge](evidence/candidate-138-native/exploration-actions.json) und
[vorab festgelegte Dateierwartungen](evidence/candidate-138-native/fixture-expectations.json)
sind dauerhaft gespeichert. Die Bildnummern unten sind ihre Dateipräfixe.

| Kombination | Beobachtung | Originale |
|---|---|---|
| F03 EN/Dark, zwei Panes, 1024×678 | Zeilenklick aktiviert die angeklickte Pane; F3 zeigt rechts `same.txt` mit 16 Bytes und links mit 7 Bytes. | 10–14 |
| Filter → Zeile, EN/Dark | Innerhalb derselben Pane und nach Wechsel zur anderen Pane erreicht F3 das Cursorziel. Die getrennte Markierung von `marked.txt` bleibt erhalten. | 15–18 |
| Rechtsklick auf inaktive Pane, EN/Dark | Pane wird aktiv, Kontextaktion „Details“ trifft `right/marked.txt` mit 10 Bytes. | 20–21 |
| EN/Dark, 760×678 | Beide Fokusrichtungen und Rückkehr aus vorherigem Suchfokus bestehen; beide Markierungen bleiben. Globaler Cmd-Klick markiert genau eine Datei. | 22–27 |
| F03 DE/Light nach vollständigem Neustart, 1024×678 | Frische Shellausgabe belegt neue PID; beide Zeilenklick-/F3-Richtungen zeigen das richtige Ziel. | 45–49 |
| DE/Light, 760×678 | Beide Fokusrichtungen bestehen; linker Suchfokus und separate Markierung bleiben nach Pane-Wechsel korrekt. | 50–52 |
| F02 DE/Light | Alle acht Aktionen sind bei 1024 Punkten in zwei und bei 760 Punkten in vier Reihen lesbar. Eine Reihe wurde in dieser Sitzung nicht erreicht. | 48–53 |
| E04 EN/Dark | `New worktree config` ist im Plusmenü abgeschnitten; auch mit geöffnetem Untermenü fehlt ein vollständiger Titel/Tooltip (#711). Escape schließt die Menüs ohne neue Pane. | 31–35 |
| E04 DE/Light | `Worktree-Konfig` ist vollständig sichtbar; Untermenü und Escape funktionieren. Keine Konfiguration oder Repository angelegt. | 54–56 |
| Zustand/Dateien | Frische EN-Ausgaben erhalten PID, nicht exportierten Marker und CWD über den Menürückweg; alle vier Dateien danach unverändert. Frische DE-Ausgaben erhalten die neue PID und CWD über die Dateiaktionen. | 30, 35, 37, 47, 58 |

Die englischen Dateien wurden nach den Interaktionen unabhängig neu gehasht;
alle vier Größen und SHA256 stimmen mit den vorher festgelegten Werten überein.
Der nach dem deutschen Durchlauf eingegebene zweite Hashbefehl wurde vor Ablauf
der VM nicht mehr ausgeführt und zählt ausdrücklich nicht als Nachweis.

Das 30-Minuten-Fenster endete regulär mit `session-expired.txt`; der Operator wurde
geschlossen und die VM gestoppt. Der Workflow-Erfolg bezeichnet die verfügbare
Infrastruktur, keine vollständige Produktfreigabe. Eine spät eingeblendete
macOS-Mitteilung fing die versuchten Pane-Schließen-Klicks ab und öffnete die
Systemeinstellungen; diese eigene unbeabsichtigte Ansicht wurde geschlossen.
Darum sind 59/60 kein Beleg für eine einreihige Leiste oder einen Zaplex-Fehler.

Zwei weitere Werkzeugabweichungen sind in der Aktionsfolge getrennt dokumentiert:
Escape-Sequenzen beim ersten Fixture-Eingabeversuch und ein nicht bestätigter
Modifier-Klick. Erst korrigierte Eingabe bzw. frische Beobachtung und explizit
globale Modifier-Koordinaten liefern die oben bewerteten Ergebnisse.
Bewegung/Reduced Motion, weitere Sprach-/Theme-Kombinationen und die vollständige
Produktmatrix bleiben ungeprüft.

Die Korrektur für [#711](https://github.com/byte5ai/zaplex/issues/711) ist als
1.2.39 mit [PR #712](https://github.com/byte5ai/zaplex/pull/712) nach `main`
gemergt (`65bf8e463098367a4a6254a2724f27d390d89492`): vorhandener lokalisierter Tooltip oberhalb der Zeile, damit
das seitliche Untermenü den vollständigen Titel nicht verdeckt. Dies ist noch
kein nativer Nachweis der Korrektur.

Die [lokalen Prüfungen für 1.2.39](evidence/menu-711-local.json) sind bestanden:
Compile-Gate, vier bestehende Menü-/Tooltip-Regressionen ohne übersprungene Fälle,
App-Clippy mit `-D warnings` und 13 Phase-A-Prüfungen. Diese Prüfungen ersetzen
die native Sichtprüfung des neuen Tooltips nicht.

## Ergänzung: deutsche einreihige Leiste

Die gezielte [zweite Exploration 40251477019](https://github.com/byte5ai/zaplex/actions/runs/40251477019)
verwendete dasselbe unveränderte 1.2.38-DMG und denselben Quellstand. Vorab
festgelegt waren alle acht lesbaren deutschen Aktionen in einer gleichmäßig
aufgeteilten Reihe bei 1024 Punkten sowie frische Shell-Ausgaben vor und nach
dem Dateimanager. Bereits bestandene F03- und GUI-Fälle wurden nicht wiederholt.

Nach Auswahl von Deutsch/Light und vollständigem App-Neustart zeigt Original 11
bei **1024×678 Punkten** alle acht Zellen in einer Reihe: F2 Benennen, F3 Details,
F4 Editor, F5 Kopieren, F6 Schieben, F7 Ordner, F8 Löschen und F10 Terminal.
Keine Beschriftung ist abgeschnitten. F10 führt zurück in die Shell;
die tatsächlich ausgeführten Ausgaben 10 und 13 zeigen dieselbe PID `2742`
und dasselbe Arbeitsverzeichnis. **Diese F02-Variante und der Shell-Rückweg bestehen.**

Alle **15 Originalbilder** wurden einzeln betrachtet und bytegenau mit dem
CI-Artefakt `12621349954` abgeglichen. Das
[Manifest mit Aktionsfolge](evidence/candidate-138-native/exploration-40251477019.json)
und die [unveränderten Originale als ZIP](evidence/candidate-138-native/exploration-40251477019-originals.zip)
sind dauerhaft gesichert. Das heruntergeladene CI-ZIP hat SHA256
`d9c3f5803c0cc04f76d1fac75e60a11677ad625ef7251e6cfb57d98e22f5bee4`.
Der ausdrücklich gesetzte `finished`-Marker und `operator/closed` sind im
erfolgreich abgeschlossenen Lauf archiviert; diese Sitzung lief nicht ab.

Die Ergänzung liefert weder nachträgliche Datei-Hashes der ersten VM noch
Bewegungsnachweise oder eine native Sichtprüfung des 1.2.39-Tooltips.
