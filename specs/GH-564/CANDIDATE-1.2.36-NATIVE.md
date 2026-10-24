# Kandidat 1.2.36: native Nachprüfung

App-Quellstand: `5a35b11e2445f20b9e47b167252a63238ed6a20d`.
Der [Batch 40125406009](https://github.com/byte5ai/zaplex/actions/runs/40125406009)
prüft die Korrekturen für den ersten Escape im Dateidialog (#684), die normale
macOS-Mindestfenstergröße (#685) und verbliebene englische Agent-Einstellungen
(#688). Der vollständige Batch und die installierte Nachprüfung sind zum
Zeitpunkt dieses Zwischenstands noch nicht abgeschlossen.

## Bisherige CI-Nachweise

Linux-/macOS-Kompilierung, Abhängigkeitsprüfung, Linux-Paketsuite, Live-SFTP,
Phase A und Signaturprüfung bestanden. Die Paketsuite meldet 4433 bestandene
und 13 übersprungene Tests; Doc-Tests vier bestanden und vier ignoriert.
Diese Ausnahmen sind kein Nachweis für eine vollständige installierte GUI-Suite.
Live-SFTP verwendet einen echten OpenSSH-Server. Die Recoveryfälle unterbrechen
gezielt den isolierten Safe-File-Testkanal vor beziehungsweise nach der Mutation;
sie sind kein neuer GUI-Nachweis eines vollständigen SSH-Netzausfalls.

App-/Daemon-Bau, Signierung und Notarisierung sind abgeschlossen. Das DMG
(195267167 Byte, Artefakt `12574841729`) hat SHA256
`98c623e3b58488ac586f590fa6350a790125c08ce45e6b1f23485db961c55039`.
Der Linux-Daemon (45541420 Byte, Artefakt `12574066919`) hat SHA256
`a3222a00dce2747870580f7525e1709d817ebf5c4356ac6aef11dee325b7d503`.
Beide Downloads stimmen mit den ZIP-Digests der GitHub-Artefakte überein;
die zusätzliche Daemon-Prüfsummendatei stimmt ebenfalls. Die Zuordnung zur
installierten GUI-Suite wird nach deren Abschluss geprüft.

Der macOS-Job `120966248327` besteht sechs Plattformtests, darunter
`undersized_window_frames_grow_without_moving_the_top_left_corner`, einen
Agentlayouttest, sechs Workspace-Fälle und den lokalen Header-Einstieg.
Der Geometrietest allein beweist keine native Ziehgeste oder Bedienbarkeit
der Einstellungen bei Mindestgröße.

Bei der Vorbereitung fiel außerdem eine veraltete positive Testfixture der
Veröffentlichungssperre auf: 15 bestandene Fälle trotz nun 16 Pflichtfällen.
[PR #692](https://github.com/byte5ai/zaplex/pull/692) korrigiert Fixture und
vorbereiteten Beschreibungstext; 21 Prüfungen bestehen danach, ein Nachweis
mit nur 15 Fällen bleibt abgewiesen. App und GUI-Testfälle sind unverändert.
Es wurde kein Release veröffentlicht.

## Native Render-Fixtures

Alle 44 erzeugten Original-PNGs wurden einzeln betrachtet: 41 Workspace- und
drei Agentbilder. Das [Manifest](evidence/candidate-136-native/manifest.json)
bindet jede Beobachtung an Quellstand, Artefaktpfad und SHA256. Die 41
Workspace-Hashes stimmen mit dem vom Job erzeugten Herkunftsnachweis überein;
beide ZIP-Hashes wurden gegen die GitHub-Artefaktmetadaten geprüft.

| Zustand | Beobachtung und Aussagegrenze |
|---|---|
| Verbindungen, EN, Hell/Dunkel, Sidebar 250/320/480 | Aktionen bleiben getrennt, lange Titel werden gekürzt; der sichtbare Tooltip zeigt die vollständige Hostidentität. |
| Favoriten-Flyout, EN/Light, 900×640 | Vollständiger Langnamenhinweis liegt über dem linken Untermenü und innerhalb des Bildes. Keine Aussage über stationäres Hover-Verhalten oder Mausdurchlässigkeit aus einem Einzelbild. |
| Leere Verbindungen/Favoriten, DE/EN, Hell/Dunkel | Hinweise umbrechen innerhalb ihrer Flächen; Connection-Sidebars bei 250/320/480 geprüft. |
| Dateimanager, DE/EN, Hell/Dunkel, normal/schmal | Acht beschriftete Aktionen, aktive/inaktive Zustände, Zielzeile, voller Dateinamen-Tooltip und fokussierter Ordnerdialog bleiben lesbar. |
| Dateimanager-Rückkehr und Pane-Drag | Terminals zeigen erhaltene Eingabeentwürfe und getrennte Sessions. Die Bilder belegen Endzustände, keinen kontinuierlichen Drag. |
| Agent, normal/schmal, Dark | Freigabe, beendete Session und Fehleransicht zeigen erreichbare Aktionsgruppen ohne Überlappung. Lange Metadaten sind gekürzt; vollständige Tooltip-Bedienung wurde hier nicht geprüft. |

Keine neue sichtbare Regression in diesen dargestellten Zuständen. Es handelt
sich um deterministische Render-/Interaktionsfixtures mit isolierten lokalen
PTYs und Dateien. Installierte DMG, echte SSH-/Abo-Abläufe, Reduced Motion,
Neustartwiederherstellung und Cross-Host-Transfers sind damit nicht abgenommen.
Die Gesamtmatrix bleibt offen.
