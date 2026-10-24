# Kandidat 1.2.32: CI und gezielte native Abnahme

Der vollständige [Batch 40073495704](https://github.com/byte5ai/zaplex/actions/runs/40073495704)
auf `c513867d323ad393a8cdfe951eff37a4a35ddcfc` besteht einschließlich
Signierung, Notarisierung und **15/15 installierten GUI-Pflichtfällen** ohne
Fehler oder übersprungene Fälle. Die zusätzliche
[Exploration 40077852543](https://github.com/byte5ai/zaplex/actions/runs/40077852543)
findet den initialen Dialogfokusfehler #673. Eine vollständige Produktfreigabe
ergibt sich daraus nicht; es wurde kein Release veröffentlicht.

## Herkunft und CI

App und Explorationsharness stammen vom selben Commit. DMG-SHA256:
`d950c9db3cf0d17bdbc2505ed9b8b78d77e3d707dad72f0cdf5609d3ccf412f8`
(195170421 Byte, Artefakt 12553647796). Linux-Daemon-SHA256:
`bf2896b240852d8594535c47cf07ce208c3e6a36a074cbd3ecbbf431bda684ec`
(45527703 Byte, Artefakt 12553418384). Live-Lauf, Jobs, Artefakte und
GUI-Abnahmebeleg wurden mit den vorhandenen Herkunftsprüfungen abgeglichen;
der Veröffentlichungsaufruf wurde nicht ausgeführt.

Der Batch bestätigt Linux/macOS-Kompilierung, Clippy, Abhängigkeitsrichtlinie,
16 Kryptographie-/Managed-Secret-Regressionen, fünf Maus-Routingtests,
native Agent-/Workspace-Fixtures, Live-SFTP und Phase A. Die Paketsuite meldet
4428 bestanden und 13 übersprungen; Dokumentationstests vier bestanden und
vier ignoriert. Diese Ausnahmen sind von den **null übersprungenen installierten
GUI-Pflichtfällen** zu unterscheiden.

Alle 44 erzeugten nativen Fixturebilder wurden inzwischen einzeln betrachtet;
das [Sichtprüfmanifest](evidence/candidate-132-local/fixture-review.json) ordnet
jedem Bild Hash, Sprache und Beobachtung zu. DE/EN-Leerzustände bleiben in
Hell/Dunkel bei 250/320/480 Punkten lesbar; Dateimanager zeigen ihre acht
Aktionen, Zielauswahl und Fokuszuordnung auch schmal. Die Agenten-Fixtures
zeigen erreichbare Aktionsgruppen für Freigabe, Fehler und beendete Session,
aber teilweise gekürzte Metadaten. Volle Identität per Tooltip, echte
Providerabläufe, initialer Dialogfokus und laufende Transfers sind damit
nicht nachgewiesen. Der EN/Light-Favoriten-Flyout zeigt auch in diesem
Fixture den verdeckten Identitätshinweis #672 auf 1.2.32; die spätere
Korrektur wird daraus nicht als bestanden bewertet. Pane-Bilder zeigen
erhaltene Entwürfe und Endgeometrien, keinen kontinuierlichen Drag.

## Installierte Exploration

Englisch, Zaplex Dark/Light, Fenster 1024×678 bei (0,30), 24.10.2026.
Erwartungen für Entwurf, Auswahl, erste Eingabe ohne Feldklick und belegte
Shell wurden vor den Aktionen festgehalten. Alle 47 Originalbilder wurden
einzeln betrachtet und mit Artefakt 12554771586 bytegenau abgeglichen; das
[Manifest](evidence/candidate-132-local/manifest.json) enthält ihre SHA256.
Acht Schlüsselbilder liegen neben dem Manifest; der vollständige Satz bleibt
im Workflow-Artefakt und im zusätzlich gesicherten Originalarchiv erhalten.
Workflow und eigene Prozessbereinigung sind erfolgreich; Ende 08:54:47 UTC.
Der verfügbare Operator unterstützt keine Videoaufnahme. Die Standbilder
und tatsächlichen Eingabeergebnisse belegen die unten genannten Endzustände,
keine kontinuierlichen Übergänge.

| Fall | Beobachtung und Bewertung | Originale |
|---|---|---|
| Filter, Dark→Light | Auswahl und Entwurf bleiben über Settings erhalten; rohe Taste `z` ersetzt ohne Feldklick. EN-Teilnachweis #656 bestanden. | 007–015 |
| Rename, Light→Dark | Nach explizitem anfänglichem Feldklick bleiben Entwurf und Auswahl erhalten; `r` ersetzt ohne erneuten Klick. EN-Teilnachweis #656 bestanden. | 019–025 |
| Neuer Ordner, Dark→Light | Nach explizitem anfänglichem Feldklick bleiben Entwurf und Auswahl erhalten; `n` ersetzt ohne erneuten Klick. EN-Teilnachweis #656 bestanden. | 028–033 |
| Initialer Dialogfokus | F2 lässt Eingabe im Filter; F7 nimmt den Entwurf nicht auf. Escape/F7/rohe Taste `h` bestätigt den Fehler unabhängig vom `type`-Operator. Fehler #673. | 017–018, 026–027, 034 |
| Abbruch ohne Dateimutation | Alle Dialoge abgebrochen. Frischer Befehl bestätigt gleiche Shell, Marker, CWD, Dateien und Hashes. | 035–037 |
| F11, lokal belegte Shell | Shell wartet in `read`; Dateimanager navigiert nach `child`. F10 meldet aufgeschobenen Wechsel. `read` erhält exakt die explizite manuelle Eingabe, erst danach läuft `cd`. | 038–044 |
| F11, Identität/Inhalt | Frische Ausgabe bestätigt Ziel-CWD, identische PID1488, nicht exportierten Marker und unveränderte Ausgangsdateien. | 045–046 |

Der Marker lautet `FOCUS_40077852543`. Die Dateien enthalten `alpha` und `beta`;
SHA256 `8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8`
und `f44e64e75f3948e9f73f8dfa94721c4ce8cbb4f265c4790c702b2d41cfbf2753`.
Der wartende Befehl empfängt genau `MANUAL_RELEASE_40077852543`, keinen
eingespeisten Verzeichniswechsel. Das ist ein lokaler F11-Teilnachweis;
verschachtelte und Remote-Shells bleiben davon unberührt.

Die Header-Ordnerklicks in 004–005 öffnen die Ansicht nicht; wegen der
Operator-/Fokusmehrdeutigkeit wird kein neuer Produktfehler daraus abgeleitet.
CmdShiftE öffnet sie zuverlässig. `type` meldet häufig einen Fehler trotz
ausgelieferter Eingabe; vor Bestätigung oder Wiederholung wurde jeweils das
tatsächliche Bild geprüft. Die entscheidenden Ersatzzeichen sind rohe
Tastendrücke. Kein Rename-/Neuanlage-Dialog wurde bestätigt.

Remote #657/#658 wurden in dieser Exploration nicht ausgeführt. Frühere
Nachweise behalten ihre jeweilige Versionsbindung. Echte Abonnements und
Kontingentrouting bleiben ungeprüft.

## Deutsche Fortsetzung und Verbindungen

Die [zweite Exploration 40080220445](https://github.com/byte5ai/zaplex/actions/runs/40080220445)
nutzt dasselbe unveränderte DMG. Nach Sprachwechsel und vollständigem Neustart
läuft die App auf Deutsch bei 1024×678. Alle 66 Aufnahmen wurden einzeln
betrachtet und mit Artefakt 12555353703 abgeglichen; das
[Manifest](evidence/candidate-132-de-connections/manifest.json) bindet die Hashes.
Vierzehn Schlüsselbilder liegen daneben; alle Originale sind zusätzlich
archiviert. Der eigene Abschluss und die Workflow-Bereinigung sind erfolgreich.

| Fall | Beobachtung und Bewertung | Originale |
|---|---|---|
| #656, Deutsch | Filter Dark→Light, explizit fokussiertes Rename Light→Dark und explizit fokussierter neuer Ordner Dark→Light erhalten Entwurf/Auswahl; rohe Ersatzzeichen z/r/n landen ohne erneuten Feldklick im richtigen Feld. Drei Teilfälle bestanden. | 011–032 |
| Shell/Dateien | Nach Dialogabbruch gleiche PID1918, nicht exportierter Marker DE_40080220445, CWD und unveränderte alpha/beta-Hashes. | 034 |
| #663, DE/Light | Neuer Server und gespeicherte Umbenennung aktualisieren denselben Pane-Header unmittelbar; ungespeicherte Entwürfe lassen den alten Titel stehen. Speichern erzeugt keine weitere Pane. | 041–048 |
| #664, DE/Light | Deutsche Werkzeughinweise für Ordner, Server und Ausklappen; vollständiger langer Registryname lesbar; Favoriten-Stern samt Zustandshinweis bedienbar. Teilnachweis. | 037–039, 045, 048 |
| #672, DE/Light | Favoriten-Aktionsmenü verdeckt weiterhin den vollständigen Identitätshinweis. Escape schließt das Untermenü, worauf der Hinweis vollständig sichtbar wird. Fehler bestätigt. | 050–052 |
| Primärer Favoritenzugang | Erneut geöffnetes Menü: Klick auf den Namen startet eine separate Verbindung zur synthetischen lokalen Testadresse. | 054–055 |
| #661, DE/Light | Echter Connection-refused-Zustand; danach ein unabhängig als bereit bestätigter, eigener TCP-Listener. Enter erreicht diesen Listener und erzeugt einen unterscheidbaren neuen SSH-Fehler. Escape zeigt ausdrücklich den Abbruchzustand. | 055, 060–063 |
| Unabhängiger Retrybeleg | Logmarker RETRY_ACCEPT_40080220445 bestätigt die neue Verbindung. Der begrenzte Listener ist beendet; Originalshell1918 und Marker/CWD bleiben erhalten. | 065 |

Der Listener wartet maximal 90 Sekunden, nutzt ein eigenes Log und umgeleitete
Standardeingabe. Der Retrybeleg ist kein erfolgreicher SSH-Handshake und keine
Abnahme aller Recoveryzustände. 053 ist ein Operator-Leerlauf: Nach Escape war
das Elternmenü noch offen und wurde versehentlich vor dem Zielklick geschlossen.
Daraus wird kein Produktfehler abgeleitet.

Einklappen mit vorhandenem Ordner, Inline-Rename und die EN-Verbindungsvarianten
sind hier nicht geprüft. In Aufnahme 012 enthält die deutsche Agent-Einstellungsseite
eine englische Supportzeile; der bestätigte Befund ist #674. Remote #657/#658
und weitere Matrixkombinationen bleiben separat auszuführen.

Die anschließende [Remote-Nachprüfung](CANDIDATE-1.2.32-REMOTE.md) bestätigt
native SSH-Steuerbefehle (#657), dieselbe persistente Shell nach Transportabbruch
und die einzelne Transferanzeige (#658) samt Pause/Abbruch für EN/dunkel.

## Ergänzte Mausauswahl

Die kurze [Exploration 40087983824](https://github.com/byte5ai/zaplex/actions/runs/40087983824)
nutzt dasselbe unveränderte 1.2.32-DMG mit Harness
`3fe2d03d7fe503f2ad1b7c97e512703f22374550`, EN/Zaplex Dark, 1024×678 bei (0,30).
Alle 15 Originalaufnahmen wurden einzeln betrachtet und mit Artefakt 12557259505
bytegenau abgeglichen; das [Manifest](evidence/candidate-132-selection/manifest.json)
bindet die Hashes und Operatorgrenzen. Eigener Abschluss und Workflow-Bereinigung
sind erfolgreich. Die Prüfung dauerte im interaktiven Zeitfenster rund sechs Minuten.

| Fall | Beobachtung | Originale |
|---|---|---|
| Cmd-Klick | Markiert den tatsächlich getroffenen Ordner `child`; fetter Akzenttext, Tönung und „1 folder marked · folder size not computed“ sichtbar. | 008 |
| Einfacher Klick | Verschiebt den Cursor zu `beta.txt`, erhält die Ordner-Markierung; Cursor und Auswahl sind unterscheidbar. | 009 |
| Shift-Klick | Ergänzt von `beta.txt` bis `gamma.txt` beide Dateien; Ordner bleibt markiert. Zählung: ein Ordner, zwei Dateien, 9 B ohne Ordnergrößen. | 010–011 |
| Escape | Hebt alle Markierungen auf, Cursor bleibt auf `gamma.txt`. | 012 |
| Unveränderter Zustand | F10 zurück zur Shell; frische Ausgabe bestätigt PID2042, nicht exportierten Marker `SEL_40087983824`, dasselbe CWD und dieselben drei Dateihashes. | 003, 014 |

Die Dateien enthalten unabhängig vorgegebene Bytes `alpha`, `beta`, `gamma`.
Ihre SHA256 lauten `8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8`,
`f44e64e75f3948e9f73f8dfa94721c4ce8cbb4f265c4790c702b2d41cfbf2753`
und `be9d587defa1f0c09ef49eb17e206983a5f8f8289e4281860bd0ee5a19592c67`.
Es wurde keine Dateiaktion bestätigt.

005 ist ein vor Ausführung abgewiesener Operatoraufruf. Bei 008 traf der
korrigierte Cmd-Klick wegen globaler statt fensterbezogener Koordinaten den
Ordner `child`, nicht die zunächst beabsichtigte Datei. Die Bildnamen 006/009
nehmen irrtümlich eine Alpha-Markierung vorweg; bewertet wird der tatsächliche
Bildinhalt. Der anschließende Shift-Klick nutzt explizite globale Koordinaten.
Die Maus-Rücksetzung wurde beim ersten ausgeführten Modifikatorklick nicht
bestätigt; vor dem nächsten Schritt wurde der Desktop erneut beobachtet.
Cmd+A/Elternzeile, Entmarkieren per Cmd-Klick, Shift-Pfeiltasten und Einfg wurden
hier nicht erneut bzw. nicht ausgeführt. Weitere Varianten bleiben offen.
