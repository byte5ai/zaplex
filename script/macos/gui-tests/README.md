# Nutzertests am fertigen DMG

Der Workflow `gui-acceptance.yml` installiert die unveränderte App aus einem DMG
in einer frischen macOS-VM. Nur das separate XCTest-Bundle wird kompiliert.
Es gibt keinen App-Neubau, keine Test-Featureflags, keine vorab gefüllten
Zaplex-Einstellungen und keine Anmeldedaten im Testprofil.

## Pflichtfälle und unabhängige Erwartungen

| Ablauf | Erwartung |
|---|---|
| Start und Terminaleingabe | Fenster erscheint; `printf` erzeugt den erwarteten Text sichtbar im Fenster |
| Neues Fenster mit Cmd-N | Fensterzahl steigt um eins; eine Shell im neuen Fenster verarbeitet Eingabe |
| Nativer Schließen-Button | Ein echter Klick schließt das Fenster innerhalb von zehn Sekunden |
| App beenden und neu starten | Fenster erscheint wieder; Terminal nimmt Eingabe an und zeigt das Ergebnis |
| Tabs wechseln | Neue Shell ist unabhängig; Hin- und Rückwechsel erhalten den jeweiligen Zustand |
| Tab schließen | Ursprüngliche Shell bleibt erhalten und nimmt Eingaben an |
| Split rechts/links/unten/oben | Neue Shell ist unabhängig; Ausgabeposition stimmt mit der gewählten Richtung überein |
| Split abbrechen | Escape erhält die ursprüngliche Shell; kein neuer Pane bleibt zurück |
| Einstellungen | Sichtbar als Tab geöffnet; nach Schließen des Tabs bleibt die ursprüngliche Shell benutzbar |
| Dateimanager | Zeigt die Datei im Pane-Arbeitsverzeichnis und Dateiaktionen; Zurückwechseln erhält die Shell |
| Connections | Öffnet sichtbar; Schließen erhält die ursprüngliche Shell |
| Schmaler Dateimanager | Bei höchstens 700 Punkten Breite bleiben alle acht Dateiaktionen sichtbar |
| Mindestfenstergröße | Native Ziehgeste hält mindestens 480×320 Punkte; am Minimum und nach Vergrößern verarbeitet die ursprüngliche Shell Eingaben sichtbar |

Die erwartete Ausgabe steht nie vollständig im eingegebenen Befehl. Tesseract
liest die gerenderten Pixel; bloßes Echo der Eingabe oder ein interner Modellzustand
reicht nicht. XCTest-Anhänge enthalten Screenshots und den Accessibility-Baum.
Alle sechzehn Fälle müssen ausgeführt werden und bestehen; übersprungene Tests,
Zeitüberschreitungen und fehlende Ergebnisse lassen den Workflow scheitern.

## Ausführung

- Manuell: `gui-acceptance.yml` mit `release_tag` eines bestehenden Releases.
  Asset-ID und SHA-256 aus der GitHub-API werden vor der Installation geprüft.
  `suite=terminal` prüft gezielt die drei Terminalfälle ohne den Schließen-Button;
  dieser Diagnoselauf gilt ausdrücklich nicht als vollständige Abnahme.
  `suite=navigation` prüft nur die acht neuen Navigationsfälle. Shellzustände
  werden durch nicht exportierte Variablen und pro Prüfung neue Ausgabemarker
  geprüft. TSV-OCR liefert die Positionen der sichtbaren Ausgabe und Menüpunkte.
  `suite=surfaces` prüft Dateimanager, Connections und Mindestfenstergröße. `only_tests` begrenzt
  Diagnoseläufe auf bekannte Testnamen (`Klasse/testMethode`, kommagetrennt);
  damit können nur fehlgeschlagene Fälle erneut ausgeführt werden. `full`
  lehnt eine solche Einschränkung ab.
- Fortsetzung am vorhandenen CI-Artefakt: `gui-resume.yml` prüft dessen Herkunft.
  `only_tests` beschränkt den Lauf auf fehlgeschlagene Fälle; `current_tests=true`
  verwendet die XCTest-Revision des aktuellen Workflow-Commits. App- und
  Testrevision werden separat archiviert. Das DMG wird nicht neu gebaut.
  Ein solcher Teillauf bleibt diagnostisch und erteilt keine vollständige Abnahme.
- Im Batch: `test-dmg.yml` ruft den Workflow nach dem Verpacken auf und übergibt
  genau dessen DMG-Artefakt. Fehler lassen den Test-DMG-Workflow scheitern.
  Im Batch wird immer die vollständige Suite ausgeführt.
