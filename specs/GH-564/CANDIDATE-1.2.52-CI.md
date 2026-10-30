# Abgebrochener CI-Stand 1.2.52

[PR #751](https://github.com/byte5ai/zaplex/pull/751) korrigiert das zunächst
leere Repository-Feld aus [#749](https://github.com/byte5ai/zaplex/issues/749).
Quelle von [Batch 41106705216](https://github.com/byte5ai/zaplex/actions/runs/41106705216):
`7d16fee989aea6d4ab8b0727478143f5064fb9bd`, Version 1.2.52.

Bestanden sind Apple-Silicon-Compile, Policy, Abhängigkeitsprüfung, App-Regressionen,
Mac-Paketsuite und echtes SFTP. Die App-Stufe bestätigt die sieben neuen Fälle
jeweils genau einmal; 27 positive Ergebnisblöcke melden 821 bestandene
Testausführungen ohne Fehler oder ignorierte Fälle. Wegen überlappender Filter
ist dies kein disjunkter Gesamtbestand.
[Originalprotokoll-Nachweis](evidence/candidate-152-native/repo-picker-ci.json).

Die Mac-Paketsuite enthält exakt die vor ihrer Ausführung festgehaltenen
4.402 unterschiedlichen Fälle: den vollständigen 1.2.51-Bestand plus sieben neue
Komponententests. Keine Fehler, Skips oder aufgezeichneten Wiederholungsfehler.
Der öffentlich im Issue gespeicherte Hash der Erwartungsdatei ist nachweislich
älter als der Start der Paketstufe.
[JUnit-, Archiv- und Fallnachweis](evidence/candidate-152-native/package-tests.json).

Ein ergänzender realer Komponententest hat vor der nativen Abnahme einen weiteren
Pfad desselben Fehlers bestätigt: Im tatsächlichen Such-Editor einen Text ohne
Treffer eingeben, „Add new repo“ anfordern und anschließend Repository B wählen.
Der erhaltene Suchfilter blendet B aus; die gerenderte Dropdown-Beschriftung
bleibt leer. Der vollständige Fallname führt genau einen Test aus und scheitert
an dieser sichtbaren Beschriftung. Ein anfänglicher kurzer Selektor mit null
Treffern wurde nicht als Prüfung gewertet.

Deshalb wurde der eigene Batch vollständig abgebrochen, bevor ein DMG entstand.
Abschluss ist **cancelled**; die Linux-Daemon-Prüfung wurde abgebrochen. Kein
vollständiger Batch-Pass, keine installierte oder native 1.2.52-Abnahme und kein
Release/Tag. Die bestandenen Nachweise bleiben erhalten.
[Abbruch, einzelne Stufen und Gegenprobe](evidence/candidate-152-native/cancelled-batch.json).
Die Fortsetzung setzt den vorhandenen Suchfilter vor dem Einsetzen der neu
gewählten Repository-Auswahl zurück und wird als 1.2.53 qualifiziert.

Die lokale Workflow-Typprüfung von #751 lief ohne ShellCheck. Der zusätzliche
Standardlauf mit ShellCheck enthält neun Befunde in unveränderten
Release-Skripten, exakt identisch zur Basis; ein vollständiger ShellCheck-Pass
wird nicht behauptet.
[Präzisierter lokaler Prüfumfang](evidence/repo-picker-749-local.json).
