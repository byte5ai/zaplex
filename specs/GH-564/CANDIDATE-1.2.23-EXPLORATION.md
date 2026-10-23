# Installiertes DMG und reale Remote-Arbeit: 1.2.23

Die [GUI-Fortsetzung 39920461986](https://github.com/byte5ai/zaplex/actions/runs/39920461986)
bestand alle **15 verpflichtenden Fälle gemeinsam, ohne Fehler oder übersprungene Fälle**.
Geprüft wurde das signierte, notarisierte DMG aus
[39871910859](https://github.com/byte5ai/zaplex/actions/runs/39871910859),
App-Commit `bf52cf4b75033f48c4d3d945da26bdcaa803f323`,
Test-Commit `ba7c781e19f97e8d162fa669bad6ef4163888a5d`.
Der Testbeleg wurde gegen Fall-IDs, xcresult, Version und DMG-Digest geprüft.
Die anschließende direkte GUI-Exploration am 23.10.2026 fand zwei weitere Fehler;
die automatischen Fälle sind deshalb keine vollständige Release-Abnahme.

Die Exploration lief in Englisch/dunkel bei 1024×678 Punkten. Das
[Manifest](evidence/exploration-39920461986/manifest.json) bindet die Originalbilder
an den Kandidaten. Originalbilder und Aktionsprotokoll sind gesichert; sie enthalten
private Verbindungsdaten und werden hier nicht erneut veröffentlicht.

| Vorab festgelegte Erwartung | Beobachtung und Belege |
|---|---|
| Escape schließt das tatsächlich geöffnete Kopfzeilenmenü; dieselbe Shell bleibt erhalten | Bestanden: Menü in 07, geschlossen in 08, gleiche PID, nicht exportierter Marker und CWD in 11. |
| Importierter SSH-Host verbindet eine persistente Remote-Shell | Bestanden: Import und Verbindung 13–15, Shellzustand 17. Der Kandidat brachte seinen passenden Daemon mit. |
| Installierte Claude-Code- und Codex-CLIs antworten über den isolierten Testzugang | Beide Exit 0 und erwartete Antwort in 24; Ausgaben zusätzlich als Dateien gesichert. Das prüft API-Testprofile, keine Abonnements oder Kontingentrouten. |
| Verbindungsverlust erhält Remote-PID, nicht exportierten Marker und CWD | Bestanden: ausschließlich den eigenen SSH-ControlMaster beendet (23), automatische Wiederverbindung mit identischem Zustand (26). Die Providerbefehle waren vorher beendet; ein während des Abbruchs laufender Agent wurde hier nicht geprüft. |
| Dateimanager verwendet denselben importierten Host wie das Terminal | Fehlgeschlagen: wiederholter Verbindungsfehler in 27/28. Nur die diagnostische Änderung des gespeicherten Hosts auf seine bereits konfigurierte Zieladresse führte zur Schlüsselabfrage und Dateiliste (38–46). Fehler #639; dieser Workaround ist keine bestandene Alias-Abnahme. |
| Schlüsselbestätigung gehört zum angesprochenen Ziel | Angezeigter ECDSA-Fingerprint vor Bestätigung unabhängig über die vorhandene authentifizierte SSH-Verbindung geprüft (40/44/45). |
| Remote-Datei wird ins ausdrücklich gewählte lokale Fenster kopiert | Zielauswahl 50/51, Completed 52, Original weiterhin vorhanden. Lokaler Inhalt unabhängig mit der Remote-Ausgabe verglichen; identische 17 Bytes, SHA-256 `264790275b730734441d226074f9a73e4c4eeb36ed89f9f3e51d4a1cd8ebfbe3`. |
| Konfliktabbruch lässt vorhandene Datei unverändert | Zweites F5 zeigt Konflikt (53); Escape ergibt Skipped (54). Anschließender Inhaltsvergleich stimmt. |
| Ziel-Dateiliste zeigt die abgeschlossene Kopie ohne manuellen Refresh | Fehlgeschlagen: Zielansicht zeigt die neue Datei nicht (55), obwohl der Shell-Inhaltsvergleich sie nachweist (58). Fehler #640. |
| Rückkehr aus dem Dateimanager erhält die lokale Shell | F10 (56), gleiche PID, nicht exportierter Marker und CWD sowie Dateiprüfung (58). |

Eine Dock-Erweiterungsmeldung fing den ersten Klick oben rechts ab und öffnete
Systemeinstellungen. Erst die vollständige Bildschirmaufnahme zeigte dieses
überlagernde Systemfenster. Nach Schließen des dadurch geöffneten Fensters
funktionierte derselbe Menüeinstieg; es wurden keine Systemeinstellungen geändert.
Das erklärt nicht rückwirkend den alten Befund #604. Der aktuelle lokale und
entfernte Kopfzeilen-Einstieg funktionierte, daher wurde #604 mit diesem begrenzten
Nachweis geschlossen.

In vier separat geprüften Originalaufnahmen der automatisierten DMG-Suite liegen die neuen Panes jeweils unten, links, rechts beziehungsweise oben. Die sichtbaren Shellmarker unterscheiden neue und ursprüngliche Shell. Die Dock-Meldung verdeckt dabei Teile der oberen rechten Kopfzeile; diese verdeckten Bedienelemente gelten durch die Aufnahmen nicht als visuell geprüft. Bildnamen und Digests stehen getrennt im Manifest. Das ist eine Sichtprüfung automatisierter Zustände, kein Ersatz für die direkte Split-Exploration.

Die weiteren vorab vorgesehenen Aufgaben – sämtliche lokalen Dateiaktionen,
Settings-Korrektur und Split-Erzeugung in dieser direkten Exploration – sind durch
diesen Lauf nicht vollständig abgedeckt. Separate automatisierte/native Belege
werden nicht nachträglich als vollständige Exploration ausgegeben. Abo-Funktionen
bleiben ausdrücklich außerhalb des isolierten Testzugangs.

Das Bedienfenster wurde regulär beendet. Der Job meldet erfolgreiche Bereinigung
der eigenen Prozesse und des mitgebrachten Remote-Testziels. Der Kandidat 1.2.23
enthält #639 und #640; er wurde nicht veröffentlicht. PR #641 korrigiert #639 auf
`main`; seine native Bestätigung benötigt das nächste DMG.
