# Kandidat 1.2.36: native Nachprüfung

App-Quellstand: `5a35b11e2445f20b9e47b167252a63238ed6a20d`.
Der [Batch 40125406009](https://github.com/byte5ai/zaplex/actions/runs/40125406009)
prüft die Korrekturen für den ersten Escape im Dateidialog (#684), die normale
macOS-Mindestfenstergröße (#685) und verbliebene englische Agent-Einstellungen
(#688). Der Batch ist abgeschlossen: alle Bau- und Paketgates bestanden,
die installierte GUI-Suite meldet zunächst 14 bestandene und zwei fehlgeschlagene
Fälle. Eine gezielte Fortsetzung ergänzt einen weiteren Erfolg am gleichen DMG.
Der Mindestgrößenfall und die vollständige interaktive Nachprüfung stehen noch aus.

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
die zusätzliche Daemon-Prüfsummendatei stimmt ebenfalls. Die installierte
GUI-Suite verwendet dieses DMG; die zwei fehlgeschlagenen Fälle werden mit
demselben Paket fortgesetzt.

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

## Installierte GUI-Suite: gezielte Fortsetzung

Der ursprüngliche GUI-Job `120977429187` meldet 14/16 bestanden, zwei Fehler
und keine übersprungenen Fälle. Sein Artefakt `12574886767` wurde gegen den
GitHub-ZIP-Digest `53b937309e0614cd647f4e197cfdbd66a7f94579177f63b93e1101a35e060727`
geprüft. Die Originalaufnahmen zeigen zwei Fehler der Texterkennung:

- `SurfaceTests/testFileManagerUsesPaneDirectoryAndReturnsToShell`: `SAMPLE`
  ist sichtbar, fehlt aber vollständig im OCR-Ergebnis.
- `SurfaceTests/testWindowMinimumSizePreservesOriginalShell`: die sichtbare
  Setup-Ausgabe `ZQA 967960 963` wird als `ZQA 967968 963` gelesen. Der Test
  erreicht die Ziehgeste nicht; dieser Lauf beweist die Mindestgröße noch nicht.

[PR #694](https://github.com/byte5ai/zaplex/pull/694) ergänzt Pixelwiederholung,
vollständige Zeilensegmentierung und feste Gegenproben. Die Linux-Diagnose
besteht, die [erste Fortsetzung 40131239409](https://github.com/byte5ai/zaplex/actions/runs/40131239409)
stoppt jedoch vor XCTest an der schmalen Fensteraufnahme. Kein GUI-Fall wird
in diesem Lauf ausgeführt. Das gesicherte Artefakt `12575350789` hat ZIP-SHA256
`2ebaf49adca4243435b22564711908c1fd074ca93d103415789df6f688970406`.

Der anschließende [native Bildvergleich 40131681579](https://github.com/byte5ai/zaplex/actions/runs/40131681579)
verwendet dieselbe Swift-Implementierung und die tatsächliche macOS-Engine
Tesseract 5.5.3. 96 Lesungen aus sechs Bildern und 16 festen Konfigurationen
reproduzieren die Ziffernverwechslung bei dreifacher Pixelwiederholung.
Zweifache Vergrößerung besteht alle positiven und negativen Erwartungen samt
Koordinaten bei beiden Interpolationsarten und beiden Segmentierungsmodi.
Artefakt `12575246799`, ZIP-SHA256
`140c20036d4c94b27fd40cdf2ccd58f7de259e946719a8241753a4ac79ac4fdf`.
Sprachdaten sind gegenüber der Linux-Diagnose identisch; deren Erfolg ersetzt
die tatsächliche native Implementierung nicht.

[PR #696](https://github.com/byte5ai/zaplex/pull/696) setzt den festen Standard
auf zweifache Pixelwiederholung und vollständige Zeilensegmentierung.
Exakte Vergleiche bleiben erhalten; kein Bild bekommt eine Sonderbehandlung.
[Fortsetzung 40131994068](https://github.com/byte5ai/zaplex/actions/runs/40131994068)
verwendet Teststand `31931bbd68994f3127a622cc266e788cca6bee1f` und besteht die
vollständige Swift-Gegenprobe einschließlich Retina-Abbildung. Der Dateimanager-
Fall besteht ebenfalls. Der Mindestgrößenfall erreicht nun die Ziehgeste, aber
das Fenster bleibt bei 1024×678; der Test bricht vor Mindestgrößen- und
Shellnachprüfung ab. Original `EC568B59-1F8D-4486-B141-505E5D4B09EE.png` wurde
visuell geprüft, die zugehörige AX-Aufnahme bestätigt die unveränderte Geometrie.
Das ist noch kein belegter Produktfehler; die Ursache wird in [#697](https://github.com/byte5ai/zaplex/issues/697)
untersucht. Artefakt `12575780668`, ZIP-SHA256
`f09a7c41f606419901fca56d6731cf081568a0611ef911957ac7d67670d75cda`.

Damit sind 15 der 16 Pflichtfälle am unveränderten DMG tatsächlich bestanden.
Nach drei fehlgeschlagenen Läufen wurde die Pipeline geprüft. [PR #698](https://github.com/byte5ai/zaplex/pull/698)
erlaubt eine streng geprüfte Folge weiterer Teilläufe, ohne Erfolge zu wiederholen;
die tatsächliche unvollständige 14+1-Evidenz bleibt abgewiesen. Ein ausdrücklich
getrennter Diagnosemodus stellt interaktiven Zugang zur Ursachenklärung bereit
und erzeugt keinen Abnahmepass. Der Originalbatch bleibt fehlgeschlagen; ein
vollständiger Nachweis erfordert den verbliebenen Fall und die geprüfte Kombination
aller tatsächlichen GUI-Ergebnisse. Der App-Build wurde nicht wiederholt.

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
