# Prüfstand 1.2.54

Quelle `28db964d5839a06089040fecb85166fad366448a`, Version 1.2.54.
[PR #754](https://github.com/byte5ai/zaplex/pull/754) erweitert die echte
Mausfläche der Repository-Hinzufügen-Zeile auf die konfigurierte Pickerbreite.
[Batch 41143783370](https://github.com/byte5ai/zaplex/actions/runs/41143783370)
ist vollständig erfolgreich: 16 ausgewählte Stufen bestanden; die beiden nicht
gewählten optionalen Jobs sind separat als übersprungen ausgewiesen.
[Stufennachweis](evidence/candidate-154-native/completed-batch.json).

Die frühe App-Stufe enthält die neun eingefrorenen Repository-/Modalfälle jeweils
genau einmal. 27 positive Ergebnisblöcke melden 823 bestandene Ausführungen,
keine Fehler oder ignorierten Fälle. Überlappende Filter sind kein disjunkter
Gesamtbestand. Die Mac-Paketsuite enthält exakt die 4.404 vorher festgelegten
unterschiedlichen Fallkennungen, ohne Fehler, Skips oder aufgezeichnete
Wiederholungsfehler.
[App](evidence/candidate-154-native/repo-picker-ci.json),
[JUnit-/Archivvergleich](evidence/candidate-154-native/package-tests.json).

Die eingefrorene Paketliste enthält drei aus der älteren 1.2.52-Baseline
mitkopierte Provenienz-Anmerkungen. Tatsächliche Baseline ist der erfolgreiche
1.2.53-Lauf 41119368489, Quelle `3fe124f00d7615398309753415639465d42b1450`,
Paketjob 124300892055. Der zusätzliche direkte Vergleich beider Original-JUnit-
Dateien bestätigt genau 4.403 alte Fälle plus einen neuen Maustest. Aktuelle
Quelle, Version, Fallzahl und sämtliche erwarteten Kennungen waren korrekt und
wurden nach den Ergebnissen nicht verändert. Die vier Erwartungsdateien sind
bytegleich zum vorab veröffentlichten Hash erhalten; ausschließlich die drei
Anmerkungen werden im getrennten
[Erratum](evidence/candidate-154-native/frozen-manifest-metadata-erratum.json)
korrigiert.

Alle 16 vorab eingefrorenen GUI-Kennungen des unveränderten installierten DMG
sind unabhängig exakt verglichen: bestanden, keine Fehler, Skips oder erwarteten
Fehler. App-/Testquelle, installierte Version, Originalarchive und tatsächlicher
DMG-Hash passen. Der öffentliche Erwartungshash ist älter als der GUI-Jobstart.
DMG-SHA `259cd20b19f163898e809ca17ebc25ac33fedf86602f96df69b093983e456da2`.
[Installations-/GUI-Nachweis](evidence/candidate-154-native/installed-gui.json).

Die zusätzliche [reale Sitzung 41160181833](https://github.com/byte5ai/zaplex/actions/runs/41160181833)
prüft Englisch/dunkel bei 1024×678. Das aktuelle Repository ist sichtbar.
Nach echter Suche ohne Treffer färbt Hover die gesamte Hinzufügen-Zeile; Klicks
rechts und mittig öffnen je einen echten nativen Ordnerdialog. Escape im ersten
Dialog erhält die ursprüngliche Auswahl. Im zweiten Dialog wird das vorhandene
lange Git-Unterverzeichnis über die echte Ordnernavigation gewählt; es erscheint
trotz ehemaligem Suchfilter im geschlossenen Wähler, der Tooltip zeigt den
vollständigen gewählten Pfad. Damit sind die hier geprüften Auswahl-/Klickpfade
von #749/#753 bestätigt.

Nach Modalabbruch erreicht unassistierte Texteingabe jedoch das sichtbare
Ausgangsterminal nicht. Bild 018 nach dem Typen ist bytegleich zum leeren
Terminal in 017; die Kontrolle hat Return zurückgehalten. Erst nach explizitem
Promptklick liefert frische Ausgabe PID 1457, unveränderten nicht exportierten
Marker und dasselbe Arbeitsverzeichnis. Archivierte Git-Schnappschüsse sind
vorher/nachher bytegleich: sauberer Status, gleicher HEAD und genau ein Worktree.
Das ist unterstützter Backend-Erhalt, kein bestandener automatischer Fokus-Rückweg.
Der neue Fehler ist [#755](https://github.com/byte5ai/zaplex/issues/755).

19 Originalbilder wurden tatsächlich angesehen; alle 22 lokalen Bilder sind
bytegleich gegen das Workflow-Artefakt geprüft. Sitzung ausdrücklich beendet,
eigene VM beendet; 378,296528 Sekunden liegen unter dem vorab festgelegten
480-Sekunden-Budget. Worktree-Erstellung sowie weitere Sprachen-/Themen-/
Schmalvarianten sind in dieser Sitzung nicht ausgeführt. Sie folgen mit der
Korrektur des Fokusfehlers.
[Native Auswahl- und Fokusnachweise](evidence/candidate-154-native/repository-selection-and-focus.json).

Kein nativer Gesamtpass der Produktmatrix, kein Release/Tag.
