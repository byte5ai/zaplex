# Native Tabtitel und verschachtelte Pane-Gruppen

Die [Exploration 40065487791](https://github.com/byte5ai/zaplex/actions/runs/40065487791)
verwendet die unveränderte DMG aus [Batch 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378).
App-Quelle: `b270ee372f8548b83363916dc8fbddc1ee7b5884`; Harness:
`bcf84322f3ad764fd6d2c9c2d4ceda451015ae50`.
DMG-SHA256: `fcb082266c6b4f61fb8bb0c3d871e9ccafb8c229225e528f288bafb36f800644`.
Englisch, dunkles Theme, 1024×678 Pixel, Sidebar etwa 250 Punkte.
Erwartungen für P03/P04/P05 und J1–J8 wurden vor den Aktionen festgehalten.

| Ablauf | Ergebnis | Aufnahmen |
|---|---|---|
| Expliziter Tabtitel | `Acceptance Left` bleibt bei CWD-Wechsel erhalten; Löschen des Namens stellt den automatischen Titel `Local · /` wieder her. | 006–010 |
| Einen Tab in anderen einfügen | Drag an den rechten Rand erzeugt zwei Panes im Zieltab, entfernt den Quelltab und fokussiert die Quelle. Frische Befehle bestätigen beide Shell-Identitäten. | 011–017 |
| Pane-Kopfzeile verschieben | Vier Automationsdrags und ein Headerklick bleiben wirkungslos. **Befund korrigiert:** Es fehlte das ausdrückliche Heranführen des Mauszeigers vor Klick/Drag; die [native Gegenprobe](CANDIDATE-1.2.27-HEADERS.md) besteht mit unveränderter App. Diese Sequenzen belegen keinen Produktfehler. | 018–022 |
| Verschachtelten Tab einfügen | Zwei-Pane-Tab an linken Rand eines dritten Tabs gezogen: Untergruppe, interne Reihenfolge und Quellfokus bleiben erhalten; Quelltab entfällt. Drei frische Ausgaben bestätigen alle Shells. | 024–032 |
| Drop auf eigenen Tab | Einziger Tab auf eigene Pane gezogen: Layout bleibt unverändert. | 033 |

Die Shell-Marker sind nicht exportierte Variablen. Vorher/nachher stimmen
PID, Marker und CWD überein: `1298 / LEFT_40065487791 / /`,
`2578 / RIGHT_40065487791 / /Users`, `3814 / THIRD_40065487791 / /var`.
Damit ist eine bloße Wiederherstellung des sichtbaren Scrollbacks ausgeschlossen.

Alle 34 Aufnahmen wurden einzeln betrachtet und im
[Manifest](evidence/candidate-127-panes/manifest.json) mit Hash gebunden.
002 ist ein fehlerhafter Operatorbefehl mit interpretiertem Zeilenumbruch;
005 ein zunächst falscher Menüpfad, 023 eine nicht angekommene Eingabe.
Diese Versuche zählen weder als Produktfehler noch als bestandene Prüfung.
Der Abschlussbefehl wurde vor Ausführung visuell geprüft (034);
Workflow einschließlich Cleanup ist erfolgreich.

Nicht nachgewiesen: kontinuierliche Sichtbarkeit des Ziels während des Drags,
Escape-Abbruch, ungültige Ziele, sämtliche vier Kanten, Remote-Varianten und
Reconnect. P03/P05 haben Teilnachweise; für P04 gilt die spätere
[native Gegenprobe](CANDIDATE-1.2.27-HEADERS.md).
