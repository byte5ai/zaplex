# Installierte Abnahme und Exploration von 1.2.51

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562). App- und GUI-Suite-Quelle:
`421125b94239368c718d30784e7c3d17448b3506`, Version 1.2.51. Das unveränderte,
signierte und notarisierte DMG stammt aus
[Batch 41056697950](https://github.com/byte5ai/zaplex/actions/runs/41056697950).
DMG-SHA256: `20afd5f5d8900f34c5429044eb142ced8787d7059f539f2599c33b7fa5fc0195`.
Es wurde kein Release oder Tag erstellt.

## CI und unabhängige installierte Prüfung

Der vollständige Batch ist erfolgreich, ohne Übernahme alter Produktgates.
Alle **16 unterschiedlichen installierten GUI-Pflichtfälle** bestehen, ohne
Fehler, übersprungene oder erwartete Fehler. Die tatsächlichen XCTest-Fall-IDs
stimmen exakt mit dem vor Ausführung auf diese Quelle gebundenen Bestand überein.
App-/Testquelle, installierte Version und DMG-Digest passen zusammen. Die
Installation des unveränderten Pakets prüft `codesign --deep --strict`.
[GUI-Nachweis](evidence/candidate-151-native/installed-gui.json).

Die Mac-Paketsuite enthält **4.395 bestandene Fälle**, ohne Fehler oder Skips.
Der neue Inbox-Kollisionsfall ist im originalen JUnit genau einmal enthalten.
[Paketnachweis](evidence/candidate-151-native/package-tests.json).
In der frühen App-Prüfung ist derselbe Fall ebenfalls genau einmal erfolgreich
ausgeführt; die 27 positiven Ergebnisblöcke melden insgesamt 814 bestandene
Testausführungen. Diese teilweise überlappenden Auswahlen werden nicht als
disjunkte Gesamtmenge addiert.
[App- und Kürzelnachweis](evidence/candidate-151-native/inbox-shortcut-ci.json).
Linux-Daemon-Regressionen, echtes SFTP, Sicherheitsprüfung, Compile und regulärer
Binary-Clippy sind ebenfalls erfolgreich.

## Echte Inbox und getrennte Kürzel

[Exploration 41075900378](https://github.com/byte5ai/zaplex/actions/runs/41075900378)
prüft Englisch am unveränderten DMG. Der spätere Exploration-Harness
`cdda5e7b114ae978371ae1a12293a21e1f990066` unterscheidet sich von der App-/Suite-Quelle
ausschließlich durch zehn GH-564-Dokumentationsdateien; App, Ressourcen, Workflows
und GUI-Skripte sind unabhängig bytegleich verglichen. Beide Quellen bleiben im
Nachweis getrennt erfasst.

Vor Ausführung festgelegte Erwartungen: Ctrl+Cmd+O öffnet die echte Inbox statt
eines Dateidialogs. In jedem Theme und jeder Größe ist das X vor/mit Hover sichtbar;
ein tatsächlicher Klick schließt nur die Inbox. Anschließend bestätigt frische
Ausgabe dieselbe Quellshell-PID, denselben nicht exportierten Marker und dasselbe
Arbeitsverzeichnis. Nach dem Minimum wird für vollständig lesbare Ausgabe wieder
auf Normalgröße vergrößert.

| Variante | 1024×678 | 480×320 |
|---|---|---|
| EN / Zaplex Dark | „Open items“ und vollständiger Leerzustand sichtbar; X bei Ruhe/Hover, Klick und frische ursprüngliche Shell bestätigt (003–006). | Derselbe Ablauf bestätigt (007–010). |
| EN / Light | Derselbe Ablauf nach echtem Themewechsel bestätigt (014–017). | Derselbe Ablauf bestätigt (018–021). |
| DE / Zaplex Dark | Noch nicht durchgeführt. | Noch nicht durchgeführt. |
| DE / Light | Noch nicht durchgeführt. | Noch nicht durchgeführt. |

Cmd+Shift+O öffnet weiterhin den echten nativen Repository-Dateidialog. Das aktive
Dialogfenster wurde über das tatsächliche Fensterinventar bestimmt und explizit
aufgenommen (022). Escape schließt ohne Auswahl; das Hauptfenster wird wieder
aktiv, und frische Ausgabe bestätigt dieselbe ursprüngliche Shell (023).
Damit sind beide Standardkürzel in EN tatsächlich getrennt bedienbar.

23 englische Originale wurden einzeln betrachtet. Alle 30 lokal gesicherten PNGs
stimmen bytegenau mit dem abgeschlossenen Workflow-Artefakt überein. Die Sitzung
wurde nach knapp sieben Minuten eigener Bedienung ausdrücklich beendet, ohne
Ablauf des Operatorfensters.
[EN-Fallurteile, Artefaktherkunft und Einzelhashes](evidence/candidate-151-native/41075900378.json).

## Aussagegrenzen

Die native Prüfung betrifft die echte leere Inbox und den bestehenden Repository-
Einstieg. Wartende Agenten, gefüllte Kontenansichten, eigene Nutzerbelegungen und
Live-Abonnementstarts werden daraus nicht abgeleitet. Die isolierten Abo-Testprofile
fehlen weiterhin; API-Profile, unangemeldete CLIs und synthetische Fixtures ersetzen
sie nicht. Der Terminal-Worktree-Einstieg wird separat geprüft. Eine vollständige
Produktmatrix- oder Releasefreigabe folgt aus diesen gezielten Fällen nicht.
