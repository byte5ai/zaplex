# Kandidat 1.2.36: native Nachprüfung

App-Quellstand: `5a35b11e2445f20b9e47b167252a63238ed6a20d`.
Der [Batch 40125406009](https://github.com/byte5ai/zaplex/actions/runs/40125406009)
prüft die Korrekturen für den ersten Escape im Dateidialog (#684), die normale
macOS-Mindestfenstergröße (#685) und verbliebene englische Agent-Einstellungen
(#688). Der Batch ist abgeschlossen: alle Bau- und Paketgates bestanden,
die installierte GUI-Suite meldet zunächst 14 bestandene und zwei fehlgeschlagene
Fälle. Zwei gezielte Fortsetzungen ergänzen die verbleibenden Erfolge am gleichen
DMG: alle 16 Pflichtfälle sind mit geprüfter gemeinsamer Herkunft bestanden.
Direkte native Bedienung bestätigt zusätzlich Mindestgeometrie, Shellerhalt und
erste englische Einstellungen-/Escape-Varianten; die Gesamtmatrix bleibt offen.

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

## Direkte native Nachprüfung

Die ausdrücklich diagnostische [Sitzung 40132816971](https://github.com/byte5ai/zaplex/actions/runs/40132816971)
verwendet dasselbe installierte DMG mit Teststand
`0a91784ba63bbcf832be8379cda49e51e7ffa726`. Sie wurde vor dem Zeitlimit explizit
beendet; 54 Originalaufnahmen wurden einzeln betrachtet und stimmen bytegenau
mit dem gesicherten Artefakt `12576836740` überein. ZIP-SHA256:
`d9b675f45a50b345d135fe234c443332222ee69647ec9cf10946977ce00948f5`.
Das [Manifest](evidence/candidate-136-native/exploration-40132816971.json) enthält
die Einzelhashes. Der erfolgreiche Infrastrukturjob ergibt keinen GUI-Suitenpass.

| Variante / Aufgabe | Beobachtung | Originalbelege im Artefakt |
|---|---|---|
| EN/Dark, native Ecke ziehen | 1024×678 bei (0,30) schrumpft auf 480×320 und lässt sich per Maus wieder vergrößern. Frische Ausgaben bestätigen durchgehend PID 1271, nicht exportierten Marker `NATIVE136` und dasselbe Verzeichnis. | 05–08, 14–16 |
| EN/Dark, Gegenproben | Ziehen nach separater Zeigerbewegung und direktes Ziehen funktionieren. AX-Anforderung 100×100 bleibt bei 480×320; frische Shellausgabe auch dort unverändert. Fehlender Hover allein ist daher keine belegte Ursache des alten XCTest-Fehlers. | 09–13 |
| Agent-Einstellungen EN/Dark, 480×320 | Horizontales Scrollen erreicht rechte Schalter; vertikales Scrollen erreicht spätere Einstellungen. Der eigene Navigationsscrollbereich erreicht die unteren Unterseiten. | 17–23 |
| Agent-Einstellungen EN/Dark, normale Größe | Vollständige englische Sprachhilfe mit Wispr-Link, Rücksetzen sowie linke/rechte Werkzeugleiste sichtbar. Das Denkmodusmenü zeigt `Show & collapse`, `Always show`, `Never show`; `Always show` bleibt nach Schließen/Wiederöffnen ausgewählt. | 23, 25–27 |
| Agent-Einstellungen EN/Light, 480×320 | Nach tatsächlichem Themewechsel bleiben rechte und spätere Kontrollen erreichbar. Alle drei Denkmodusoptionen sind im geöffneten Menü vollständig lesbar; Navigation scrollt unabhängig. | 28–37 |
| F2 EN/Light, vollständig ausgewählter Anfangsname | Genau ein Escape schließt den Dialog; Pfeil nach unten bewegt anschließend den Listencursor. | 42–45 |
| F2 EN/Light, teilweise ausgewählter Name | Nur das letzte Zeichen ist ausgewählt; genau ein Escape schließt den Dialog. | 46–47 |
| F7 EN/Light, bearbeiteter Entwurf | `cancelled136` ist eingegeben; genau ein Escape schließt den Dialog. Pfeil nach oben bewegt anschließend den Listencursor. | 48–50 |
| Dateien und Shell nach Dialogabbrüchen | Unabhängig neu gelesene Dateinamen und SHA256 stimmen exakt mit der Ausgangsmenge überein; kein Ordner wurde angelegt. F10 kehrt zur ursprünglichen Shell zurück; frische Ausgabe bestätigt PID/Marker und das bewusst gewechselte Fixture-Verzeichnis. | 39–41, 51–53 und `fixture-baseline.json` / `fixture-after-light.json` |

Die anfängliche vorbereitende `printf`-Eingabe wurde vom Operator wegen dessen
Zeilenumbruchinterpretation falsch eingegeben (03); die gültige Shellbaseline
beginnt mit 05. Aufnahme 24 zeigt nach einem verfehlten Klick kein geöffnetes
Menü und zählt nicht als Menübeleg. Das Denkmodusmenü ist erst in 25 geöffnet.
Der AX-Verkleinerungsbefehl meldet erwartungsgemäß, dass die angeforderten
100×100 nicht erreicht wurden; entscheidend ist die beobachtete Begrenzung.

[PR #699](https://github.com/byte5ai/zaplex/pull/699) verwendet für den
automatischen Fenstertest den ausdrücklichen nativen Maus-Klick-/Ziehpfad von
XCTest. Die Geometrie- und Shell-Erwartungen bleiben unverändert. Nur dieser
verbliebene Fall läuft in [40134867825](https://github.com/byte5ai/zaplex/actions/runs/40134867825)
erneut und besteht (ein ausgeführter Fall, keine Auslassungen). Artefakt
`12575818712`, ZIP-SHA256
`31a98dd0f4a6a17b9370420300ef2998e6f59a9d7cdd1373c3b7b3bcebc7cc51`.
Die [drei visuell geprüften Originale](evidence/candidate-136-native/minimum-resume-40134867825.json)
zeigen 1024×678 vor der Geste, 480×320 am Minimum und 1024×678 nach
Wiedervergrößern. Frische Nonces 578658/176609/303497 tragen jeweils den
erhaltenen, nicht exportierten Shellwert 741. Die AX-Hierarchie bestätigt die
Geometrie des Hauptfensters; das unveränderte Hintergrundfenster ist separat.

Die [kombinierte Herkunft](evidence/candidate-136-native/gui-completion.json)
prüft Originalbatch, Teillauf 40131994068 und abschließenden Einzelfall
40134867825 gegen aktuelle GitHub-Lauf-/Jobdaten, tatsächliche Testbäume,
Zusammenfassungen, App-/Testrevisionen, identische DMG-Hashes und Artefakt-Digests.
Alle 16 Pflichtfälle sind damit bestanden, ohne vorherige Erfolge zu wiederholen.
Der ursprüngliche fehlgeschlagene Batch wird dadurch nicht umgeschrieben.

Die zusätzlichen Sprach-/Theme-Varianten folgen im nächsten Abschnitt. Weitere
Flächen bei Mindestgröße, vollständige Neustartwiederherstellung und die übrige
Gesamtmatrix sind damit nicht abgenommen.

## Sprach- und Theme-Ergänzung

[Exploration 40135145362](https://github.com/byte5ai/zaplex/actions/runs/40135145362)
prüft vor dem Start die vollständige 16-Fall-Herkunft aus den drei genannten
Läufen und verwendet dasselbe DMG. Bedienrevision:
`f053a58d92954f624f24fcbe6c7c1cf373db8620`. Die Sitzung endet explizit vor ihrem
Zeitlimit. Artefakt `12577325601`, ZIP-SHA256
`e5cc818c8541097097e251be0f919f94a48d6479712db6a056c1b61d83e8ca7b`.
Alle 64 erzeugten Original-PNGs wurden einzeln betrachtet und bytegenau mit
dem Archiv verglichen; das [Manifest](evidence/candidate-136-native/exploration-40135145362.json)
enthält die Einzelhashes. Für Sequenznummer 17 entstand kein Bild: nach dem
erfolgreichen Beenden war der erste Startaufruf syntaktisch ungültig und
führte keine Mutation aus. Der korrigierte Vordergrundstart gelingt; Bild 18
zeigt die deutsche App nach vollständigem Neustart.

| Variante / Aufgabe | Beobachtung | Belege |
|---|---|---|
| EN/Dark, F2 Anfangsauswahl und Teilmarkierung | Je genau ein Escape schließt den Dialog. Pfeil nach unten wählt danach die zweite Datei, deren Name im folgenden Dialog steht. | 04–08 |
| EN/Dark, F7 bearbeiteter Entwurf | `cancelled136` wird ohne Feldklick eingegeben; ein Escape schließt, Pfeil nach oben bewegt den Listencursor. Dateinamen und unabhängige Hashes unverändert. | 09–12, `fixture-after-en-dark.json` |
| DE/Dark, Agent-Texte | Nach Sprachwahl und vollständigem Neustart sind Sprachhilfe samt Wispr-Link, „Standard wiederherstellen“, „Linke Seite“ und „Rechte Seite“ deutsch. Denkmodusmenü: „Anzeigen und einklappen“, „Immer anzeigen“, „Nie anzeigen“. „Immer anzeigen“ bleibt nach Schließen/Wiederöffnen ausgewählt. | 14–26, [Original](evidence/candidate-136-native/23-de-dark-agent-lower.png) |
| DE/Dark, 480×320 | Horizontales Scrollen erreicht rechte Kontrollen, vertikales Scrollen spätere Abschnitte; die Navigation scrollt unabhängig bis „Über“. Alle drei Denkmodusoptionen sind vollständig lesbar. | 27–32 |
| DE/Dark, F2/F7 | Anfangsauswahl, Teilmarkierung und bearbeiteter Ordnerentwurf schließen jeweils mit einem Escape. Pfeilnavigation funktioniert anschließend; alle Dateinamen/Hashes unverändert. | 33–41, `fixture-after-de-dark.json` |
| DE/Light, Agent-Texte und 480×320 | Tatsächlicher Themewechsel; dieselben korrigierten Texte und drei Denkmodusoptionen sind deutsch. Rechte/spätere Kontrollen, linke Beschriftungen und untere Navigationspunkte bleiben über ihre Scrollbereiche erreichbar. | 43–53, [Original](evidence/candidate-136-native/47-de-light-agent.png) |
| DE/Light, F2/F7 | Alle drei Varianten schließen mit einem Escape; Pfeilnavigation kehrt in die Liste zurück. Dateinamen und Hashes stimmen erneut exakt mit der Ausgangsmenge überein. | 54–62, `fixture-after-de-light.json` |
| Deutsche Shell nach Einstellungen und Dateiaktionen | Frische Ausgaben vor und nach der gesamten deutschen Folge zeigen PID 2730, nicht exportierten Wert `DE136` und dasselbe Fixture-Verzeichnis. | 21, 63–64, [Abschlussausgabe](evidence/candidate-136-native/64-de-original-shell.png) |

Damit ist #684 für die drei geprüften Escape-Varianten in DE/EN und Hell/Dunkel
am installierten 1.2.36-DMG bestätigt (EN/Light stammt aus der vorherigen Sitzung).
Die konkret in #688 korrigierten Texte sind nativ in DE/Hell/Dunkel und
EN/Dark belegt; EN/Light ergänzt Denkmodusmenü und Erreichbarkeit am Minimum,
keine vollständige erneute Textsichtprüfung bei normaler Größe.
Die Einstellungen sind bei der tatsächlich erreichten Mindestgröße
bedienbar; daraus folgt kein Pass für jede andere Fläche oder Tab-Drag.

Die Folge findet zwei zusätzliche Fehler: #701 betrifft weiterhin englisches
`None` bei der Spracheingabetaste, `Agents` als Navigationsgruppe und `Default`
beim App-Symbol. #702 betrifft fast weiße Kontext-Chip-Vorschauen auf hellem
Grund nach Dark→Light, während benachbarte Aktionschips lesbar bleiben.
Diese Befunde stehen einer pauschalen Settings-Freigabe entgegen. Sprachaufnahme,
Netzaktionen, reale Konten, Bewegungsbelege und die übrigen Matrixvarianten
wurden hier nicht geprüft. Die Bilder belegen abgetastete Zustände.

## Ergänzung: dunkle Favoriten, Tab-Join und englische Light-Texte

[Exploration 40137786358](https://github.com/byte5ai/zaplex/actions/runs/40137786358)
verwendet dasselbe unveränderte 1.2.36-DMG, mit Bedienrevision
`3ff222a98f386dd4d660a5d264cdb807495a47e4`. Die 16 bestandenen Pflichtfälle
werden nur anhand ihrer Herkunft geprüft. Alle 42 neuen Originalbilder sind
einzeln betrachtet und bytegenau gegen Artefakt `12577909106` verglichen;
ZIP-SHA256 `e74b3cd1458eb804c53757d144918f97dd5962476f4e258c0be727022ba98614`.
Das [Manifest](evidence/candidate-136-native/exploration-40137786358.json)
erfasst Fensterrechteck, Theme, Einzelhash und tatsächliche Beobachtung.
Die Sitzung wurde ausdrücklich vor Zeitablauf beendet; der Workflow besteht.

| Prüfung | Ergebnis und Grenze | Bilder |
|---|---|---|
| #672 EN/Dark, beide Öffnungsrichtungen | Vollständiger Langnamenhinweis über dem rechten Flyout bei 1024×678 und linken Flyout bei 650×678; je zwei stationäre Aufnahmen. Ein Escape schließt rechts nur das Untermenü; links führt Bearbeiten zum bestehenden Host-Editor. | 12–18, [links](evidence/candidate-136-native/16-dark-left-tooltip.png) |
| Tab-Drag nach Mindestgrößenkorrektur #685 | Zweiter Tab an rechten Rand des ersten gezogen: zwei Panes, Quelltab entfällt, Quellpane fokussiert. Frische Ausgaben in beiden Panes bewahren PID, nicht exportierten Marker und CWD: `1272 / TAB136 / /tmp` und `2637 / SECOND136 / /Users`. | 21–29, [frische Nachweise](evidence/candidate-136-native/29-joined-left-output.png) |
| #688 EN/Light bei normaler Größe | Vollständige Sprachhilfe samt Wispr-Link, Restore default, Left side/Right side und alle drei Denkmoduswerte sind lesbar. Die Überschrift Voice Input liegt im unteren Ausschnitt teilweise oberhalb des Viewports; sie ist im vorherigen Ausschnitt sichtbar. | 34–39 |
| Zusätzlicher Fehler #704 | Nach Dark→Light sind Platzhalter und der eingegebene Wert `draft-zaplex136` in der Natural-language-denylist nahezu weiß auf weiß. Quellprüfung bestätigt einmalig gespeicherte Editorfarben. Der bereits bekannte Chip-Fehler #702 ist auch in EN/Light sichtbar. | 30, 33–38, [Eingabe](evidence/candidate-136-native/36-light-denylist-draft.png) |
| Shellrückkehr | Nach Settings/Light bleibt die ursprüngliche linke Shell mit frischer Ausgabe `1272 / TAB136 / /tmp` erhalten; die zweite Pane bleibt bestehen. | 40–41 |

Die bislang fehlende Kombination EN/Dark der Favoritenhinweise und die
konkret korrigierten englischen Light-Texte aus #688 sind damit ergänzt.
Tab-Join besteht als gezielte Regression des Mindestgrößenpatches; daraus
folgt keine Abnahme sämtlicher Drag-/Fenstertransfer-Varianten. Stills belegen
keinen kontinuierlichen Bewegungsverlauf. #701/#702/#704 benötigen für ihre
anschließenden Korrekturen einen neuen App-Kandidaten; diese alten Bilder
bestätigen ausschließlich die beschriebenen Befunde auf 1.2.36.

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
