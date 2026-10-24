# Agent-Persistenz und Theme-Wechsel auf installiertem 1.2.27

Die [Exploration 40056368953](https://github.com/byte5ai/zaplex/actions/runs/40056368953)
prüft die unveränderte DMG aus [Batch 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378),
App-Quelle `b270ee372f8548b83363916dc8fbddc1ee7b5884`, Harness
`45935546251bbaa7ebc01d9b9ead22c5333941e9`, EN bei 1024×768.
DMG-SHA256: `fcb082266c6b4f61fb8bb0c3d871e9ccafb8c229225e528f288bafb36f800644`.
Die Erwartungen stammen aus der [Abnahmematrix](MATRIX.md). Das
[Manifest](evidence/candidate-127-agent-theme/manifest.json) enthält die Hashes
aller 66 einzeln gesichteten Originalaufnahmen; Remote-Details bleiben im privaten Archiv.

| Ablauf | Ergebnis | Aufnahmen |
|---|---|---|
| Laufender Claude Code bei SSH-Transportabbruch | **Pass für diesen Prozessablauf:** offizieller CLI-Prozess im Vordergrund führt `sleep 240` aus. Eine unabhängige SSH-Beobachtung bestätigt dieselben Shell-, CLI- und Sleep-PIDs vor und nach dem erfolgreichen nativen ControlMaster-Exit. Zaplex verbindet automatisch wieder; CLI endet mit Exit 0 und erwartetem Ergebnis. Nicht exportierter Shellmarker und CWD bleiben erhalten. Kein nohup, disown oder Hintergrundagent. | 007, 025, 028, 030, 032, 034, 036 |
| Codex-Shellwerkzeug | **Umgebungsblockade:** CLI meldet `Creating new namespace failed: Permission denied`. Exit 0 des CLI belegt keinen erfolgreichen Werkzeugaufruf. Kein laufender Codex-Prozess beim Transportabbruch nachgewiesen; Sandbox unverändert. Konkrete Containerursache noch ungeprüft. | 013, 036 |
| Erster Claude-Versuch mit 120 Sekunden | **Keine Aussage zur aktiven Persistenz:** Werkzeug war unmittelbar vor dem Abbruch fertig. Nicht als Erfolg gezählt. | 020, 022, 023 |
| Filterentwurf, Dunkel→Hell, Rückkehr aus Einstellungen | Text, Auswahl und gefiltertes Ergebnis bleiben lesbar erhalten. **Fokusfehler:** Einfügen und rohe Taste erreichen das Feld nach der Rückkehr nicht; nach erneutem Klick funktioniert die Eingabe. [#656](https://github.com/byte5ai/zaplex/issues/656). | 042–043, 048, 050–052, 054 |
| Umbenennungsentwurf, Hell→Dunkel | Entwurf und Auswahl bleiben lesbar erhalten. Kein Fokusurteil: Nach der letzten Taste war das begrenzte Testfenster abgelaufen; keine Folgeaufnahme. Umbenennung nicht bestätigt, Originaldatei unverändert. | 058–059, 064, 066 |

Der unabhängige Ergebnis-Hash des erfolgreichen Claude-Ablaufs ist
`eea268e64ba7075434446c62c1abedd0ae9a4f6feeb97c5a59ec0afa2330a45c`.
Verwendet wurden API-Testprofile: dies ist kein Nachweis echter Abonnements,
Kontowahl oder Kontorouting. Der Kontrollaufruf mit `/usr/bin/ssh` funktionierte;
die vorherige Variante mit bloßem `ssh` wurde von der Shellintegration abgefangen
und abgebrochen. Die isolierte Bash-Klassifikation und die Quellprüfung bestätigen den Fehler; er ist als [#657](https://github.com/byte5ai/zaplex/issues/657) erfasst.

Nicht ausgeführt: Ordnerentwurf beim Theme-Wechsel, deutsche Varianten,
Sprachwechsel und übrige Matrixfälle. Die fehlende Aufnahme 067 wird nicht gezählt.
Das Fenster endete am konfigurierten Zeitlimit; der Workflow bestätigte das
Cleanup der eigenen Prozesse und Testdaten. Es wurde kein Release erstellt.
