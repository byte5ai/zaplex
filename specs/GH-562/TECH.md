# Technischer Aufbau

## Bestand und Erweiterung

`script/macos/gui-tests` steuert die installierte App mit XCUITest. Native
Menüeinträge und Tastaturbefehle sind zugänglich; der eigene Renderer stellt
seine Bedienelemente überwiegend nicht einzeln im Accessibility-Baum bereit.
Tesseract liest deshalb die tatsächlichen Screenshots. Es ist nur Testwerkzeug.

Gemeinsame XCTest-Hilfen starten pro Fall die App, öffnen für Navigation eine
eigene Arbeitsfläche und halten Screenshots/AX-Baum fest. Nicht exportierte
Shellvariablen prüfen Isolation und Sitzungsfortbestand. Ausgabe enthält einen
pro Prüfung neuen numerischen Marker, der nur zusammen mit dem korrekten
Shellzustand den erwarteten Text ergibt. TSV-OCR liefert Positionen relativ
zum fotografierten Fenster; Klicks werden aus frischen Aufnahmen abgeleitet.

Die Suites `terminal`, `navigation` und `surfaces` dienen der gezielten Diagnose. `full`
führt alle Fälle aus. Der Aufrufer eines wiederverwendbaren Workflows kann die
Suite nicht reduzieren. Die Zahl ausgeführter Fälle und die Fehler-/Skipzahlen
werden zusätzlich zum Exitcode geprüft.
`only_tests` erlaubt ausschließlich bekannte, eindeutige Testnamen für einen
Diagnoselauf. Damit werden nach einem Fund nur betroffene Fälle erneut ausgeführt.
Die vollständige Suite verbietet eine solche Teilmenge.

Die schmale Ansicht wird über das native Fenstermenü hergestellt und anhand der
tatsächlichen Fensterbreite geprüft. Der erste Versuch, den rechten Rand des
bildschirmfüllenden Fensters mit XCTest zu ziehen, änderte dessen Größe nicht;
dieser Versuch belegt noch keinen App-Resize-Fehler. Die Layout-Erwartung bleibt
unverändert: höchstens 700 Punkte Breite, alle acht beschrifteten Dateiaktionen
innerhalb des sichtbaren Fensters und Rückkehr zur bestehenden Shell.

## Veröffentlichung und Belege

`zap_release.yml` ruft die GUI-Prüfung nach `release_macos` mit `zap-macos-arm64`
auf. Testquellen werden auf die unveränderliche `release_sha` festgelegt.
`publish_release` hängt zusätzlich von der GUI-Prüfung ab. Der Download für die
Veröffentlichung wählt nur `zap-*`; `release-body` wird getrennt geladen.
Die anschließende Bereinigung entfernt nur diese Zwischenartefakte und erhält
`macos-gui-evidence-*` gemäß deren Aufbewahrungsfrist.

Ein maschinenlesbarer Bericht unterscheidet lokale Ausführung und vollständige
Produktabnahme; Remote-, visuelle und explorative Prüfungen bleiben bis zur
echten Durchführung `not-run`. Es gibt keine automatisch erzeugten Pass-Belege
für diese Bereiche. Infrastrukturbedarf wird im zuständigen privaten Infra-Repo
verfolgt, ohne private Endpunkte oder Konten in diesem öffentlichen Repo.

## Validierung

- Swift-Parser und Bash-Syntax lokal; XCTest-Kompilierung und echte GUI auf macOS.
- Neue Navigationsfälle zuerst gezielt am vorhandenen DMG, ohne App-Neubau.
- Negative Validatorfälle: leere/fehlende/ausgelassene Tests, falsche Fallzahl,
  Testfehler und Diagnoseteilmengen dürfen keine volle Abnahme liefern.
- Workflow-Struktur prüfen: GUI-Abhängigkeit, unveränderliche Quellrevision,
  getrennte Belegartefakte und keine Aufweichung durch `continue-on-error`.
- Eine Korrektur in `main` ist erst mit einem DMG dieses Standes nachgewiesen.
