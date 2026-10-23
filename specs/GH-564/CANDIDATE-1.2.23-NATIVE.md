# Native Sichtprüfung des Kandidaten 1.2.23

Der [Batch 39871910859](https://github.com/byte5ai/zaplex/actions/runs/39871910859)
prüft den unveränderten App-Commit `bf52cf4b75033f48c4d3d945da26bdcaa803f323`.
Die nativen Integrationsfälle dieses Commits lieferten am 22.10.2026 um
23:51 UTC **42 Originalaufnahmen**, die einzeln visuell geprüft wurden.
Das ist ein aktueller nativer Integrationsbuild, noch keine installierte DMG-Abnahme.

Die Erwartungen stammen aus der bestehenden [Prüfmatrix](MATRIX.md) und den dort
zugeordneten Designreferenzen; die neuen Bilder werden nicht zur Sollreferenz.
Das [Manifest](evidence/candidate-123-native/manifest.json) bindet jede Aufnahme
an Quelle, Größe und SHA-256. Die Originale sind lokal gesichert und in den
Actions-Artefakten `subscription-agent-layout-evidence` und
`native-workspace-evidence-bf52cf4b75033f48c4d3d945da26bdcaa803f323` zugeordnet.
Die Rohbilder enthalten teilweise temporäre Laufzeitpfade und werden hier nicht
erneut veröffentlicht.

| Prüfgegenstand | Aufnahmen / Varianten | Beobachtung |
|---|---|---|
| Agentenansicht | 3; 1180×640 und 540×670, Englisch/dunkel | Freigabeaktionen getrennt; beendete Session bietet neue Unterhaltung und Shell-Rückweg; wiederherstellbarer Fehler zeigt Resume, Neustart, Ende und Änderungsaktionen. Lange Identitäten werden gekürzt. Keine neue Überlagerung der geprüften Aktionen. |
| Gefüllte Verbindungsliste und Favoriten-Flyout | 7; Seitenleiste 250/320/480, Hell/Dunkel; Flyout am rechten Rand | Host-/Sessionhierarchie und feste Aktionsplätze bleiben erkennbar. Metadaten umbrechen, lange Namen kürzen. Das Flyout öffnet nach links und zeigt die drei Folgeaktionen. |
| Leere Verbindungen/Favoriten | 16; DE/EN, Hell/Dunkel, Seitenleiste 250/320/480 | Die vollständige Anleitung nennt Ordner und Plus auch bei 250 Punkten. Der Favoriten-Leerzustand erklärt die Sternaktion, ohne die folgenden Menüeinträge zu überlagern. |
| Zwei Dateimanager | 14; DE/EN, Hell/Dunkel, 1180×640 und 760×640 | Alle acht beschrifteten Dateiaktionen in zwei bzw. vier Reihen sichtbar; F3 heißt Details. Fokus und inaktive Nachbaraktionen unterscheiden sich. Voller Dateiname im Tooltip; Ordnerdialog bleibt in der fokussierten Pane. Nach Rückkehr sind beide Terminals und der Entwurf sichtbar. |
| Pane-Anordnung | 2; Englisch/dunkel, 1180×640 | Unterschiedliche Pane-Identitäten trotz gleichem CWD; nach Drag vertikale Anordnung, erhaltener Entwurf und expliziter Tabtitel. |

Für diese sichtbaren Zustände wurde keine neue blockierende Abweichung gefunden.
Die gekürzten Transferzielpfade sind allein noch kein Nachweis einer verständlichen
Zielauswahl; das tatsächliche Kopieren/Verschieben mit kontrolliertem Ziel bleibt
Teil der anschließenden Exploration. Auch Tooltips langer Konten, alle Varianten
von Drag/Restore, OS Reduced Motion sowie echte Remote-/Abo-Zustände werden aus
diesen Aufnahmen nicht als bestanden abgeleitet.

Die funktionalen Assertions stammen aus dem nativen Testlauf; die hier dokumentierte
Sichtprüfung ist davon getrennt. Konstruierte Host-/Sessiondaten und Agentenzustände
belegen ihre native Darstellung, keine echte SSH-Verbindung oder Provider-Anmeldung.
