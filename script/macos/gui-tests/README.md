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

Die erwartete Ausgabe steht nie vollständig im eingegebenen Befehl. Tesseract
liest die gerenderten Pixel; bloßes Echo der Eingabe oder ein interner Modellzustand
reicht nicht. XCTest-Anhänge enthalten Screenshots und den Accessibility-Baum.
Alle vier Fälle müssen ausgeführt werden und bestehen; übersprungene Tests,
Zeitüberschreitungen und fehlende Ergebnisse lassen den Workflow scheitern.

## Ausführung

- Manuell: `gui-acceptance.yml` mit `release_tag` eines bestehenden Releases.
  Asset-ID und SHA-256 aus der GitHub-API werden vor der Installation geprüft.
  `suite=terminal` prüft gezielt die drei Terminalfälle ohne den Schließen-Button;
  dieser Diagnoselauf gilt ausdrücklich nicht als vollständige Abnahme.
- Im Batch: `test-dmg.yml` ruft den Workflow nach dem Verpacken auf und übergibt
  genau dessen DMG-Artefakt. Fehler lassen den Test-DMG-Workflow scheitern.
  Im Batch wird immer die vollständige Suite ausgeführt.
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

Dies ist ein erster Funktionstest, keine vollständige Produktabnahme. Remote-Hosts,
Persistenz bei Netzverlust, Provider-Accounts, Dateiübertragung, Tab-/Pane-Abläufe,
visuelle Sollbilder und explorative Agententests sind noch nicht abgedeckt.
Die Release-Pipeline wird hier noch nicht umgestellt; zunächst wird die tatsächliche
Steuerbarkeit der App mit diesem Pilot geprüft. Screenshots werden als Belege
gespeichert und nicht automatisch zu freigegebenen Designreferenzen erklärt.
