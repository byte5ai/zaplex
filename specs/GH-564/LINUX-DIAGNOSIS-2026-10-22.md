# Lokale Diagnose der Terminal-Einführung am 22.10.2026

Dies ist ein Linux-Teilnachweis für #601/#564, keine macOS- oder DMG-Abnahme.
Quellstand `79efc54d7`, Version 1.2.15: Beide benannten Integrationstests
(DE/EN) bestehen mit echter Bash-Shell und Xvfb/Vulkan-Software-Rendering.
Die Prüfung erfolgt in isolierten Profilen mit temporären Laufzeitbibliotheken.

Der Test aktiviert nach der SSH-Formularprüfung die Terminal-Pane mit einem
Klick und prüft deren Fokus. Erst danach klickt er „Nicht mehr anzeigen“.
Die bestehende Assertion verlangt sowohl das Verschwinden der Einführung als
auch die gespeicherte Ausblendeinstellung. Alle Textgrenzen werden weiterhin
gegen die tatsächliche Pane geprüft. Die erfolgreichen Läufe enthalten keine
zusätzlichen Diagnose-Klicks. Der vorherige Ablauf hatte mit seinem einzigen
Klick lediglich die benachbarte Terminal-Pane aktiviert.

Alle zehn unveränderten Originalaufnahmen wurden visuell geprüft:
Titel und Hinweise sind vollständig, lange deutsche Texte umbrechen in der
schmalen Pane, die Ausblendaktion bleibt sichtbar. Nach dem Klick fehlt die
Einführung in beiden Sprachen. Die übrige Terminalausgabe gehört zur lokalen
Testumgebung und ist kein Sollvergleich für das macOS-Produkt.

| Sprache | Dunkel 1024 | Dunkel 760 | Hell 1024 | Hell 760 | Ausgeblendet |
|---|---|---|---|---|---|
| EN | [Bild](evidence/linux-introduction-2026-10-22/en/terminal-introduction-dark-1024.png) | [Bild](evidence/linux-introduction-2026-10-22/en/terminal-introduction-dark-760.png) | [Bild](evidence/linux-introduction-2026-10-22/en/terminal-introduction-light-1024.png) | [Bild](evidence/linux-introduction-2026-10-22/en/terminal-introduction-light-760.png) | [Bild](evidence/linux-introduction-2026-10-22/en/terminal-introduction-dismissed.png) |
| DE | [Bild](evidence/linux-introduction-2026-10-22/de/terminal-introduction-dark-1024.png) | [Bild](evidence/linux-introduction-2026-10-22/de/terminal-introduction-dark-760.png) | [Bild](evidence/linux-introduction-2026-10-22/de/terminal-introduction-light-1024.png) | [Bild](evidence/linux-introduction-2026-10-22/de/terminal-introduction-light-760.png) | [Bild](evidence/linux-introduction-2026-10-22/de/terminal-introduction-dismissed.png) |

Die [SHA256-Zuordnung](evidence/linux-introduction-2026-10-22/manifest.json)
und die Aktionsprotokolle je Sprache sichern die Zuordnung zu den beiden Fällen.
Native macOS-Nachprüfung und installiertes aktuelles DMG bleiben gesonderte Nachweise.

Der separate lokale Header-Klicktest auf dem Quellbaum von 1.2.14 besteht;
sein begrenzter Aussageumfang ist in [#604](https://github.com/byte5ai/zaplex/issues/604#issuecomment-6342565109)
dokumentiert. Er ersetzt den beobachteten Befund am alten macOS-DMG nicht.
