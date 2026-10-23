# Native Sichtprüfung des Kandidaten 1.2.25

[Batch 39935546650](https://github.com/byte5ai/zaplex/actions/runs/39935546650)
prüft App-Commit `3592d05e6ed09c46a41bf4421ae948744ee4813e`.
Alle **42 Originalaufnahmen** des nativen Integrationsbuilds vom 23.10.2026,
10:04 UTC wurden einzeln betrachtet. Das ist noch keine installierte DMG-Abnahme.
Die Erwartungen bleiben die bestehende [Prüfmatrix](MATRIX.md) und die
[freigegebenen Referenzen](../../docs/ui/README.md); die Bilder setzen keine neue
Sollreferenz. Das [Manifest](evidence/candidate-125-native/manifest.json) nennt
Quelle, tatsächliche Bildgröße und Digest jedes Originals.

| Bereich | Geprüfte Varianten | Beobachtung |
|---|---|---|
| Agentenansicht | 3 Bilder, 1180×640 und 540×670, EN/dunkel | Freigabeaktionen getrennt; beendete Sitzung bietet neue Unterhaltung und Shell-Rückweg. Recovery zeigt Resume, Neustart, Ende und Änderungsaktionen. Lange Identitäten/Fehlertexte werden gekürzt; volle Tooltip-Texte hier nicht geprüft. |
| Gefüllte Verbindungen und Favoriten-Flyout | 7 Bilder, Sidebar 250/320/480 bei 1440×640, Hell/Dunkel; Flyout bei 900×640 | Hierarchie und feste Aktionsplätze erkennbar; Metadaten umbrechen. Sehr lange Namen werden gekürzt. Das Flyout öffnet am rechten Fensterrand nach links und erhält das Elternmenü mit drei Folgeaktionen. |
| Leere Verbindungen/Favoriten | 16 Bilder, DE/EN, Hell/Dunkel, Sidebar 250/320/480 | Vollständiger Hinweis auf Ordner und Plus auch bei 250 Punkten. Favoritenhinweis einschließlich Sternaktion bleibt vollständig und überlagert keine Folgeaktion. |
| Zwei Dateimanager | 14 Bilder, DE/EN, Hell/Dunkel, 1180×640 und 760×640 | Acht beschriftete Aktionen in zwei bzw. vier Reihen. Aktive und inaktive Pane unterscheidbar; voller Dateiname im Tooltip. Ordnerdialog erscheint in der aktiven Pane; Rückkehr zeigt beide Terminals und erhaltenen Entwurf. **Abweichung:** Filterhinweis verschwindet nach Dark→Light (#644). |
| Pane-Anordnung | 2 Bilder, EN/dunkel, 1180×640 | Unterschiedliche Pane-Identitäten bei gleichem CWD; nach Drag vertikale Anordnung, erhaltener Entwurf und expliziter Tabtitel. |

## Abweichung: Editorfarben nach Themewechsel

[#644](https://github.com/byte5ai/zaplex/issues/644), vorläufig **P2**: In beiden
Sprachen ist der Filterhinweis im dunklen Theme lesbar und nach Dark→Light in
normaler wie schmaler Ansicht nicht mehr sichtbar. Der Feldhintergrund und die
Dateiliste wechseln korrekt. Die Originale `native-file-managers-normal.png` /
`native-file-managers-normal-light.png` und die beiden `narrow`-Varianten belegen
den Unterschied für beide Dateimanager-Fixtures.

`make_editor` übernimmt Editor-Farb-Overrides beim Anlegen aus dem damaligen
Theme. Die Auswirkungen auf eingegebenen Text, Rename-/Ordnerfelder und das
erneute Öffnen eines Dateimanagers wurden in diesen Bildern nicht beobachtet;
dafür ist eine direkte Prüfung am installierten DMG vorgesehen. Aus dem
Screenshot-Befund wird keine vollständige Aussage über diese Eingaben abgeleitet.

## Aussagegrenzen

Die Integrationsfälle verwenden konstruierte Host-/Session-/Agentenzustände.
Sie belegen Darstellung und die zugehörigen Testassertions, keine echten
Provider-Anmeldungen, Kontingente oder Remote-Persistenz. Gekürzte Zielpfade sind
allein kein Nachweis einer eindeutigen echten Transferwahl. OS Reduced Motion,
alle Drag-/Restore-Varianten und vollständige Tooltips langer Identitäten sind
hier nicht abgedeckt. Die tatsächlichen Alias-Verbindungen und Dateiübertragungen
von 1.2.25 wurden anschließend separat am installierten DMG geprüft; Ergebnisse
und Grenzen folgen unten.

Die Originale sind zusätzlich archiviert. Rohbilder mit temporären Laufzeitpfaden
werden hier nicht erneut veröffentlicht; die beiden Actions-Artefakte und das
Manifest ordnen sie eindeutig zu.


## Installierte Pflichtfälle und reale Exploration

Der oben verlinkte Batch ist abgeschlossen und erfolgreich. Die installierte
GUI-Suite besteht **15/15 Pflichtfälle, ohne Fehler oder übersprungene Fälle**.
Der Linux-Testlauf besteht 4409 Tests bei 13 übersprungenen Tests. Das signierte
und notarisierte DMG hat SHA-256
`5651663b7c8330af56f2a5b9e0f5df75a912134a6ab1624ff5ccf586bd9ebe40`.
Die reine Veröffentlichungsvorbereitung validiert Quellstand, GUI-Beleg und
Artefaktdigests; ein Release oder Tag wurde nicht angelegt.

Die [reale Exploration 39944384551](https://github.com/byte5ai/zaplex/actions/runs/39944384551)
verwendet genau dieses Artefakt. Der Job einschließlich Aufräumen ist beendet;
sein grüner Infrastrukturstatus ist keine vollständige Produktabnahme.

- **Alias-Verbindung (#639):** Importierter SSH-Alias verbindet Terminal und
  Remote-Dateimanager ohne manuelle Ersetzung durch eine IP-Adresse. Der separat
  angebotene Hostschlüssel wurde über die bereits verifizierte SSH-Verbindung
  unabhängig abgeglichen.
- **Falsche Pane (#645):** Bei aktivem lokalen Dateimanager öffnet ein Klick auf
  das Ordnersymbol der anderen, entfernten Terminal-Pane stattdessen einen
  lokalen Dateimanager in der aktiven Pane mit dem entfernten Pfad. Nach
  ausdrücklicher Fokussierung der entfernten Pane funktioniert derselbe Klick.
- **Lokaler Verzeichnisalias (#646):** Ein Download derselben 26-Byte-Datei in
  einen lokalen Verzeichnisalias scheitert mit lokalem E/A-Fehler. Nach Wechsel
  zum kanonischen Pfad desselben Verzeichnisses gelingt er. Die identische
  Verzeichnisidentität wurde über die Inode-Nummer geprüft.
- **Zielaktualisierung (#640):** Beim erfolgreichen kanonischen Download
  erscheint die Datei unmittelbar im Ziel, ohne Fokuswechsel oder manuelles
  Aktualisieren; Verzeichnis und vorherige Cursorzeile bleiben erhalten.

Ein unabhängiger Inhaltsvergleich der aktuellen Kopie wurde vor Ende des
Bedienfensters nicht mehr ausgeführt. Gegenrichtung, Verzeichnistransfers,
Konflikt-/Abbruchkombinationen, ein während der Transporttrennung laufender
Agent und die Auswirkungen von #644 auf eingegebenen Text sind damit nicht
nachgewiesen. Die Exploration ersetzt auch keine Prüfung echter Abonnements.

## Fortsetzung nach der angeforderten Neustartpause

Die Arbeit pausiert auf Wunsch des Nutzers am 23.10.2026 nach abgeschlossenen
Jobs. Produktionscode für #644/#645/#646 wurde noch nicht geändert.

1. #645: Die aufrufende Pane-ID durch `WorkspaceAction::OpenLocalFileManager`
   mitführen. Die verzögerte Hostauflösung und Pane-Ersetzung beibehalten;
   ein zwischenzeitlicher Fokuswechsel darf das Ziel nicht ändern. Regression
   mit inaktiver Remote-Pane neben aktivem lokalem Dateimanager absichern.
2. #646: Den lokalen Streaming-Writer und die Auflösung des Elternverzeichnisses
   prüfen. Die vorhandene Behandlung kanonischer Eltern bei Rename als
   Referenz verwenden; Schutz vor Symlink-Austausch und Überschreiben erhalten.
   Regression mit Alias und kanonischem Pfad desselben Verzeichnisses prüfen.
3. #644: Bestehende Theme-Änderungsbenachrichtigung auch für die drei
   Dateimanager-Editoren verwenden. Text, Auswahl und Fokus beim Wechsel
   erhalten und unabhängig prüfen.
4. Gezielte Tests und Compile-Gate, dann einen Kandidaten mit den Korrekturen
   bauen. Bereits grüne unveränderte Teile wiederverwenden. Die fehlenden
   Produktnachweise in einem neuen Bedienfenster vervollständigen; das alte
   Fenster ist beendet. Vor einer Veröffentlichung ist die konkrete
   Release-Freigabe erforderlich.
