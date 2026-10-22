# Terminal-Einführung und Ordner-Klick nativ bestätigt

[Run 39798639024](https://github.com/byte5ai/zaplex/actions/runs/39798639024)
bestand am 22.10.2026 vollständig: Build und separater Layout-Job. App und
Testrevision: `75227a198be7279bd13100f2a545d22b0026d3f2`, **1.2.15**.
Drei ausgewählte native Integrationstests, keine ausgelassenen Fälle.
Dies ist ein Testprogramm mit realem macOS-Rendering, **kein installiertes DMG**.
Designreferenz: `6c956383ad560913dc31fa383104dc489104d561` und MATRIX.md.

[Manifest mit Prüfsummen](evidence/native-terminal-39798639024/manifest.json).
Alle zwölf Original-PNGs wurden einzeln angesehen und unverändert archiviert;
die drei zugehörigen `recording.log` protokollieren Schritte und Assertions.

| Fall | Vorher-/Nachher-Belege | Ergebnis und Grenze |
|---|---|---|
| Einführung EN | [Dark 1024](evidence/native-terminal-39798639024/en/terminal-introduction-dark-1024.png), [Dark 760](evidence/native-terminal-39798639024/en/terminal-introduction-dark-760.png), [Light 1024](evidence/native-terminal-39798639024/en/terminal-introduction-light-1024.png), [Light 760](evidence/native-terminal-39798639024/en/terminal-introduction-light-760.png), [ausgeblendet](evidence/native-terminal-39798639024/en/terminal-introduction-dismissed.png) | Titel, vier Hinweise und Ausblendaktion vollständig innerhalb der schmalen Terminal-Pane. Lange Texte umbrechen; keine Überdeckung der Aktion. Nach explizitem Terminal-Fokus entfernt ein Klick die Einführung dauerhaft im Testprofil. |
| Einführung DE | [Dark 1024](evidence/native-terminal-39798639024/de/terminal-introduction-dark-1024.png), [Dark 760](evidence/native-terminal-39798639024/de/terminal-introduction-dark-760.png), [Light 1024](evidence/native-terminal-39798639024/de/terminal-introduction-light-1024.png), [Light 760](evidence/native-terminal-39798639024/de/terminal-introduction-light-760.png), [ausgeblendet](evidence/native-terminal-39798639024/de/terminal-introduction-dismissed.png) | Deutsche Texte sichtbar; bei 760 Punkten umbrechen auch Titel und längere Hinweise. Gleicher Fokus-/Ausblendnachweis. |
| Echter Ordner-Klick | [Terminal vorher](evidence/native-terminal-39798639024/header/file-manager-header-before.png), [Dateimanager danach](evidence/native-terminal-39798639024/header/file-manager-header-after.png) | Der gerenderte Ordner-Button wird geklickt. Genau die auslösende Pane wechselt zum Dateimanager im Fixture-CWD und zeigt `ORIGINAL.txt` mit 6 Byte. Die Prüfung löst die Workspace-Aktion nicht direkt aus. Geprüft ist eine fokussierte lokale Pane, EN/Dark, 1024×640. |

Die Einführungsfälle verwenden Fenster mit 1024×640 und 760×640 Punkten,
nebeneinander ein lang benanntes synthetisches SSH-Formular und die lokale
Terminal-Pane. Die vollständigen Einführungslabels werden zusätzlich gegen die
realen Layoutbounds geprüft. Das Formular ist Kontext dieser Prüfung, kein
erneuter vollständiger Formularnachweis. Kontrast, Reduced Motion und weitere
Pane-Konstellationen sind hier nicht geprüft. Vor dem Ausblendklick wird die
Terminal-Pane ausdrücklich fokussiert; ein Klick aus einer inaktiven Pane ist
keine Aussage dieses Falls.

Damit ist die Textkorrektur aus PR #603 zu #601 nach dem Fokus-Test aus PR #612
auch nativ bestätigt. Der aktuelle native Header-Test reproduziert den Fehler
#604 nicht. Er erklärt jedoch noch nicht den wirkungslosen externen Klick am
älteren installierten DMG 1.2.2; dieses Issue bleibt für die gezielte Gegenprobe
offen. Die Fixtures liefern keine vollständige UX-/DMG-Abnahme für #564/#562.

## Build und Wiederverwendung

Der Buildschritt dauerte 13 Minuten 32 Sekunden, Cargo selbst 12 Minuten
58 Sekunden. Vier Cargo-Jobs, keine Debug-Informationen, kein nachträgliches
Stripping. Die ausführbaren Testprogramme wurden vor dem Layout-Job separat
archiviert und in dessen frischer VM wiederhergestellt; es gab keinen zweiten
App-Build. Compilerlog und 27 Ressourcen-Stichproben sind ebenfalls gesichert.

Der erfolgreiche Lauf beweist keine Speicherursache für frühere Abbrüche.
Buildprofil, Cache und Infrastruktur haben sich zwischen den Versuchen verändert.
Insbesondere wird die frühere RAM-Angabe nicht ungeprüft als aktuelle Messbedingung
übernommen. Es wurde kein Release erzeugt und kein Linux-Daemon erneut gebaut.
