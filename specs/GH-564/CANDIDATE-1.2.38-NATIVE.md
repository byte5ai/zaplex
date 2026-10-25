# Native Abnahme des Kandidaten 1.2.38

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).

App- und Testquellstand: `2434f6cd84ec3db40769c5b351e28279b6010d7c`,
[PR #710](https://github.com/byte5ai/zaplex/pull/710). Dieser Stand korrigiert
#709: Dateizeilenklicks aktivieren ihre Pane und geben der Liste nach Suchfokus
den Tastaturfokus zurück, ohne Markierungen zu löschen.

## Lauf und Aussagegrenze

Der freigegebene [CI-Batch 40242122433](https://github.com/byte5ai/zaplex/actions/runs/40242122433)
wurde am 25.10.2026 um 22:51 UTC mit Test-DMG gestartet. Die Vorprüfung,
Abhängigkeitsprüfung und Compile-Jobs für Linux und macOS sind bestanden.
Die gezielten Plattform-/Keymapprüfungen und Live-SFTP sind ebenfalls bestanden.
Die Linux-Gesamtsuite meldet **4438 bestanden, 13 übersprungen**; die
Dokumentationstests **4 bestanden, 4 ignoriert**. Diese Ausnahmen werden nicht
als ausgeführte Erfolge gezählt. Phase A, der App-Bau und der Bau des gebündelten
Linux-Daemons sind bestanden. Signieren/Paketieren und installierte GUI-Abnahme
sind noch nicht als erfolgreich nachgewiesen.

Die neun gezielten lokalen Regressionstests, Compile-Gate und App-Clippy sind
bereits im [lokalen Nachweis](evidence/focus-709-local.json) dokumentiert und
werden nicht erneut lokal ausgeführt. Der vorherige F03-Fehler auf 1.2.37 bleibt
bis zur Prüfung des neu gebauten DMG ein nativer Fehlernachweis.

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
