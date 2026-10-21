# Native Remote-Abnahme

Der manuelle Workflow `gui-explore.yml` bereitet mit `remote_target=true` den
isolierten Linux-Testendpunkt vor. Er verwendet den bestehenden DMG und dessen
passenden Linux-Daemon; kein erneuter App- oder Daemon-Build. Der SSH-Alias
`zaplex-test` lässt sich über „Add a host“ aus der SSH-Konfiguration importieren.

## Kriterien vor dem Lauf

1. Der übertragene Daemon stimmt per SHA-256 mit dem DMG-Bundle überein.
2. Der importierte Host öffnet im persistenten Modus eine echte Remote-Shell.
   Ein dort gesetzter Marker und die Shell-PID werden vor dem Trennen festgehalten.
3. Nach Trennen und Wiederverbinden bleiben Marker, Arbeitsverzeichnis und
   Shell-PID erhalten. Ein neu gestarteter Shellprozess erfüllt dieses Kriterium nicht.
4. Claude Code und Codex antworten auf je eine harmlose, begrenzte Anfrage in
   der Zaplex-Remote-Sitzung. API-Testprofile belegen weder Abo-Anzeige noch
   Kontingentfenster oder Abo-Routing.
5. Originalaufnahmen vor und nach den Aktionen zeigen den tatsächlichen Zustand.
   Ein erfolgreicher Infrastrukturjob allein gilt nicht als Produktabnahme.
6. Cleanup beendet ausschließlich Prozesse dieses Laufs und verschiebt seine
   Daten in den Papierkorb des isolierten Testbenutzers. Vorhandene Zaplex-Daten
   oder ein abweichender Eigentümer führen zum Abbruch statt zum Überschreiben.

## Belege und Grenzen

Remote-Modus lädt ausschließlich konstruierte Metadaten und Cleanup-Fakten
hoch. Rohe Screenshots, SSH-Konfiguration, Schlüssel und Logs sind kein
öffentliches CI-Artefakt. Originalaufnahmen werden über den freigegebenen
GUI-Bedienzugang privat abgeholt und vor einer Veröffentlichung geprüft.
Der Zugang endet nach zehn Minuten; der Job führt auch bei Fehlern Cleanup aus.
Die Zielbelegung ist serialisiert. Ein hart verlorener Runner kann keinen
Cleanup garantieren; der nächste Lauf verweigert dessen vorhandene Daten.

Die lokalen Ownership-Prüfungen laufen auf Linux mit
`python3 script/macos/gui-tests/remote-target-tests`.
