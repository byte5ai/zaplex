# Native Abnahme des Kandidaten 1.2.37

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).

App- und Testquellstand: `a0705b38a92c8fe2877d43492f5190d372a053de`,
PR [#705](https://github.com/byte5ai/zaplex/pull/705). Dieser Stand korrigiert
die deutschen Einstellungswerte (#701), Kontext-Chip-Farben nach Themewechsel
(#702) und die Farben des Ausschlusslisten-Editors (#704).

## Batch-Ergebnis und OCR-Diagnose

Die abgeschlossenen Compile-, Clippy-, Keymap-, Abhängigkeits- und SFTP-Gates
sind erfolgreich. Die Linux-Suite meldet **4436 bestanden, 13 übersprungen**;
Dokumentationstests melden **4 bestanden, 4 ignoriert**. Diese Ausnahmen werden
nicht als ausgeführte Tests gezählt. Phase A ist ebenfalls bestanden.
Paketbau und Signatur-/Notarisierungsprüfung sind ebenfalls erfolgreich.
Die installierten GUI-Pflichtfälle melden **14 bestanden, 2 fehlgeschlagen,
0 übersprungen**. Der Originalbatch bleibt damit fehlgeschlagen.

Die beiden Fehler sind anhand der Originalbilder und gespeicherten OCR-Werte
als Erkennungsfehler eingegrenzt:

- `NavigationTests/testClosingTabPreservesOriginalShell`: Schon der gemeinsame
  Startnachweis scheitert, bevor die Tab-Aktionen erreicht werden. Sichtbar ist
  `ZQA 799030 963`; die OCR liefert `ZQA 7990308 963`.
- `NavigationTests/testSettingsReturnToOriginalShell`: Die Einstellungsseite
  und „Keyboard shortcuts“ sind sichtbar; der OCR-Text enthält diese Beschriftung
  nicht. Die Rückkehr zur Shell wird in diesem Fall nicht erreicht.

Die drei unveränderten Originalbilder (beide Fehler plus nicht abgeschickter
Befehl als Gegenprobe), Hashes und AX-Fensterrechtecke liegen unter
[`fixtures/ocr/provenance-137.json`](../../script/macos/gui-tests/fixtures/ocr/provenance-137.json).
Der [Tesseract-Dokumentation](https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html#page-segmentation-method)
zufolge nimmt der bisherige Modus 6 einen einheitlichen Textblock an; Modus 11
sucht verstreuten Text. Die native Diagnose vergleicht Segmentierung und
Skalierung auf alten und neuen Originalen, bevor die feste Konfiguration
geändert wird. Exakte Nonce, Shellzustand, negative Gegenproben und richtige
Bildkoordinaten bleiben die Abnahmekriterien. Die anschließende Fortsetzung
prüft ausschließlich die zwei fehlgeschlagenen Nutzerabläufe am unveränderten DMG.

Die [native Diagnose 40146026138](https://github.com/byte5ai/zaplex/actions/runs/40146026138)
auf `d9f60714fee76ba9f96452e66364609a2a1eee43` ist erfolgreich abgeschlossen.
144 Lesungen vergleichen neun Bilder mit 16 Konfigurationen. Die
[Auswertung](evidence/candidate-137-native/ocr-diagnosis.json) hält die exakten
Positiv-/Negativprüfungen und die unabhängig gemessenen Koordinaten fest.
Bei zweifacher Pixelwiederholung besteht Modus 11 **9/9**, während Modus 6
beide aktuellen Fehler reproduziert (**7/9**). PR #707 stellt den Testhelfer auf
diese eine feste Konfiguration um. Die Diagnose allein ist kein GUI-Pass.

## Installierte GUI-Abnahme vervollständigt

[Fortsetzungslauf 40146604008](https://github.com/byte5ai/zaplex/actions/runs/40146604008)
auf Teststand `88d754b123fdf4d44dd241e281e3272de0c08e36` besteht die vollständige
Swift-OCR-Gegenprobe einschließlich Retina-Abbildung und ausschließlich die
beiden zuvor fehlgeschlagenen Nutzerabläufe. App und DMG bleiben unverändert.

Der [kombinierte Nachweis](evidence/candidate-137-native/gui-completion.json)
bestätigt **16/16 lokale GUI-Pflichtfälle** aus Original und Fortsetzung. Er
gleicht die tatsächlichen Testbäume, Zähler, Prozessausgänge, Quellstände,
GitHub-Jobstatus und Paket-/Artefakt-Hashes ab. Jeder Fall nennt seinen eigenen
Lauf; die 14 ursprünglichen Erfolge wurden nicht wiederholt. Der Originalbatch
bleibt fehlgeschlagen. Produktmatrix, Remote-/Provider-Varianten und die
direkte Einstellungsprüfung bleiben von diesem funktionalen Nachweis getrennt.

Fortsetzungsartefakt `12580912705`, ZIP-SHA256:
`7e3336b86ebf4965760000c2d22fac1527d8408d2a41505c81cab551616cb878`.

| Artefakt | GitHub-ID | SHA-256 |
|---|---|---|
| App-ZIP | `12579753960` | `792a5ad5e575507839dae0f4410aaaf2c6f3382a698dcab7e9ea4401825d805e` |
| Enthaltenes DMG, 195238241 Bytes | siehe App-ZIP | `7aaa925da5abbced8fb537ed4af8a5fdbdba92b8bf8cd1a19bb65328f296213e` |
| Daemon-ZIP | `12579553847` | `5838bad7b4c39142cb49afb2289b338f5a050dc0ad5c5bdc100e30e62547aac6` |
| Enthaltenes Daemon-Archiv, 45541087 Bytes | siehe Daemon-ZIP | `d6e65560081b21b06bae1077318beaf1982c889c2540bd44d0fe951138808b55` |
| GUI-Belege | `12579744904` | `c07c035b55c473f2c3348774d9857f76aaca587b9d25970fe6261bb1de810cd3` |

## Nativer Fixture-Nachweis

Der [CI-Batch 40140544738](https://github.com/byte5ai/zaplex/actions/runs/40140544738)
hat das macOS-Compile-Gate samt nativen Fixture-Prüfungen bestanden.
Das abgeschlossene Jobprotokoll bestätigt sechs Plattformtests, einen
Agenten-Layouttest, sechs Workspace-Fälle und einen Header-Einstiegstest,
jeweils ohne Fehler oder übersprungene Tests.
Die später vervollständigte installierte GUI-Abnahme ist davon getrennt
oben dokumentiert.

Alle **44 Originalbilder** wurden einzeln betrachtet. Die heruntergeladenen
ZIP-Hashes stimmen mit den GitHub-Artefakt-Digests überein; zusätzlich stimmen
alle 41 Workspace-Bildhashes mit dem vom Lauf erzeugten Manifest überein.
Das [Bildverzeichnis](evidence/candidate-137-native/manifest.json) enthält
Artefakt-IDs, SHA-256, Bildmaße und Beobachtungen pro Original.

| Umfang | Beobachtung und Aussagegrenze |
|---|---|
| 3 Agentenbilder | Freigabe-, beendeter und wiederherstellbarer Zustand zeigen erreichbare Aktionen. Lange Identitätswerte sind gekürzt. Keine echte Provider-Sitzung. |
| 7 Verbindungsbilder | Gefüllte Listen bei 250/320/480 Punkten in Hell/Dunkel zeigen feste Aktionsplätze und vollständigen Host-Tooltip; ein Favoriten-Flyout ist erfasst. Deterministische Testdaten, kein echter Verbindungsaufbau. |
| 16 Leerzustände | Deutsche und englische Verbindungs- und Favoritenanleitungen sind in den dargestellten Breiten/Themes vollständig lesbar. |
| 14 Dateimanagerbilder | DE/EN, Hell/Dunkel, 1180/760 Punkte: alle acht Aktionen in zwei beziehungsweise vier Reihen, unterscheidbare aktive/inaktive Pane, vollständiger Dateiname im Tooltip und Neuer-Ordner-Dialog in der fokussierten Pane. Nach Rückkehr ist der Terminalentwurf sichtbar. Kein Nachweis einer einreihigen Leiste oder vollständiger Bedienvarianten. |
| 2 Pane-Bilder | Vorher/Nachher zeigen dieselben beschrifteten Panes nebeneinander beziehungsweise übereinander und den erhaltenen Entwurf. Kein Bewegungsbeleg. |
| 2 Header-Bilder | Der Einstieg öffnet das ursprüngliche linke Verzeichnis neben dem unveränderten Geschwisterverzeichnis; der Fokus wechselt nach links. |

In diesen dargestellten Zuständen wurde keine neue sichtbare Regression
festgestellt. Die Bilder enthalten **nicht** die gezielte native Nachprüfung
der drei Einstellungsfehler. Die bestehende [Produktmatrix](MATRIX.md) bleibt
für nicht beobachtete Kombinationen offen.

## Vorab festgelegte Nachprüfung am installierten DMG

- DE/EN nach tatsächlicher Sprachauswahl und vollständigem App-Neustart:
  Keine/None, Standard/Default, Agenten/Agents und Links/Rechts bzw. Left/Right;
  physische Tastennamen bleiben erhalten.
- Dark→Light→Dark: Kontext-Chips, eingegebener Ausschlusslistenwert und
  Platzhalter bleiben lesbar; Auswahl, Eingabe und gewählte Toolbar-Elemente
  bleiben erhalten. Relevante deaktivierte Zustände getrennt prüfen.
- Geänderte Kontrollen und Menüeinträge bei 480×320 erreichen; anschließend
  frische Shell-PID, nicht exportierten Marker und CWD nachweisen.
- Weitere direkte Exploration der Dateimanager-Aktionsleiste: eine, zwei und
  vier Reihen, fokussierte Pane und unveränderte künstliche Dateiinhalte.

Je Interaktion werden Zustand vorher/nachher, Aktion, Erwartung und Beobachtung
festgehalten. Standbilder gelten nur als Zustandsbelege. Es erfolgt keine
Release-Freigabe aus diesen Teilnachweisen.

## Direkte Einstellungsprüfung am installierten DMG

[Exploration 40146950306](https://github.com/byte5ai/zaplex/actions/runs/40146950306)
ist erfolgreich abgeschlossen und ausdrücklich beendet. App und DMG entsprechen
dem obigen Kandidaten; der Teststand ist `88d754b123fdf4d44dd241e281e3272de0c08e36`.
Alle **46 Originalaufnahmen** wurden einzeln betrachtet und stimmen bytegenau
mit dem archivierten Operator-Verzeichnis überein. Das
[Manifest](evidence/candidate-137-native/exploration-40146950306.json)
enthält pro Aufnahme Hash, Fenstergeometrie und Beobachtung. Artefakt
`12581575270`, ZIP-SHA256
`d877e70f781bceb7080d92524f37fcce031ef99d101e5f66396d27d0cd0861c7`.

| Nachprüfung | Direkt beobachtetes Ergebnis |
|---|---|
| #701, Sprache | EN: Agents, Default, None und Left/Right. Nach tatsächlicher Sprachauswahl und vollständigem Neustart: Agenten, Standard, Keine und Links/Rechts. Alle sechs angebotenen Tastenoptionen sind sichtbar; Fn, Option und Control bleiben erhalten. Keine Sprach-/Modellanfrage ausgelöst. |
| #702, Kontext-Chips | EN und DE: Dark→Light→Dark mit lesbaren Texten und Symbolen. Maus-Drag fügt `alice` links ein; im englischen Ablauf bleibt die Anordnung beim Themewechsel erhalten, Entfernen stellt den verfügbaren Chip wieder her. Im deutschen Ablauf stellt „Standard wiederherstellen“ nach Drag die ursprüngliche Anordnung wieder her. |
| #704, Ausschlussliste | `draft-zaplex137` bleibt nach Themewechseln und vollständigem Sprachneustart erhalten und lesbar. Nach Leeren ist der deutsche Platzhalter in Light und Dark lesbar. Die vor dem ersten Wechsel markierte Teilzeichenfolge ist dokumentiert; ihr Erhalt nach Fokusverlust ist visuell nicht unabhängig belegt. Der bestehende gezielte Unit-Test deckt Auswahl/Fokus getrennt ab. |
| Mindestgröße | Bei 480×320 ist die Seite horizontal scrollbar. Die zunächst rechts abgeschnittenen Kontrollen sind erreichbar; alle sechs Einträge des deutschen Tastenmenüs passen im hellen Theme vollständig ins Fenster. Danach wieder 1024×678. |
| Shell-Rückweg | Nach dem Sprachneustart frischer Nachweis `PID 2837 / DE137 / /tmp`; nach erneutem Öffnen und Schließen der Einstellungen derselbe nicht exportierte Marker, dieselbe PID und dasselbe Verzeichnis. Die historische Ausgabe vor dem App-Neustart wird nicht als Persistenznachweis gezählt. |

Ausgewählte unveränderte Originale:
[Ausschlussliste Light](evidence/candidate-137-native/10-denylist-light.png),
[beibehaltene Chips Dark](evidence/candidate-137-native/16-chips-dark-retained.png),
[deutsches Tastenmenü](evidence/candidate-137-native/24-de-voice-options.png),
[deutsche Chips Light](evidence/candidate-137-native/27-de-chips-light.png),
[Menü bei Mindestgröße](evidence/candidate-137-native/37-min-menu.png),
[frischer Shell-Nachweis](evidence/candidate-137-native/45-return-output.png).

In den geprüften Zuständen sind die drei korrigierten Fehler nicht erneut
aufgetreten. Deaktivierte Varianten, Bewegungsbelege sowie die weiteren
Dateimanager-/Remote-/Provider-Fälle wurden in dieser Sitzung nicht geprüft;
daraus folgt keine vollständige Produktabnahme.

## Direkte Dateimanager-Prüfung und Fokusbefund

[Exploration 40148243776](https://github.com/byte5ai/zaplex/actions/runs/40148243776)
verwendet dasselbe DMG mit Teststand `c136c772e6437231c30824f8e82b02ecb62792f0`.
Die Sitzung ist ausdrücklich beendet. Alle **33 Originale** sind einzeln
betrachtet und bytegleich mit dem archivierten Operator-Verzeichnis;
[Manifest](evidence/candidate-137-native/exploration-40148243776.json).
Artefakt `12581960971`, ZIP-SHA256
`26fba290af9c65d279752ee6093644c553cbcda478c55db7b97589f1600cc52d`.

**F02, Teilprüfung EN/Dark:** Alle acht Kurzbeschriftungen sind bei 1024 Punkten
ohne Sidebar in [einer Reihe](evidence/candidate-137-native/files-06-one-row.png),
bei 760 Punkten in [zwei Reihen](evidence/candidate-137-native/files-07-two-rows.png)
und bei Fensterbreite 480 mit Sidebar in
[vier Reihen](evidence/candidate-137-native/files-09-four-rows-focused.png)
sichtbar. Die Zellen sind je Reihe gleich breit; keine horizontale Leiste
oder ausgeblendete Beschriftung. Andere Sprach-/Theme-Kombinationen sind in
dieser Sitzung nicht geprüft.

**F03, fail:** Ein Klick auf eine Dateizeile der inaktiven Pane aktiviert sie
nicht. Nach [linkem Zeilenklick](evidence/candidate-137-native/files-18-left-focus.png)
zeigt F3 weiterhin die [rechte Datei mit 16 Byte](evidence/candidate-137-native/files-19-left-details.png).
Nach Klick auf den linken Pane-Kopf zeigt F3 korrekt die
[linke Datei mit 7 Byte](evidence/candidate-137-native/files-22-left-details-after-header.png).
Die umgekehrte Gegenprobe reproduziert den Fehler:
[rechter Zeilenklick](evidence/candidate-137-native/files-23-right-row-click.png),
danach [weiter linke Details](evidence/candidate-137-native/files-24-right-row-f3.png).
Klicks auf den Pane-Kopf oder leere Fläche wechseln dagegen den Kontext.
Das ist [#709](https://github.com/byte5ai/zaplex/issues/709), Priorität High;
nativ wurde ausschließlich die nichtdestruktive Details-Aktion verwendet.

F10 kehrt nach korrekter Aktivierung in beide ursprünglichen Shells zurück:
links PID1304/LEFT137, rechts PID2248/RIGHT137, jeweils ursprüngliches CWD.
Die [unabhängigen Datei-Hashes](evidence/candidate-137-native/files-32-final-hash-output.png)
bleiben unverändert. Die zwei gleichnamigen Testdateien unterscheiden sich
bewusst in Inhalt und Größe. Die Session bestätigt keinen Transfer und keine
destruktive Dateiaktion; der Fokusfehler verhindert den Pass von F03.

## Lokale Korrektur für 1.2.38

Die Korrektur zu #709 lässt die Dateiliste auch bereits konsumierte Mausklicks
beobachten. Sie stellt zuerst den Listenfokus her und aktiviert anschließend
die zugehörige Pane. Vorhandene Markierungs- und Zeilenaktionen bleiben erhalten.
Die verzögerte Pane-Aktivierung erfolgt nach der Fokusaktualisierung und vermeidet
auch den Wiedereintritt in die gerade bearbeitete Browser-View.

Der neue Mausregressionstest scheiterte vor der Korrektur am fehlenden Pane-Wechsel
vor Mouse-up. Nach der Korrektur bestehen **neun gezielte Tests**, darunter beide
neuen Fälle: F3 erreicht in beiden Wechselrichtungen die richtige gleichnamige
Datei; ein Zeilenklick nach Suchfokus gibt den Listenfokus zurück und erhält
Markierungen, sowohl innerhalb der aktiven Pane als auch nach einem Pane-Wechsel.
Bestehende Gegenproben für Modifier-Klicks, Elternzeile, veraltete Dateiidentität,
deaktivierte Aktionen, Dialogfokus und Kontextmenü bestehen ebenfalls.

`cargo check --locked -p warp`, App-Clippy mit `-D warnings`, die Formatprüfung
der drei geänderten Rust-Dateien und alle 13 Phase-A-Prüfungen sind bestanden.
Die ausschließlich in Versionswerten geänderten Workflows liefern dieselben
Actionlint-Befunde wie der Basisstand; daraus wird kein vollständiger
Actionlint-Pass abgeleitet. Der [lokale Nachweis](evidence/focus-709-local.json)
nennt die tatsächlich ausgeführten Tests und bindet sie an Quell- und Loghashes.

**F03 bleibt nativ fehlgeschlagen**, bis ein neues 1.2.38-DMG den Zeilenklick in
beiden Richtungen und die Rückkehr aus dem Suchfeld bestätigt. Die alten
Aufnahmen werden nicht zum Nachweis der Korrektur umgedeutet.
