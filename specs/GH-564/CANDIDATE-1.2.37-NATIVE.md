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
Bildkoordinaten bleiben die Abnahmekriterien. Danach sind ausschließlich die
zwei fehlgeschlagenen Nutzerabläufe am unveränderten DMG nachzuprüfen.

Die [native Diagnose 40146026138](https://github.com/byte5ai/zaplex/actions/runs/40146026138)
auf `d9f60714fee76ba9f96452e66364609a2a1eee43` ist erfolgreich abgeschlossen.
144 Lesungen vergleichen neun Bilder mit 16 Konfigurationen. Die
[Auswertung](evidence/candidate-137-native/ocr-diagnosis.json) hält die exakten
Positiv-/Negativprüfungen und die unabhängig gemessenen Koordinaten fest.
Bei zweifacher Pixelwiederholung besteht Modus 11 **9/9**, während Modus 6
beide aktuellen Fehler reproduziert (**7/9**). Der Testhelfer wird auf diese
eine feste Konfiguration umgestellt. Die vollständige Swift-Gegenprobe samt
Retina-Abbildung und die zwei tatsächlichen GUI-Abläufe sind bis zum gezielten
Fortsetzungslauf weiterhin unbestätigt; die Diagnose allein ist kein GUI-Pass.

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
Die beiden fehlgeschlagenen installierten GUI-Fälle sind davon getrennt;
die vollständige DMG-Abnahme ist noch nicht bestätigt.

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
