# Native Fixture- und DMG-Nachweise

**Keine UX-Gesamtfreigabe.** Connections, Favoriten und Dateipanes sind nach
PR #586 auf `1800ec77` nativ erfolgreich geprüft und visuell ausgewertet.
[Der aktuelle Teilnachweis](#workspace-korrekturen-nativ-bestätigt-am-20102026)
ersetzt für diese konkreten Ansichten die unten gesicherten historischen Fehler.
Der Agentenfall ist separat fehlgeschlagen. Fixtures, installiertes DMG und freie
Exploration bleiben getrennte Nachweise.

## Historische Herkunft und Aussagegrenzen

- [Batch 39246675914](https://github.com/byte5ai/zaplex/actions/runs/39246675914),
  Quell- und Teststand `1201fc4f3cafcbab7464343cad8b715d8cf3db2e`, Paketversion 1.2.1.
- Native macOS-App im Integrationstest mit echtem Display. Kein installiertes DMG;
  deshalb kein DMG-Digest. Die Aufnahmen sind keine Browser-Mockups.
- 15 unveränderte PNGs mit Herkunft, Abmessungen und SHA-256 im
  [Manifest](evidence/native-1201fc4f/manifest.json). Die 13 Workspace-Aufnahmen
  wurden gegen das vom CI-Job erzeugte `evidence.json` geprüft.
- Englisch; Connections explizit hell/dunkel und 250/320/480 Punkte Sidebarbreite.
  Übrige Aufnahmen zeigen die dunkle Darstellung, das Favoriten-Flyout die helle.
  Die im Dateinamen genannten Fenstermaße werden in den nativen Tests gesetzt
  und geprüft; keine Gleichsetzung mit einer AX-Messung des installierten DMGs.
- Host-/Sessioninventar und Agentenzustände sind gesetzte, bezeichnete Fixtures.
  Lokale Dateimanager und Panes verwenden echte isolierte Dateien und PTYs.
  Kein echter SSH-/Provider-Erfolg, keine unabhängige Exploration, kein Nachweis
  für Deutsch, Kontrastvariante oder Reduced Motion.

## Priorisierte Abweichungen

### NF01 — P1: Dateinamen verschwinden in zwei schmalen Dateipanes

**Korrektur:** [#572](https://github.com/byte5ai/zaplex/issues/572).
Bezug: Matrix F03/F08–F10 und Variante „Native Mindestbreite“ / lesbare Identität.

Soll: Dateiidentität bleibt vor optionalen Metadaten erkennbar. Ist: Bei
760×640 Punkten verschwinden „Name“, `projects` und `acceptance-file.txt`
vollständig; Größe und Änderungsdatum bleiben. Der aktive Rahmen und alle acht
F-Aktionen sind sichtbar, aber der Nutzer kann das Ziel nicht am Namen erkennen.
Bei 1180×640 sind dieselben Namen lesbar.

| Normal | Schmal |
|---|---|
| [1180×640](evidence/native-1201fc4f/native-file-managers-normal.png) | [760×640](evidence/native-1201fc4f/native-file-managers-narrow.png) |

Reproduktion: `test_native_workspace_file_managers_evidence`, zwei lokale
Dateipanes mit offener Sidebar, dann Resize von 1180 auf 760 Punkte. Auch der
[anschließende F7-Dialog](evidence/native-1201fc4f/native-file-manager-focused-action.png)
zeigt die verschwundenen Namen.

Quellbefund: `app/src/sftp_manager/file_list.rs` reserviert 80 Punkte für Größe,
120 für Datum sowie Icon/Abstände; der Name bekommt nur die verbleibende Breite.
Header und Dateizeilen haben denselben unbedingten Aufbau.

Testauftrag in #572: responsive Priorität für Namen und tatsächliche sichtbare
Namensbounds/Textdarstellung prüfen. Der vorhandene `file_manager_layout()`-Test
findet `acceptance-file.txt` nur im Datenmodell. Das beweist keine lesbaren Pixel.

### NF02 — P2: Handlungsanleitung im schmalen Agentenfenster abgeschnitten

**Korrektur:** bestehendes [#153](https://github.com/byte5ai/zaplex/issues/153)
für diesen verbleibenden Layoutpunkt wieder geöffnet. Bezug: lesbare
Handlungsanleitungen, Zustände und schmale Variante aus #564.

Im [540×670-Bild](evidence/native-1201fc4f/subscription-agent-narrow-540x670.png)
läuft „Start a new conversation or return to the she…“ rechts aus dem sichtbaren
Bereich. Auch die allgemeine Shortcut-Hilfe wird hart beschnitten. Der
Session-Ende-Text ist gekürzt; „New conversation“ und „Back to shell“ bleiben
sichtbar. Das [1180×640-Bild](evidence/native-1201fc4f/subscription-agent-normal-1180x640.png)
zeigt separat den gesetzten Approval-Zustand mit vier sichtbaren Aktionen.
Diese beiden Bilder sind unterschiedliche Zustände, keine Vorher-/Nachher-Folge
eines echten Providerlaufs.

Testauftrag in #153: Anleitung umbrechen oder vollständig zugänglich machen;
die tatsächlichen Textzeilen zusätzlich zu Container-/Buttonbounds prüfen.
Die erfolgreiche Geometrieprüfung allein erfasst abgeschnittene Texte nicht.

## Eng begrenzte positive Beobachtungen

| Gegenstand | Sichtbarer Beleg | Grenze |
|---|---|---|
| Connections-Hierarchie | Sechs Aufnahmen mit 250/320/480 Punkten, hell/dunkel: Aktionsslots bleiben rechts; lange Titel werden gekürzt, Recovery-Metadaten umbrechen. | Gesetztes Inventar; Tooltip-Vollständigkeit, echte Verbindung, DE und freie Bedienung nicht geprüft. |
| Favoriten-Flyout | `native-favorite-flyout.png`: seitliches Menü mit New agent/Edit/Remove; Elternmenü bleibt sichtbar. | Keine vollständige Identitäts-/Tooltipbewertung langer Favoriten, kein Verbindungsaufbau. |
| Dateiaktionen | Bei 1180 Punkten zwei, bei 760 vier Reihen; F2–F8/F10 und Zielzeile sichtbar, Fokuswechsel und F7-Dialog aufgenommen. | NF01 verhindert eine Freigabe der gesamten Dateiliste. |
| Rückkehr zur Shell | `native-file-managers-restored-terminals.png`: zwei Terminals und der erhaltene Entwurf `printf file-manager-draft`. | Kein Neustart-/Remote-Restore und keine freie Exploration. |
| Pane-Drag | `native-panes-side-by-side.png` und `native-panes-after-drag.png`: nebeneinander → übereinander; Entwurf bleibt sichtbar, expliziter Tabtitel steht oben. | Native Interaktionsassertions für diese feste lokale Folge; keine vollständige Vier-Richtungen-/Mehrhost-Abnahme. |

Diese Beobachtungen ersetzen keine vollständige `pass`-Zeile der
[vorab festgelegten Matrix](MATRIX.md). Die vollständigen Abläufe am neuen
installierten DMG sind noch nicht durchgeführt; die unabhängige Exploration
benötigt zusätzlich den bereits erfassten isolierten GUI-Zugang.

## Erneuter Nachweis mit Paketversion 1.2.2

Die nativen Layout-Tests in [Batch 39257382471](https://github.com/byte5ai/zaplex/actions/runs/39257382471)
sind auf `cadcb19130bbe0630f39bdd61b4e183ff154981a` erfolgreich. Die erneute
Sichtprüfung bestätigt trotzdem beide oben beschriebenen Fehler:

- **NF01 / #572:** Bei [1180×640](evidence/native-cadcb191/native-file-managers-normal.png)
  sind `projects` und `acceptance-file.txt` lesbar; bei
  [760×640](evidence/native-cadcb191/native-file-managers-narrow.png) verschwinden
  Namen und Name-Spaltenkopf weiterhin vollständig.
- **NF02 / #153:** Im [540×670-Agentenfenster](evidence/native-cadcb191/subscription-agent-narrow-540x670.png)
  sind Handlungsanleitung und Shortcut-Zeile weiterhin abgeschnitten.

Die drei erneut visuell geprüften Originalbilder sind mit Quelle, Teststand,
Artefakt-ID und SHA-256 im [Manifest](evidence/native-cadcb191/manifest.json)
gesichert. Auch diese Aufnahmen stammen aus nativen Integrationstest-Fixtures;
sie sind keine Aufnahmen des installierten DMGs und keine freie Exploration.

## Build-Nachweis und verbleibende Grenze

Auf demselben Commit bestehen Linux-/macOS-Compile- und Regressionstests,
4.386 Pakettests (13 bereits zuvor übersprungen) sowie vier Dokumentationstests
(vier ignoriert). Der zuvor fehlgeschlagene Verzeichnis-Austauschtest besteht
nach der Korrektur aus [#575](https://github.com/byte5ai/zaplex/pull/575)
unverändert. Die isolierte Live-SFTP-Prüfung ist ebenfalls grün; sie ersetzt
keinen nativen Remote-/Provider-Ablauf. Ihr Namensfilter erfasst sechs der sieben
ignorierten Backend-Fälle; die verbleibende Auswahl-Lücke ist in
[#576](https://github.com/byte5ai/zaplex/issues/576) erfasst.

Der anschließende macOS-App-Build verlor am 19.10.2026 um 14:01 UTC die
Runner-Verbindung. GitHub meldet keinen Compilerfehler oder Exit-Status für den
Build-Schritt; die Job-Logs sind nicht verfügbar. Der Status des Runners nach
dem Abbruch war unauffällig, die Ursache ist noch ungeklärt. Der Vorfall ist im
zuständigen privaten Infrastrukturprojekt erfasst. Der parallele Linux-Daemon-
Build ist erfolgreich; sein Artefakt `zap-remote-server-linux-x86_64`
(ID `12192687872`) ist im abgeschlossenen Lauf vorhanden. Ein neues Test-DMG und
die darauf folgenden GUI-Abläufe sind damit noch nicht nachgewiesen.

## Gezielte Fortsetzung und installiertes DMG

Dieser spätere Nachweis ersetzt die oben dokumentierte Build-Blockade:
[Run 39295080993](https://github.com/byte5ai/zaplex/actions/runs/39295080993)
baute die App auf unverändertem `cadcb191` mit `CARGO_BUILD_JOBS=4` erfolgreich
(37 min 47 s für den Build-/Vorbereitungsschritt). Der frühere erfolgreiche
Blacksmith-Daemon wurde übernommen und sein SHA-256 geprüft. Signierung,
Notarisierung und Stapling bestanden; kein neuer Gesamtbatch und kein Release.
Der Erfolg stützt die Speicherhypothese zum früheren VM-Neustart, beweist sie
aber nicht. Ein vorheriger Cargo-HTTP/2-Abruffehler wurde im Build-Schritt durch
HTTP/1.1 vermieden.

Die GUI-Fortsetzung
[39303447063](https://github.com/byte5ai/zaplex/actions/runs/39303447063)
installierte das unveränderte DMG 1.2.2 mit SHA-256
`313303cc2dd14b820cd9c01bb931b555300864914b7fc746f4c6841e4889bae3`.
App und XCTest stammen von `cadcb191`; ausschließlich der Shell-Starter stammt
von `9e200e30a39eb11d22020afa599e5838ec724104`, um den nachgewiesenen
Bash-3.2-Fehler bei leerer Filterliste zu korrigieren. Beide Revisionen stehen
getrennt in den Artefakten. Zuvor wurde die übersprungene GUI-Abhängigkeit
korrigiert; App und DMG wurden dafür nicht nochmals gebaut.

**15 Fälle tatsächlich ausgeführt: 12 bestanden, drei fehlgeschlagen, null
übersprungen.** Die exakte Fallmenge wurde unabhängig mit den 15 erwarteten IDs
abgeglichen. [result.json](evidence/dmg-cadcb191/result.json) hält Identität,
Fallresultate, Originalbeleg-Hashes und die Zuordnung der Fehlerbilder fest.
Rohbelege liegen im GUI-Artefakt `12212937860` (14 Tage Aufbewahrung).

| Fehlgeschlagener Fall | Geprüfter Befund | Aussagegrenze / Folgeauftrag in #562 |
|---|---|---|
| `testCancelSplitKeepsOriginalShell` | Scheitert bereits im Setup: [Befehl steht sichtbar im Eingabefeld](evidence/dmg-cadcb191/setup-command-not-executed.png), erwartete Ausgabe fehlt. | Der eigentliche Abbruchablauf wurde nicht erreicht. Bereitschaft des Terminals und Verarbeitung der Return-Eingabe untersuchen. |
| `testSplitLeft` | [Neues linkes Pane sichtbar](evidence/dmg-cadcb191/left-pane-command-not-executed.png); eingegebener Befehl bleibt ohne erwartete Ausgabe im Eingabefeld. | Kein bloßer OCR-Lesefehler im geprüften Bild. Ursache zwischen Testzeitpunkt und App-Eingabeverarbeitung noch offen; keine bestätigte Split-Geometrie-Regression. |
| `testNativeCloseButtonClosesWindow` | Accessibility zählt vor dem Klick acht Fenster, danach sieben; das sichtbare Vorderfenster wechselt. | `firstMatch.exists` prüft danach das nächste Fenster. Fehlerhafte Testidentität; der Klick schließt in diesem Lauf ein Fenster. Keine erneute Ampel-Regression daraus ableiten. |

Die drei Fälle bleiben im Lauf fehlgeschlagen. Bestandene Fälle werden nicht
erneut ausgeführt; es wurde kein weiterer Lauf ohne neue Korrektur gestartet.
Diese lokale deterministische Suite ersetzt weder die vollständige Varianten-
matrix noch Remote-/Provider-Abnahme und unabhängige Exploration.


## Sichtvergleich am installierten DMG 1.2.2

Die folgenden Originalaufnahmen aus GUI-Lauf 39303447063 wurden nachträglich
visuell gegen C:C1, L2 und C:F2 der vorab festgelegten Matrix geprüft. Damit sind
Funktionserfolg und visuelles Ergebnis getrennt bewertet. Herkunft, Prüfsummen
und AX-Fenstermaße: [visual-review.json](evidence/dmg-cadcb191/visual-review.json).
Englisch, dunkel; keine Aussage über unaufgenommene Varianten oder Exploration.

| Fall / konkrete Variante | Sichtbare Beobachtung und Sollvergleich | Ergebnis |
|---|---|---|
| C01, leere Connections, 1024×678 Punkte Fenster, etwa 250 Punkte Sidebar | Die Anleitung endet hinter „Use the folder b…“; der nächste Schritt mit Plus fehlt. Die betreffende linke Fläche ist nicht vom Systembanner verdeckt. [Originalbild](evidence/dmg-cadcb191/connections-empty-clipped.png). | **fail**, #565; nun am aktuellen DMG bestätigt. |
| L02, leere Favoriten im linken Split-Menü, 1024×678 Punkte | „No favorite hosts yet — ★ a h…“ bleibt abgeschnitten. Die Anweisung zum Favorisieren ist nicht vollständig sichtbar. [Originalbild](evidence/dmg-cadcb191/favorites-empty-clipped.png). | **fail**, #566; nun am aktuellen DMG bestätigt. |
| F02, einzelner Dateimanager, AX-Fenster 513×680 Punkte | Alle acht Aktionen F2–F8/F10 mit Kurzbeschriftung in zwei Reihen, Dateiname SAMPLE und Zielwahl sichtbar. [Originalbild](evidence/dmg-cadcb191/file-actions-513-points.png). | **pass nur für diese Ansicht**; keine Freigabe der gesamten Breitenfolge oder der zwei schmalen Dateipanes aus #572. |

Der Banner oben rechts verdeckt Teile der App. Die hier bewerteten Anleitungen
links bzw. die untere Aktionsleiste bleiben vollständig einsehbar. Verdeckte
Bereiche werden nicht bewertet. Weitere Varianten, freie Bedienung und echte
Remote-/Providerabläufe bleiben ohne den dafür angeforderten Zugang ungeprüft.


## Gezielte Nachprüfung: drei zuvor fehlgeschlagene Fälle bestanden

[Run 39329863382](https://github.com/byte5ai/zaplex/actions/runs/39329863382)
führt nach [PR #583](https://github.com/byte5ai/zaplex/pull/583) genau die drei
zuvor fehlgeschlagenen Fälle aus: **3 bestanden, 0 fehlgeschlagen, 0 übersprungen**.
Die Swift-Tests wurden im macOS-Job erfolgreich kompiliert. Testrevision:
`1f2a9b16595318ef695873b8ebbf3f5414a2fff0`; App weiterhin `cadcb191`, DMG-Digest
unverändert `313303cc2dd14b820cd9c01bb931b555300864914b7fc746f4c6841e4889bae3`.
Kein App-/Daemon-Neubau und keine Wiederholung der zwölf erfolgreichen Fälle.

- Schließen: AX zählt vorher drei und danach zwei Fenster. Die korrigierte
  Assertion prüft die Anzahl statt des neu aufgelösten `firstMatch`.
- Split-Abbruch: [Bild](evidence/dmg-cadcb191/cancel-preserves-shell.png) zeigt
  dieselbe Shell mit `qa_state=741` nach Escape und nach dem anschließenden
  Pane-Wechselversuch; kein zusätzliches Pane bleibt bestehen.
- Split links: [Bild](evidence/dmg-cadcb191/left-split-independent-shells.png)
  zeigt frische Ausgabe im linken Pane mit Zustand 963 und die erhaltene
  ursprüngliche Shell rechts mit Zustand 741.

Vor jedem Return wurde die vollständig angekommene Eingabe geprüft; jeder
Befehl wurde genau einmal abgeschickt. Der Lauf erfasste keine der ausgewählten
App-Ablehnungen. Das stützt die Korrektur der Eingabesynchronisierung, beweist
aber rückwirkend nicht die Ursache beider früheren Eingabefehler.

[retry-result.json](evidence/dmg-cadcb191/retry-result.json) enthält die exakten
Fall-IDs, Revisionen, Digests und Belegzuordnung. Zusammen mit dem vorherigen
Lauf liegt damit für jeden der 15 lokalen Fälle ein Erfolg am selben DMG vor.
Es handelt sich ausdrücklich um zwei Läufe mit unterschiedlichen Testrevisionen,
nicht um einen vollständigen grünen Lauf der neuen Tests. `local_gui` und
`product_acceptance` bleiben im Teillauf korrekt `not-run`; der Release-Gate
wird nicht umgangen. Die Gesamtprüfung benötigt weiterhin die in #562/#564
benannten Varianten und die separat angeforderte GUI-/Remote-/Providerumgebung.


## Workspace-Korrekturen nativ bestätigt am 20.10.2026

[Run 39339343993](https://github.com/byte5ai/zaplex/actions/runs/39339343993),
App- und Testrevision `1800ec77c61690f5d2c54ac29e33ce95da832357`, Version 1.2.4:
**Vier Workspace-Fälle bestanden, null übersprungen.** Der gesamte Lauf ist
wegen des separaten Agentenfalls fehlgeschlagen. Diese Auswertung erklärt den
Lauf nicht nachträglich für grün.

Die [30 Original-PNGs samt SHA-256 und Herkunft](evidence/native-1800ec77/manifest.json)
sind unverändert archiviert und sämtlich visuell geprüft. Das
[Ergebnisprotokoll](evidence/native-1800ec77/result.json) trennt Fallstatus,
Sichtprüfung und Aussagegrenzen. Native App mit echtem Display, echten isolierten
lokalen Dateien und PTYs; kein installiertes DMG. Die angeforderte Sprache wurde
im jeweiligen Prozess vor den Aufnahmen ausdrücklich geprüft.

| Korrektur / konkrete Variante | Beobachtung | Ergebnis |
|---|---|---|
| #565, Connections leer, DE/EN × Hell/Dunkel × 250/320/480 Punkte Sidebar | Ordner- und Plus-/Serverhinweis vollständig sichtbar; letzte gerenderte Zeichenbounds innerhalb des Panels. Beispiele: [DE/250 dunkel](evidence/native-1800ec77/connections-de/native-empty-connections-dark-250.png), [EN/250 hell](evidence/native-1800ec77/connections-en/native-empty-connections-light-250.png). | pass für diese Anleitung und Varianten |
| #566, Favoriten leer, DE/EN × Hell/Dunkel, gemeinsames Startmenü | Vollständiger Hinweis einschließlich „Seitenleiste“ / „sidebar“ in zwei bzw. drei Zeilen; deaktivierte Hilfe bleibt im Menü. Beispiele: [DE dunkel](evidence/native-1800ec77/connections-de/native-empty-favorites-dark.png), [EN hell](evidence/native-1800ec77/connections-en/native-empty-favorites-light.png). | pass für diesen Menüplatz; übrige Einstiege/Randlagen nicht geprüft |
| #572, zwei Dateipanes, 1180×640 und 760×640, DE/EN × Hell/Dunkel | Name-Spaltenkopf und drei Namen bleiben in beiden Panes lesbar. Schmal entfallen Größe/Datum; Zielwahl und F2–F8/F10 bleiben sichtbar. Reale Namens-Textbounds mit mindestens 100 Punkten Breite und gezeichnete Glyphen geprüft. Beispiele: [DE schmal dunkel](evidence/native-1800ec77/files-de/native-file-managers-narrow.png), [EN schmal hell](evidence/native-1800ec77/files-en/native-file-managers-narrow-light.png). | pass für diese Ansichten |
| Vollständiger Dateiname und F7-Fokusfolge | [Tooltip](evidence/native-1800ec77/files-de/native-file-manager-full-name.png) zeigt `other-project-with-a-long-name.txt`. Nach Fokuswechsel öffnet F7 den [Ordnerdialog links](evidence/native-1800ec77/files-de/native-file-manager-focused-action.png); rechts bleibt die Liste erhalten. | pass für diese feste lokale Folge |
| Rückkehr zur Shell | Nach Dialogabbruch und Schließen beider Dateiansichten sind beide Terminals und der [Entwurf `printf file-manager-draft`](evidence/native-1800ec77/files-de/native-file-managers-restored-terminals.png) erhalten. | pass für diesen lokalen Rückweg |

Der separate Agentenfall scheitert wie zuvor in Run 39337035377 bei
`narrow/recoverable_error_with_session`: Eingabebereich bis y=671,8 bei
670 Punkten Fensterhöhe. PR #587 hatte eine andere Hilfeansicht geändert;
die Annahme, damit die Überhöhe zu beseitigen, wurde durch den zweiten Lauf
widerlegt. Die vier erfolgreichen Workspace-Fälle werden bei der weiteren
Agenten-Nachprüfung nicht wiederholt.

Die vollständige #564-Matrix bleibt davon getrennt: kein neuer installierter
DMG-Nachweis dieser Korrekturen, keine freie Exploration, keine echte
Remote-/Providerinteraktion und keine Kontrast-/Reduced-Motion-Abnahme.


## Diagnose nach drei fehlgeschlagenen Agentenläufen

Der ausschließlich auf den Agentenfall begrenzte
[Run 39341539630](https://github.com/byte5ai/zaplex/actions/runs/39341539630)
scheitert auf `9c7aaf5b` (1.2.5) erneut mit denselben Bounds. Damit ist auch die
Annahme widerlegt, eine kleinere Umbruchschwelle der Identitätsfelder würde
diesen Überstand beseitigen. Die vier erfolgreichen Workspace-Fälle wurden
nicht wiederholt.

Die vor der Assertion gesicherte [Fehleraufnahme](evidence/native-9c7aaf5b/subscription-agent-recoverable-error-540x670.png)
und der vollständige Snapshot zeigen nun die gesamte Konfiguration unterhalb
der primären Wiederherstellungsaktionen: Agent-/Konto-/Modellwechsel sowie
Verzeichniswahl. [Herkunft, Prüfsumme und Bounds](evidence/native-9c7aaf5b/result.json).
Diese Konfiguration ist auch bei einem wiederherstellbaren Fehler absichtlich
verfügbar, unter anderem für eine nicht mehr verfügbare Route; die bestehende
Zustandslogik und ihr Regressionstest bleiben erhalten.

Die Pipeline wurde vor einem weiteren Versuch überprüft: genau ein Fall läuft,
bereits erfolgreiche Fälle bleiben ausgespart, Fehlerbilder werden vor der
Assertion gesichert und Logs enthalten den vollständigen Layout-Snapshot.
Die weitere Korrektur entfernt die erzwungene volle Zeile der drei kurzen
Wechselaktionen und verwendet denselben natürlichen Umbruch wie die primären
Aktionen. Der unwirksame Versuch an den Identitätsfeldern wird zurückgenommen.
Eine erfolgreiche native Nachprüfung dieser Korrektur liegt hier noch nicht vor.
