# Mehrfachauswahl, Löschen und Transferziel auf installiertem 1.2.27

Die [Exploration 40059647737](https://github.com/byte5ai/zaplex/actions/runs/40059647737)
verwendet die unveränderte DMG aus [Batch 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378).
App-Quelle: `b270ee372f8548b83363916dc8fbddc1ee7b5884`; Harness:
`6927cdbfdede5c18165dfa302393a5ee00755922`; EN/dunkel, App-Aufnahmen 1024×678 Pixel.
DMG-SHA256: `fcb082266c6b4f61fb8bb0c3d871e9ccafb8c229225e528f288bafb36f800644`.
Die Erwartungen wurden vor den Aktionen aus [F06/F07/F10](MATRIX.md) festgehalten.
Alle Daten sind eigens angelegte lokale Fixtures in der kurzlebigen Testumgebung.

| Ablauf | Beobachtung und unabhängiger Nachweis | Aufnahmen |
|---|---|---|
| Ordner und Datei mit Space markieren | Getrennte Zählung für Ordner/Dateien; Bytezahl ausdrücklich ohne Ordnerinhalt. Markierungen und Dateicursor bleiben beim absteigenden Sortieren und Refresh am selben Objekt. | 005–008 |
| Cmd+A | Markiert einen Ordner und zwei Dateien; Elternverzeichniseintrag bleibt ausgenommen. | 009 |
| F8 und Abbrechen | Dialog nennt einen Ordner einschließlich Inhalt und zwei Dateien. Markierungen bleiben erhalten; unabhängige Shellprüfung bestätigt alle drei ursprünglichen Datei-Hashes. | 003, 010–011, 014 |
| Gewähltes Transferziel wechselt Verzeichnis | Ziel in anderem Tab folgt dessen Navigation in einen Unterordner. F5 kopiert die gesamte markierte Auswahl; unabhängige Hashes bestätigen beide Dateien und den verschachtelten Ordnerinhalt. Quellen bleiben erhalten. | 017–021, 024 |
| Ziel-Dateipane kehrt zur Shell zurück | Eine dritte kompatible Dateipane existiert. Trotzdem wird das bisherige Ziel ungültig; F5 verlangt eine neue explizite Wahl. Abbrechen lässt die Alternative leer, unabhängig per Shell geprüft. | 025, 027–029, 034 |
| Teilmenge tatsächlich löschen | Ein Ordner und eine Datei markiert, zweite Datei unmarkiert. Dialog zählt korrekt; nach Bestätigung fehlen genau die beiden ausgewählten Objekte samt Ordnerinhalt. Unmarkierte Datei behält ihren Hash. | 030–034 |
| Transferanzeige | **Fehler [#658](https://github.com/byte5ai/zaplex/issues/658):** dieselben drei Transfers erscheinen gleichzeitig im Dateimanager- und Workspace-Overlay, auch in einer unbeteiligten Dateipane. Die unteren Aktionen werden verdeckt. Schließen blendet beide Anzeigen aus. | 021, 025–026 |

Die erwarteten Inhalts-Hashes wurden vor den Produktaktionen erfasst:

| Fixture | SHA256 |
|---|---|
| `alpha.txt` | `8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8` |
| `beta.txt` | `f44e64e75f3948e9f73f8dfa94721c4ce8cbb4f265c4790c702b2d41cfbf2753` |
| `folder/child.txt` | `ddc9e669194254cef019a29d3619a2c16592e5d52e1a81e98b01bd52319149a3` |

Das [Manifest](evidence/candidate-127-local-multi/manifest.json) bindet alle 35
einzeln gesichteten Originalaufnahmen. Das Fenster wurde ausdrücklich beendet;
Workflow und Cleanup sind erfolgreich abgeschlossen. Kein Release oder Tag.

Nicht geprüft sind Maus-/Bereichs-/Insert-Markierungen, das tatsächliche
Schließen des Ziel-Tabs, Split-Pane-Varianten, Deutsch und helles Theme.
Die einzelnen bestandenen Abläufe ersetzen diese Varianten und die Gesamtmatrix nicht.