- Interaktive Exploration: `gui-explore.yml` verwendet ebenfalls das vorhandene
  DMG. Nach gezielten Fortsetzungen kann `gui_completion_run_id` deren Läufe
  benennen. Die Vorprüfung lädt alle Originalbelege und verlangt dieselbe App,
  Version und denselben DMG-Hash. Die Fortsetzungen müssen zusammen alle ursprünglich
  fehlgeschlagenen Pflichtfälle bestehen. Fehlende, doppelte, übersprungene oder
  zusätzliche Fälle sperren den Einstieg. Mehrere Lauf-IDs werden in Ausführungsfolge
  durch Kommas getrennt: Jeder Lauf prüft genau die verbliebenen Fehler; bereits
  bestandene Fälle werden nicht wiederholt. Der letzte Lauf muss alle Restfälle
  bestehen. `gui-completion.json` hält die Herkunft
  jedes Falls fest, ohne die ursprünglichen XCTest-Berichte umzuschreiben.
  Das bestätigt die lokale Fallabdeckung; die Release-Prüfung bleibt eigenständig. Das Bedienfenster ist auf 30 Minuten begrenzt, weil verzögerte Bild- und
  Eingabeabrufe wiederholt das frühere Zehn-Minuten-Fenster überschritten haben.
  Die Datei `finished` im Verzeichnis aus `/tmp/zaplex-exploration-path` beendet
  das Fenster vorzeitig; anschließend folgen Aufräumen und Belegsicherung.
  Ein erfolgreicher Bereitstellungslauf ist keine Produktabnahme.
- Native Fehlerdiagnose: `gui-explore.yml` mit `diagnostic=true` öffnet dasselbe
  verifizierte DMG auch bei unvollständiger GUI-Abnahme für die Ursachenklärung.
  Die Bau-/Signier-/Paketgates bleiben erforderlich. Dieser Modus lässt sich nicht
  mit Completion-Behauptungen kombinieren und erzeugt ausschließlich
  `gui-diagnosis.json` mit `local_gui` und `product_acceptance` auf `not-run`.
  Der normale Abnahmepfad und die erforderlichen Nutzerabläufe bleiben unverändert.
- Vor Veröffentlichung: `zap_release.yml` prüft sein `zap-macos-arm64`-Artefakt
  mit Tests von der unveränderlichen Build-Revision. Ein Fehler sperrt die
  Veröffentlichung. Belege werden separat aufbewahrt und nicht als Release-Assets
  hochgeladen oder zusammen mit den Zwischenpaketen gelöscht.
- Die Job-VM benötigt Xcode, xcodegen und die Werkzeuge `macrunner-gui-check` und
  `macrunner-gui-run`. Die GUI-Vorprüfung ist verpflichtend.
- Jeder Lauf archiviert DMG-Hash, App-Metadaten, Test-Commit, Werkzeugversionen,
  Build-/Testlogs, `.xcresult`, Screenshots und die strukturierte Testzusammenfassung.
  Ein fehlgeschlagener Test wird nicht automatisch wiederholt.

Der erste Pilot nutzt `v1.2.0-rc1`. Die Ampel-Korrektur in 1.2.1 ist dort noch
nicht enthalten; Testergebnisse beziehen sich auf den dokumentierten DMG-Hash.

Die OCR verwendet Tesseract aus einem Homebrew-Binärpaket ausschließlich im
Testprozess; die App erhält keine neue Laufzeitabhängigkeit. Die Version wird
archiviert. Apples Vision verlangte in der macOS-27-VM sowohl mit der Standardrevision
als auch mit Revision 2 und `usesCPUOnly` einen nicht verfügbaren Neural-Engine-Backend.

## Aussagegrenzen

