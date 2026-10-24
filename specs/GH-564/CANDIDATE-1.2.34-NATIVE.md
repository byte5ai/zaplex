# Kandidat 1.2.34: gezielte native Nachprüfung

App-Quellstand: `b0eff3ee326b7a4884de3681123cd84fad5ef45f`.
Der [Batch 40087073327](https://github.com/byte5ai/zaplex/actions/runs/40087073327)
enthält den Kandidaten mit den Korrekturen #672/#673/#674. Build, Sicherheits-
und Paketprüfungen (4430 bestanden), Signierung und Notarisierung sind bestanden.
Die installierte GUI-Suite besteht zunächst 14/15 Fälle. `testSplitDown` scheitert
bereits im Setup: Das Originalbild zeigt die richtige Nonce `149060`, Tesseract
liest `149960`. Das ist #680; der Split wird noch nicht erreicht.

Nach der reinen Testkorrektur #681 besteht ausschließlich der fehlgeschlagene
Fall im [Fortsetzungslauf 40092623416](https://github.com/byte5ai/zaplex/actions/runs/40092623416).
Auch die unabhängigen OCR-Gegenproben bestehen: echte Ausgabe, Ablehnung falscher
Nonce/falschen Zustands und des noch nicht abgeschickten Befehls sowie korrekte
Koordinaten. Testrevision: `4c1f203c1ce3c66d0923f950fd1d78770eacda94`.

Beide Läufe verwenden dasselbe unveränderte DMG mit SHA-256
`ec3ce86d7fdf78c5dc5852f5b8c717c3252428450199b5a1929cebe602ec5319`.
Damit sind alle 15 lokalen Pflichtfälle belegt, verteilt auf zwei Läufe; der
ursprüngliche Batch bleibt fehlgeschlagen und die Fortsetzung bleibt ein
Teillauf. Die kombinierte Herkunftsprüfung aus #682/#683 besteht im Vorprüfjob der
[Exploration 40093237854](https://github.com/byte5ai/zaplex/actions/runs/40093237854);
der zusätzliche Bericht ordnet alle 15 Fälle ihrem tatsächlichen Lauf zu.

## Erzeugtes natives Menü-Fixture

Das [Originalbild](evidence/candidate-134/native-favorite-flyout.png) wurde
individuell betrachtet; Quellstand und alle 41 Workspace-Bildhashes stimmen
mit dem heruntergeladenen Artefakt 12557931513 überein. Nur dieses eine Bild
wird hier als visuell geprüft bewertet; das
[Manifest](evidence/candidate-134/fixture-review.json) hält die Zuordnung fest.

EN/Light, 900×640: Der vollständige Hinweis „More actions for
production-europe-west-development-0123456789“ bleibt über dem nach links
öffnenden Aktionsmenü lesbar und innerhalb des Fensters. Auf 1.2.32 wurde
er an dieser Stelle verdeckt. Der Hinweis liegt vorübergehend über der
ersten Aktionsbeschriftung; das Bild belegt weder Mausdurchlässigkeit noch
stabiles Hover-Verhalten oder Tastaturbedienung. Diese Punkte sowie die
Dialogfokus- und Sprachkorrektur werden am installierten DMG separat geprüft.


## Installierte Exploration: Englisch, dunkel und hell

[40093237854](https://github.com/byte5ai/zaplex/actions/runs/40093237854),
Bedienrevision `6b8a99e9a7312873d4943e4a334b56cd34771d3a`.
Alle 80 Originalaufnahmen wurden einzeln betrachtet und bytegenau mit Artefakt
12560781025 verglichen. Das [Manifest](evidence/candidate-134-installed/review.json)
erfasst tatsächliche Fensterrechtecke, Theme, Hash und Beobachtung pro Bild;
elf Schlüsselbilder liegen zusätzlich im Repo. Der Lauf wurde mit eigenem
`finished`-Marker beendet, ohne Zeitablauf; Aufräumen und Belegsicherung bestanden.

| Prüfung | Tatsächliches Ergebnis |
|---|---|
| #673 F2, EN/Dark und EN/Light | Alter Name vollständig ausgewählt; ein rohes `z` ersetzt ihn ohne Feldklick. Light folgt nach Settings-/Theme-Rundweg. Bilder 008–010, 031–032. |
| #673 F7 und Werkzeugleisten-Ordner, EN/Dark und EN/Light | Leeres Feld; erster roher Tastendruck kommt direkt an. Abbrechen erhält Dateien. Bilder 013–018, 033–034. |
| Sofortiger Escape nach F2 | **Fehler #684:** erster Escape löscht nur die Anfangsauswahl, Dialog bleibt; zweiter Escape schließt. Bilder 019–021. Die versehentlich als F7 benannten Bilder 011–012 zeigen ebenfalls noch den Rename-Dialog, keine bestandene F7-Prüfung. |
| #672 Favorit, EN/Light, rechts öffnend bei 1024×678 | Vollständiger Hinweis über dem Flyout, drei stationäre Beobachtungen stabil; Escape schließt nur die Unterebene, Bearbeiten bleibt erreichbar und verwendet vorhandenen Editor. Bilder 043–048. |
| #672 Favorit, EN/Light, links öffnend bei 650×678 | Vollständiger Hinweis innerhalb der Fenstergrenzen, drei stationäre Beobachtungen stabil; Bearbeiten erreichbar. Bilder 063–067. |
| Primärer Favoritenname | Separater Klick startet einen sichtbaren SSH-Versuch zum künstlichen Loopback-Ziel, endet mit tatsächlichem Connection refused. Bilder 068–070. |
| #674 englische Agent-Einstellungen | Supportfrage und Link korrekt bei 1024×678. Deutsche Nachprüfung noch nicht durchgeführt. Bild 022. |
| Schmale Agent-Einstellungen | **Fehler #685:** Bei 500×500 werden Texte/Schalter rechts abgeschnitten. Auch 350×350 und 100×100 sind erreichbar; daraus folgt keine behauptete absolute Mindestgröße. Bilder 024–026. |

Beim Startmenü mit drei Tabs öffnete sich über den Bedienweg kein sichtbares
Menü (049–058). Nach Rückkehr zu einem Tab funktionierte es wieder (059–061).
Das ist noch keine bestätigte Produktursache; die linke Öffnungsrichtung wurde
unabhängig bei einem Tab und 650 Punkten geprüft. Ebenso muss der zunächst
fehlgeschlagene Fokuswechsel vom Host-Editor zur benachbarten Dateimanager-Pane
gegen die Foreground-Aufrufe des Bedienwerkzeugs abgegrenzt werden (070–077).
Ein Prüfbefehl landete dabei lediglich als ungespeicherter Host-Feldentwurf;
er wurde weder abgeschickt noch gespeichert. Der ursprüngliche Wert wurde
wiederhergestellt, Save war wieder deaktiviert. Der abschließende Befehl wurde
erst nach sichtbarer Rückkehr ins Terminal genau einmal abgeschickt.

[Die frische Abschlussausgabe](evidence/candidate-134-installed/079-final-shell-proof.png)
bestätigt dieselbe PID 1268, den nicht exportierten Marker, dasselbe Arbeitsverzeichnis
und beide unveränderten Datei-Hashes. Es bleiben genau `alpha.txt`, `beta.txt`
und `child`; keine Rename-/Create-Aktion wurde bestätigt. Die Zustandsfolge belegt
abgetastete Hover-Zustände, keine lückenlose Animation oder vollständige
Mausdurchlässigkeit. Deutsch, dunkle Favoritenmenüs und weitere Matrixvarianten
sind hier nicht mitgeprüft.
