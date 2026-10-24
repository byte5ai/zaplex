# Deutsche Verbindungsverwaltung auf installiertem 1.2.27

Die [Exploration 40063121804](https://github.com/byte5ai/zaplex/actions/runs/40063121804)
verwendet die unveränderte DMG aus [Batch 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378).
App-Quelle: `b270ee372f8548b83363916dc8fbddc1ee7b5884`; Harness:
`e31eb877c74dbcdc0dce7f66cfa5104ea183f419`.
DMG-SHA256: `fcb082266c6b4f61fb8bb0c3d871e9ccafb8c229225e528f288bafb36f800644`.
App-Aufnahmen: 1024×678 Pixel, Sidebar etwa 250 Punkte, dunkles Theme.
Ab Aufnahme 007 Deutsch nach angekündigtem Neustart der eigenen Test-App;
001–006 zeigen den englischen Start und Sprachwechsel.

Die Erwartungen für C02/C03/C04/V02 wurden vor den Aktionen festgehalten.
Host, Ordner, Name und Passwort sind synthetische Testdaten; die Verbindung
geht ausschließlich an einen nicht lauschenden Port der lokalen Testumgebung.

| Ablauf | Ergebnis | Aufnahmen |
|---|---|---|
| Sprache ändern | Suche findet Appearance/Language; deutscher Dialog erläutert nötigen Neustart. Danach sind Sidebar, Formulare und Terminal deutsch. Kein Nachweis einer Shellzustandserhaltung bei diesem Neustart. | 001–007 |
| Verbindungen verstehen | Icon-Tooltip mit Ctrl+5 und vollständige Leerzustandsanleitung einschließlich Ordner und Plus sind lesbar. | 008–010 |
| Ordner und Host anlegen | Inline-Ordnername bestätigt; leerer Server öffnet einen Editor neben der bestehenden Terminal-Pane. | 011–014 |
| Ungültigen Host korrigieren und speichern | `invalid host` wird verständlich abgelehnt, Hostfeld fokussiert. Korrektur zu `127.0.0.1` entfernt Fehler; Port 9, Benutzer und Gruppe gespeichert. Genau ein Host im richtigen Ordner; Speichern danach deaktiviert. | 015–020 |
| Verbindung testen | Ohne Passwort nur lokale Authentifizierungsprüfung fehlgeschlagen, kein Netzwerknachweis. Mit synthetischem Passwort tatsächliches `Connection refused`; verständliche deutsche Meldung, Offline. | 021–025 |
| Favoriten und Bearbeiten | Stern-Tooltip lokalisiert; Setzen projiziert Host ins Startmenü. Bearbeiten fokussiert bestehenden Editor, ohne doppelte Konfiguration. Entfernen zeigt leeren Favoritenbereich und behält Host unter Weitere Hosts. | 027–035 |
| Verbindungsfehler im neuen Tab | Weitere Hosts öffnet neues Remote-Tab, ursprünglicher Editor und lokale Shell bleiben erhalten. Verbindung wird tatsächlich abgelehnt. Mausabbruch führt zum neutralen Abbruchzustand, Retry zum erneuten Fehler. | 036, 038–039 |
| Angezeigtes Escape | **Fehler [#661](https://github.com/byte5ai/zaplex/issues/661):** Escape wirkt weder direkt noch nach explizitem Terminalfokus; Mausklick auf dieselbe Aktion funktioniert. Enter separat nativ noch nicht geprüft. | 037–040 |
| Rückweg | Abbruch per Maus und Schließen des fehlgeschlagenen Tabs kehren zum vorhandenen Editor und lokalen Terminal zurück. | 041 |

Der veraltete Pane-Kopf nach Speichern ist als
[#663](https://github.com/byte5ai/zaplex/issues/663) bestätigt: Er bleibt
„Neuer Server“, obwohl Formulartitel und Registry den Namen tragen. Der lange Hostname wird in schmalen Zeilen gekürzt; beim geöffneten
Seitenmenü liegt sein Tooltip teilweise dahinter. Fehlende Registry- und
Toolbar-Tooltips sind zusätzlich im Code bestätigt und als
[#664](https://github.com/byte5ai/zaplex/issues/664) erfasst. Damit ist C03/V02 nicht freigegeben.

Das [Manifest](evidence/candidate-127-connections/manifest.json) bindet alle 43
einzeln betrachteten Originalaufnahmen. Das Testfenster wurde über einen neuen
lokalen Terminal-Tab beendet (042–043); Workflow und Cleanup sind erfolgreich.

Nicht geprüft sind Formular-Abbruch, direktes Verbinden aus Favoriten,
weitere Fenster-/Sidebarbreiten, helles Theme und die vollständige
Icon-Tooltip-Folge. Die Korrekturen #656/#657/#658 sind nicht in dieser DMG
enthalten. Einzelne bestandene Abläufe ersetzen die übrigen Varianten nicht.