Dies ist eine lokale Funktionsabnahme, keine vollständige Produktabnahme.
`acceptance.json` unterscheidet beide ausdrücklich: Remote-Hosts, Persistenz bei
Netzverlust, Provider-Accounts, Dateiübertragung, visueller Sollvergleich und
explorative Agententests sind `not-run`, bis sie tatsächlich durchgeführt wurden.
Der Ausbau wird in [#562](https://github.com/byte5ai/zaplex/issues/562) verfolgt.
Screenshots werden nicht automatisch zu freigegebenen Designreferenzen erklärt;
die Sollreferenzen stehen in `docs/ui/README.md` und `specs/GH-562/PRODUCT.md`.

Die Report-Logik wird mit `python3 script/macos/gui-tests/report_tests.py` geprüft.
Ein grüner Teiltest, eine geänderte erwartete Fallzahl oder leere Ergebnisse
dürfen keine volle lokale Abnahme liefern. Die Swift-Dateien lassen sich vorab
mit `swiftc -frontend -parse` syntaktisch prüfen; die eigentliche Kompilierung
und Ausführung finden ausschließlich im macOS-Testjob statt.

## Gezielte Fehlerprüfung nach Lauf 39303447063

Vor dem Wiederholungslauf sind genau drei Fälle ausgewählt: Fenster schließen,
Split-Abbruch und Split links. Das Schließen muss die Fensterzahl um genau eins
verringern, auch wenn weitere Fenster wiederhergestellt wurden. Bei den beiden
Terminalfällen muss jeder Befehl nach genau einem Return die erwartete frische
Ausgabe liefern und der ursprüngliche nicht exportierte Shellzustand erhalten
bleiben. Vor Return wird die vollständig angekommene Texteingabe am öffentlichen
AX-Wert geprüft und ein Screenshot gesichert; der Erfolgsnachweis bleibt OCR der
Ausgabe. Feste Ablehnungsgründe aus dem App-Log helfen, eine Shell-Startsperre von
verlorener Eingabe zu unterscheiden. Der alte Lauf belegt diese Ursache noch nicht.

## OCR-Auflösung und gespeicherte Gegenproben

Die Navigationserkennung vergrößert den Fenster-Screenshot im Testprozess auf
jeweils die doppelte Pixelzahl pro Achse mit hochwertiger Interpolation. Die
feste Einstellung ist an den gespeicherten Gegenproben geprüft. Die App-Schriftgröße bleibt unverändert;
die TSV-Positionen werden auf die tatsächliche Fenstergröße zurückgerechnet.
Der Vergleich von Nonce, Shellzustand und Menütext bleibt exakt. Das folgt der
[Tesseract-Empfehlung zur Bildauflösung](https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html#rescaling).

Vor XCTest prüft ein eigenständiger Swift-Test die Originalbilder aus dem
fehlgeschlagenen Lauf 40087073327: korrekte Ausgabe `ZQA 149060 963`, Ablehnung
einer falschen Nonce und eines falschen Zustands sowie Ablehnung des noch nicht
abgeschickten Befehls. Unabhängig gemessene Bildkoordinaten prüfen die Abbildung
bei normaler und halber Punktauflösung. `fixtures/ocr/provenance.json` enthält
Herkunft und Hashes. Weitere Fensterbilder aus Lauf 40125406009 prüfen die zuvor
ausgelassene ausgewählte Dateizeile, die Ziffernverwechslung im schmalen Fenster
und unabhängige Ausgaben in zwei nebeneinanderliegenden Panes.
`fixtures/ocr/provenance-136.json` dokumentiert
Originalhashes und verlustfreie Ausschnitte anhand der AX-Fenstergrenzen.
Die drei unveränderten Vollbildoriginale in `provenance-137.json` ergänzen eine
zusätzliche falsch gelesene Nonce, ihren nicht abgeschickten Befehl und die
Settings-Navigation. `fixture-image.swift` schneidet diese erst beim Laden auf
das dokumentierte AX-Fensterrechteck zu. Positive und negative Textprüfungen
sowie unabhängig gemessene Koordinaten bleiben Teil derselben Gegenprobe.
`ocr-regression.log` archiviert Ergebnis und TSV. Diese
Gegenproben ergänzen die Nutzerabläufe; seit der Mindestgrößenprüfung sind es sechzehn.

`gui-ocr-diagnosis.yml` vergleicht bei abweichenden macOS-Ergebnissen ausschließlich
die gespeicherten Bilder mit der tatsächlichen nativen Implementierung. Der
manuelle Lauf auf `main` startet weder App noch XCTest und gibt keinen
Abnahmenachweis ab. Er archiviert alle TSV-Ergebnisse für feste Kombinationen
aus Maßstab, Interpolation und Segmentierung sowie Engine- und Sprachdatenstand.
Die regulären GUI-Tests verwenden weiterhin genau eine feste Konfiguration;
ein Diagnoseergebnis ersetzt keine bestandene native Gegenprobe.

Der [native Vergleich 40131681579](https://github.com/byte5ai/zaplex/actions/runs/40131681579)
mit Tesseract 5.5.3 reproduziert die Ziffernverwechslung bei dreifacher
Pixelwiederholung. Zweifache Vergrößerung besteht alle sechs Bilder einschließlich
falscher Nonces, falscher Zustände, unbestätigter Eingaben und Koordinaten, bei
beiden Interpolationsarten und beiden Segmentierungsmodi.

Der [erweiterte native Vergleich 40146026138](https://github.com/byte5ai/zaplex/actions/runs/40146026138)
erfasst 144 Lesungen auf neun Bildern. Der bisherige Modus für einen einheitlichen
Textblock (`--psm 6`) reproduziert beide neuen Fehler. Zweifache Vergrößerung mit
Sparse-Text-Segmentierung (`--psm 11`) besteht alle neun Bilder einschließlich
der Gegenproben und Koordinaten. Der damalige Standard kombinierte deshalb zweifache
Pixelwiederholung mit Modus 11. Exakte Textvergleiche bleiben unverändert;
Ergebnisse unterschiedlicher Konfigurationen werden nicht zusammengeführt.

Der [native Vergleich40743849365](https://github.com/byte5ai/zaplex/actions/runs/40743849365)
ergänzt die zwei unveränderten Tab-Setup-Bilder aus1.2.41. Die bisherige zweifache
Pixelwiederholung liest die sichtbare Nonce `490663` dort als `499663`.
Zweifache Vergrößerung mit `CGInterpolationQuality.high` und Sparse-Text-Modus11
besteht die exakten positiven und negativen Textprüfungen aller elf Bilder.
Diese feste Einstellung ersetzt die Pixelwiederholung; es gibt weder
Zeichenersetzungen noch einen vom erwarteten Text gesteuerten OCR-Fallback.
`provenance-141.json` dokumentiert Herkunft, Originalhashes und AX-Ausschnitt;
die Gegenprobe prüft zusätzlich den falschen Shellzustand und die unabhängig
gemessenen Ausgabekoordinaten. Unabgeschickte Eingabe bleibt ausdrücklich negativ.
