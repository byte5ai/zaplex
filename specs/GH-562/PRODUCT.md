# Nutzertests am ausgelieferten macOS-Artefakt

Auftrag: [#562](https://github.com/byte5ai/zaplex/issues/562). Die GUI-Abnahme
prüft das installierte DMG. Ein erfolgreicher Code-Review ersetzt keinen Ablauf.

## Abnahmekriterien vor Testausführung

1. Start, Befehlsausgabe, neues Fenster, nativer Schließen-Button und Neustart
   bleiben verpflichtend. Jede Ausgabe muss durch einen ausgeführten Befehl
   entstehen; der erwartete Text darf nicht bereits in der Eingabe stehen.
2. Ein neuer Tab besitzt eine eigene Shell. Nach Wechsel zurück und nach
   Schließen des neuen Tabs bleibt eine nicht exportierte Variable der ersten
   Shell erhalten. Frische Ausgabemarker verhindern Treffer in altem Scrollback.
3. Split rechts, links, unten und oben erzeugen jeweils eine eigenständige
   Shell in der gewählten Richtung. Die erste Shell bleibt erhalten. Sichtbare
   Ausgabe und ihre Position belegen die Richtung, nicht ein interner Pane-Zähler.
4. Einstellungen öffnen sichtbar; nach Schließen nimmt die ursprüngliche
   Shell Eingaben an und enthält ihre vorher gesetzte Variable.
5. Ein geöffnetes Startmenü lässt sich mit Escape abbrechen, ohne eine Shell
   zu ersetzen. Menüinhalt folgt `docs/ui/pane-launch-and-tab-join.html#spec`.
6. Der Release-Workflow veröffentlicht das DMG erst nach erfolgreicher
   vollständiger lokaler GUI-Suite an genau diesem Artefakt. Diagnoseteilmengen,
   ausgelassene Tests, fehlende Ergebnisse oder Zeitüberschreitungen reichen nicht.
7. Screenshots, OCR, Accessibility-Baum, Testergebnis, Test-Revision und
   DMG-Digest bleiben erhalten. Prüfbelege sind keine Release-Installer.
8. Der Dateimanager öffnet im Arbeitsverzeichnis des aktiven Pane, zeigt eine
   dort angelegte Testdatei und die beschrifteten Dateiaktionen. Zurückwechseln
   erhält die Shell. Das Öffnen/Schließen von „Connections“ erhält sie ebenfalls.
9. Bei höchstens 700 Punkten Fensterbreite bleiben alle acht beschrifteten
   Dateimanager-Aktionen sichtbar, wie in `docs/ui/README.md` vereinbart.
   Das echte Fenster wird dafür über das native Menü „Move & Resize → Left“
   schmaler gemacht; ein bloßes Browser-Mockup reicht nicht.

## Vollständige Produktabnahme

Die lokale Suite ist ein Teil der Abnahme. Zusätzlich erforderlich sind echte
Remote-Verbindung und Wiederaufnahme nach Netzverlust, Dateiübertragung mit
Inhaltsprüfung, lokale und entfernte Claude-/Codex-Sitzungen sowie ein unabhängiger
explorativer Agent mit Zugriff auf die laufende GUI. Fehlende Ausführung wird
explizit als `not-run` ausgewiesen, niemals als bestanden.

Visueller Sollmaßstab sind die freigegebenen Referenzen in `docs/ui/README.md`,
insbesondere `cockpit-sessions-tree.html`, `premium-workspace.html` und
`pane-launch-and-tab-join.html`. Ein Screenshot des aktuellen Builds ist ein
Ist-Beleg, keine automatisch genehmigte Sollreferenz. Normalbreite, schmale
Fenster, leere Zustände und Fehlerzustände gehören zum Vergleich.

Isolierte Remote-Testzugänge und beide CLI-Provider mit begrenzten API-Testprofilen
sind inzwischen [am installierten 1.2.23-DMG nachgewiesen](../GH-564/CANDIDATE-1.2.23-EXPLORATION.md).
Dieser Nachweis umfasst keine Abonnements, Kontingente oder Verteilung auf das
freieste Abo. Ein noch laufender Claude-Prozess überlebt inzwischen die reale
Transporttrennung im [1.2.27-Teilnachweis](../GH-564/CANDIDATE-1.2.27-AGENT-THEME.md);
das API-Testprofil beweist weiterhin kein Abonnement. Die späteren Befunde
#644/#645/#646 sind korrigiert und in ihren jeweiligen Kandidatenberichten
nachgeprüft. Der [Kandidat 1.2.32](../GH-564/CANDIDATE-1.2.32-NATIVE.md) besteht
den vollständigen CI-Batch und 15 lokale GUI-Pflichtfälle. Die Exploration
bestätigt DE/EN-Theme-Fokusrückgabe und sichere CWD-Übernahme bei einer belegten
lokalen Shell, findet aber den initialen Dialogfokusfehler #673. Weitere
Sprach-, Remote-, Verbindungs- und Providerkombinationen sind noch ungeprüft.
Bis die zusätzlichen Prüfungen ausgeführt und bewertet sind, bleibt #562 offen;
ein lokaler Erfolg allein ergibt keine vollständige Produktabnahme.
Die hier implementierte Liefersperre erzwingt die lokale GUI-Suite; sie behauptet
keine vollständige Remote-, Design- oder explorative Abnahme.
