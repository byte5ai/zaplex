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

Die erwartete Ausgabe steht nie vollständig im eingegebenen Befehl. Tesseract
liest die gerenderten Pixel; bloßes Echo der Eingabe oder ein interner Modellzustand
reicht nicht. XCTest-Anhänge enthalten Screenshots und den Accessibility-Baum.
Alle fünfzehn Fälle müssen ausgeführt werden und bestehen; übersprungene Tests,
Zeitüberschreitungen und fehlende Ergebnisse lassen den Workflow scheitern.

## Ausführung

- Manuell: `gui-acceptance.yml` mit `release_tag` eines bestehenden Releases.
  Asset-ID und SHA-256 aus der GitHub-API werden vor der Installation geprüft.
  `suite=terminal` prüft gezielt die drei Terminalfälle ohne den Schließen-Button;
  dieser Diagnoselauf gilt ausdrücklich nicht als vollständige Abnahme.
  `suite=navigation` prüft nur die acht neuen Navigationsfälle. Shellzustände
  werden durch nicht exportierte Variablen und pro Prüfung neue Ausgabemarker
  geprüft. TSV-OCR liefert die Positionen der sichtbaren Ausgabe und Menüpunkte.
  `suite=surfaces` prüft Dateimanager und Connections. `only_tests` begrenzt
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
