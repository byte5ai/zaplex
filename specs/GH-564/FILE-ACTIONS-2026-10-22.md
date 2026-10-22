# Native Dateiaktionen am 22.10.2026

[Run 39801240596](https://github.com/byte5ai/zaplex/actions/runs/39801240596)
prüft das unveränderte installierte **DMG 1.2.2**, App-Commit
`cadcb19130bbe0630f39bdd61b4e183ff154981a`, SHA-256
`313303cc2dd14b820cd9c01bb931b555300864914b7fc746f4c6841e4889bae3`.
Harness: `7c4af4fb13476c85936ddc9cb0bfb16e47bb14c3` (1.2.16).
Designreferenz: `6c956383ad560913dc31fa383104dc489104d561`, MATRIX.md.
EN, Dark, Fenster 1024×678 Punkte bei (0,30), Originalbilder 1024×678 Pixel.
Andere Sprachen, Themes, Mindestbreiten und Reduced Motion sind hier nicht geprüft.

Das auf 30 Minuten begrenzte Bedienfenster wurde nach den Aufgaben durch den
vorhandenen Abschlussmarker vorzeitig beendet. Cleanup und Artefakt-Upload
bestanden; `finished` ist vorhanden, `session-expired.txt` fehlt. Der grüne Job
belegt die Bereitstellung, nicht eine pauschal bestandene Produktabnahme.

## Originalbelege und Ausführung

[Manifest](evidence/exploration-39801240596/manifest.json),
[Aktionsprotokoll](evidence/exploration-39801240596/actions.json),
[vorab festgelegte Aufgaben und ausdrücklich dokumentierte Diagnoseergänzungen](evidence/exploration-39801240596/criteria.json).
Alle **43 Bilder wurden einzeln angesehen**, unverändert kopiert und bytegenau
gegen das GitHub-Artefakt geprüft. Das Manifest enthält SHA-256 und die jeweils
beobachtete Fenstergeometrie. Die historischen Dateinamen beschreiben das Ziel
der Aufnahme; insbesondere 07, 17 und 18 belegen keinen Erfolg dieses Ziels.
`type` meldete teils einen unbestimmten Werkzeugfehler trotz vollständig sichtbarer
Eingabe. Jeder weitere Schritt beruhte auf einer frischen Aufnahme; keine blinden
Wiederholungen der Texteingabe. Die ursprünglichen Kriterien bleiben als Protokoll
mit ihrem anfänglichen `not-run` erhalten; die tatsächlichen Ergebnisse stehen hier.

| Bilder | Beobachtete Handlung und Wirkung | Bewertung |
|---|---|---|
| 01–04 | Frische Shell erzeugt `ORIGINAL.txt` mit `ALPHA` sowie `nested/CHILD.txt` mit `BETA`. Nicht exportierter Marker `1648`; Cmd+Shift+E zeigt das richtige Verzeichnis. | Isolierte Ausgangsdaten, lokaler Einstieg bestätigt. |
| [05](evidence/exploration-39801240596/05-rename-dialog.png) → [06](evidence/exploration-39801240596/06-rename-input.png) → [07](evidence/exploration-39801240596/07-rename-confirmed.png) | F2 für `ORIGINAL.txt`, Eingabe `RENAMED.txt`, Klick Confirm. Toast: „Rename failed: The local file could not be read or written.“ Original bleibt sichtbar. | **F08 fail über `/tmp`**, #614, P1. |
| 08–10, [23](evidence/exploration-39801240596/23-shell-proof.png) | Nach dem fehlgeschlagenen Rename wird der unabhängige Abbruch an der unveränderten Originaldatei geprüft: `CANCELED.txt` eingeben, Cancel klicken. Original bleibt ausgewählt; Shell bestätigt unveränderten Inhalt und fehlenden Zielnamen. | F2-Abbruch in dieser Konfiguration pass. Die geänderte Ausgangsdatei des Abbruchfalls ist ausdrücklich im Kriterienprotokoll festgehalten; der Rename-Fehler bleibt fail. |
| [11](evidence/exploration-39801240596/11-create-dialog.png) → [12](evidence/exploration-39801240596/12-create-input.png) → [13](evidence/exploration-39801240596/13-folder-created.png) | F7 legt `CREATED` an. Die Liste zeigt den Ordner; Shell bestätigt ihn als Verzeichnis. Die Cursorzeile springt dabei von `ORIGINAL.txt` auf das unbeteiligte `nested`. | Ordneranlage pass; **Cursorbefund #615, P2**. Ein beibehaltener Index ist eine Hypothese, keine nachgewiesene Ursache. |
| 14–16, 23 | F7 mit `NOT-CREATED`, Escape schließt Dialog; Name fehlt anschließend auch im Dateisystem. | Ordnerabbruch pass. |
| [16](evidence/exploration-39801240596/16-folder-canceled.png) → [17](evidence/exploration-39801240596/17-nested-directory.png) → [18](evidence/exploration-39801240596/18-nested-after-focus.png) | Nach Escape öffnet Enter den ausgewählten Ordner nicht; einfacher Zeilenklick plus Enter ändert ebenfalls nichts sichtbar. | **Tastaturbefund #616, P2**. Werkzeugzustellung angenommen; genaue Fokus-/Ereignisursache noch unbewiesen. |
| [19](evidence/exploration-39801240596/19-nested-double-click.png) → [20](evidence/exploration-39801240596/20-parent-return.png) | Doppelklick öffnet `nested` mit `CHILD.txt`; der sichtbare Hoch-Button führt zurück und zeigt `nested` ausgewählt. | F04 Teilnachweis pass für diese kurze Liste und den Mausweg; Scroll-, Sortier- und weitere Tastaturvarianten bleiben ausstehend. |
| 21–23 | F10 zurück zur Shell. `AFTER:1648`, ursprüngliches CWD, `ALPHA`, Dateirechte und Verzeichnis `CREATED` sichtbar. `RENAMED.txt`, `CANCELED.txt` und `NOT-CREATED` fehlen. | Shellzustand und die bis hier erfolgten/abgebrochenen Dateieingriffe unabhängig geprüft. |
| [24](evidence/exploration-39801240596/24-canonical-input.png)–[32](evidence/exploration-39801240596/32-canonical-proof.png) | Diagnose: Nur `cd -P .` ändert die Pfadschreibweise zu `/private/tmp/...`. Dieselbe Datei und dieselben Rechte; F2 mit `RENAMED.txt` gelingt jetzt. Shell zeigt den neuen Namen, `ALPHA`, erhaltenen Marker; Originalname fehlt. | Kanonische Gegenprobe pass. Belegt die Relevanz der Pfadvariante; **keine Korrektur des Fehlers über `/tmp`**. |
| [33](evidence/exploration-39801240596/33-view-manager.png) → [34](evidence/exploration-39801240596/34-file-view.png) → [35](evidence/exploration-39801240596/35-file-view-closed.png) | F3 „View“ öffnet „File Details“ mit Metadaten, ohne Dateiinhalt. Escape kehrt mit gleicher Auswahl zurück. | Dialog/Rückweg beobachtet; Nutzeraufgabe „Inhalt ansehen“ über F3 nicht erfüllt. **Benennungsbefund #617, P3**; die Referenz fordert hier keinen Inhaltsviewer ausdrücklich. |
| [36](evidence/exploration-39801240596/36-file-edit.png)–[42](evidence/exploration-39801240596/42-editor-proof.png) | F4 öffnet einen Editor-Tab mit `ALPHA`. Änderung zu `GAMMA`, Cmd+S zeigt „File saved“; Cmd+W schließt nur den Editor. Zurück im Dateimanager bleibt `RENAMED.txt` ausgewählt. F10 und Shellausgabe bestätigen `GAMMA` sowie Marker `1648`. | F4 Bearbeiten/Speichern/Rückkehr pass für diese lokale Textdatei über kanonischen Pfad. |
| 43 | Abschlussmarker vollständig in der Eingabe gesehen, danach Return zugestellt; Workflow beendet das Fenster und räumt auf. | Vorzeitiger Abschluss praktisch bestätigt. |

## Einordnung und Korrekturaufträge

[#614](https://github.com/byte5ai/zaplex/issues/614) ist ein blockierender Fehler
für den geprüften normalen Dateipfad. Die unveränderte Backend-Implementierung
verwendet beim geschützten Rename einen neuen Platzhalter und lehnt dabei
Symlink-Elternkomponenten ab. Auf macOS ist `/tmp` ein Alias für `/private/tmp`.
Eine Korrektur muss diese zulässige Nutzung ermöglichen und dabei die bestehenden
Root-, Objektidentitäts-, No-Replace- und Race-Garantien bewahren.

[#615](https://github.com/byte5ai/zaplex/issues/615),
[#616](https://github.com/byte5ai/zaplex/issues/616) und
[#617](https://github.com/byte5ai/zaplex/issues/617) enthalten eigene Soll/Ist-
Beschreibung, Reproduktion, Schweregrad und konkrete Regressionstestaufträge.
Die Cursor- und Tastaturursachen sind noch nicht bewiesen. Für F3 wird keine
zusätzliche Inhaltsviewer-Pflicht aus der Designreferenz erfunden; die Beschriftung
soll die tatsächliche Metadatenaktion verständlich benennen.

Der Lauf ersetzt keine Prüfung eines neu gebauten main-DMG. Mehrfachmarkierungen,
Transfers, Konflikte, F8-Löschen, Remote-Dateiaktionen und die übrigen Matrixvarianten
wurden hier nicht ausgeführt. #564/#562 erhalten keine Gesamtfreigabe.

## Nachprüfung der Korrekturen

#614–#617 sind durch PR #620/#621/#623 korrigiert. Der [native Nachweis](NATIVE-ACTIONS-FONT-2026-10-22.md) auf 1.2.19 besteht in DE/EN mit 18 einzeln geprüften Originalbildern. Die obigen Fehlerbeobachtungen beziehen sich auf das unveränderte DMG 1.2.2.
