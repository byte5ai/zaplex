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
| #674 englische Agent-Einstellungen | Supportfrage und Link korrekt bei 1024×678. Deutsch wurde in diesem Lauf nicht geprüft; siehe Folgelauf unten. Bild 022. |
| Schmale Agent-Einstellungen | Im Ausgangsausschnitt sind bei 500×500 Texte/Schalter rechts abgeschnitten. Die Nachprüfung unten bestätigt horizontales Scrollen bei 500×500 und 350×350; #685 ist auf den tatsächlich erreichbaren 100×100-Grenzfall eingegrenzt. Bilder 024–026. |

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
wurden in diesem englischen Lauf nicht mitgeprüft.


## Installierte Exploration: Deutsch nach Neustart

[40096737284](https://github.com/byte5ai/zaplex/actions/runs/40096737284),
Bedienrevision `9161ff4f060997399fcd0cd126a5335dd6f3745a`, dasselbe unveränderte
1.2.34-DMG. Die vorhandenen 15 GUI-Pflichtfälle wurden anhand ihrer Herkunft
geprüft und nicht erneut ausgeführt. Alle 69 aufgenommenen Originalbilder
wurden einzeln betrachtet und bytegenau gegen Artefakt 12562220790 geprüft;
[das Manifest](evidence/candidate-134-german/review.json) erfasst jeden tatsächlichen
Bildzustand, Sprache, Theme, Fensterrechteck und Hash. Neun Schlüsselbilder liegen
im Repo. Der eigene Abschlussmarker beendete die Sitzung vor dem Zeitlimit;
Aufräumen und Belegsicherung bestanden.

| Prüfung | Tatsächliches Ergebnis |
|---|---|
| Sprache | Deutsch gewählt und App vollständig beendet/neugestartet, bevor die Shell-Kontrollwerte angelegt wurden. Die abgewiesene Hintergrund-Neustartanforderung führte zu keinem Start; der anschließende Vordergrundstart ist in Bild 007 sichtbar. Für Nummer 006 entstand kein Bild. |
| #673, DE/Dark | Gefüllter Filter bleibt erhalten; F2 markiert `alpha.txt`, ein rohes `z` ersetzt es. F7 und Werkzeugleisten-Ordner nehmen jeweils das erste rohe `z` ohne Feldklick an. Entwürfe abgebrochen. Bilder 013–020. |
| #673, DE/Light | Dieselben drei Einstiege funktionieren nach Settings-/Theme-Rundweg; das Filterkriterium bleibt erhalten. Bilder 037–042. Kein Nachweis für den erst später korrigierten sofortigen Escape mit Anfangsauswahl. |
| #674, DE/Dark und DE/Light | „Eine Eingabe wurde falsch erkannt? Melde uns den Fehler“ ist vollständig übersetzt und bei 1024×678 lesbar. [Dunkel](evidence/candidate-134-german/021-de-dark-agent.png), [hell](evidence/candidate-134-german/036-de-light-agent-top.png). Der Link wurde nicht aktiviert. |
| #672, DE/Light | Rechts öffnend bei 1024×678 und links öffnend bei 650×678 bleibt der vollständige lange Hinweis über dem Untermenü sichtbar. Je zwei stationäre Aufnahmen. Ein Escape schließt rechts nur die Unterebene; nach erneutem Eintritt führt Bearbeiten zum vorhandenen Editor zurück. Bilder 049–057, [linke Öffnung](evidence/candidate-134-german/056-de-light-left-flyout.png). |
| #672, DE/Dark | Beide Öffnungsrichtungen behalten den vollständigen Hinweis innerhalb des Fensters, je zwei stationäre Aufnahmen. Bilder 062–065, [linke Öffnung](evidence/candidate-134-german/064-de-dark-left-flyout.png). Keine separate primäre Verbindung in diesem Lauf. |
| #685, schmale Einstellungen | Horizontales Scrollen macht die Schalter bei [500×500](evidence/candidate-134-german/023-de-dark-agent-scroll-right.png) und 350×350 erreichbar (023/028). Die ursprüngliche Behauptung verlorener Inhalte bei diesen Breiten ist widerlegt. Bei [100×100](evidence/candidate-134-german/029-de-dark-agent-100.png) bleibt praktisch keine bedienbare Fläche; #685 untersucht jetzt diesen Grenzfall. Keine behauptete absolute Mindestgröße. |
| Weitere Übersetzungen | [Bild 035](evidence/candidate-134-german/035-de-light-agent.png) zeigt englische Hilfe zur Spracheingabe, „Restore default“, „Left side“, „Right side“ und „Show & collapse“ trotz deutschem Neustart. Separat erfasst in #688; #674 betrifft ausschließlich den korrekt übersetzten Erkennungshinweis. |

[Die frische Abschlussausgabe](evidence/candidate-134-german/069-final-check-output.png)
bestätigt dieselbe PID 1832, denselben nicht exportierten Kontrollwert und dasselbe
Arbeitsverzeichnis. Beide Datei-Hashes stimmen mit dem Anfang überein; das
Verzeichnis enthält genau `alpha.txt`, `beta.txt` und `child`. Kein Rename-/Create-
Entwurf wurde bestätigt. Der erste Klick auf F10 in der benachbarten inaktiven
Dateipane aktivierte diese zunächst; der folgende F10-Tastendruck stellte die
ursprüngliche Shell wieder her (066/067). Diese Beobachtung ist noch nicht von
der Fokusbehandlung des Bedienwerkzeugs unabhängig eingegrenzt.

Die Aufnahmen zeigen abgetastete Hover-Zustände, keine lückenlose Animation
oder vollständige Mausdurchlässigkeit. Die native Kombination EN/Dark für die
Favoritenmenüs und weitere Matrixfälle bleiben separat zu prüfen. #684 ist mit
PR #687 (`37242850ce6ae4d3a57c173cf3ff6dab780481f4`, 1.2.35) inzwischen korrigiert;
sechs gezielte lokale Tests und Clippy bestehen. Dieser 1.2.34-Lauf kann die
native Korrektur nicht bestätigen.
