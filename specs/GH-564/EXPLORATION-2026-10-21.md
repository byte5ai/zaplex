# Explorative native Teilprüfung am 21.10.2026

Die erste direkte Screenshot-/Klick-/Tastatursitzung ist durchgeführt.
[Run 39578485133](https://github.com/byte5ai/zaplex/actions/runs/39578485133)
endete erfolgreich, einschließlich Aufräumen und Beleg-Upload. Das bedeutet
Bereitstellung und sauberes Ende der Sitzung, **keine Produktfreigabe**.

Prüfgegenstand: unverändertes installiertes DMG 1.2.2 aus Run 39295080993,
App `cadcb19130bbe0630f39bdd61b4e183ff154981a`, SHA-256
`313303cc2dd14b820cd9c01bb931b555300864914b7fc746f4c6841e4889bae3`.
Harness `a00e12b1d7040e20e0cfbe91e9a175b25f791046`, Designreferenz
`6c956383ad560913dc31fa383104dc489104d561`. Englisch; zuerst Zaplex Dark,
danach Light. Fenster 1024×678 Punkte bei (0,30), Aufnahmen 1024×678 Pixel.
Reduced Motion nicht geprüft. Kein erneuter App-/Daemon-Build oder Suite-Lauf.
Die späteren Korrekturen aus #586/#589 sind nicht in diesem DMG enthalten.

[Manifest](evidence/exploration-39578485133/manifest.json) enthält Herkunft,
SHA-256, Snapshot und gemessene Fenstergeometrie aller **23 unveränderten,
einzeln angesehenen Originalbilder**. [Aktionsprotokoll](evidence/exploration-39578485133/actions.json)
hält die tatsächlich ausgeführten Eingaben fest. Die nächste Aktion wurde anhand
der jeweils neuen Ansicht gewählt; dies war keine vorher festgelegte Klickfolge.

## Aufgaben und unabhängige Erwartungen

Vor Beginn standen C02/X01 (Host-Anlage finden, ungültige Eingabe erklären,
Abbruch ohne zusätzliche Anlage) und E01/X05 (Einstellung sichtbar ändern und
zum vorhandenen Arbeitskontext zurückkehren) fest. Die Anforderungen stammen
von #564/MATRIX.md; sichtbare Ist-Zustände wurden nicht zum Soll erklärt.

## Beobachteter Verlauf

| Bilder | Aktion und sichtbare Wirkung | Bewertung |
|---|---|---|
| [01](evidence/exploration-39578485133/01-start.png) → [02](evidence/exploration-39578485133/02-connections.png) → [03](evidence/exploration-39578485133/03-connections-foreground.png) | Der erste Hintergrundklick hatte keine sichtbare Wirkung. Nach erneuter Beobachtung wurde Vordergrundzustellung gewählt; das Server-Icon öffnet die Verbindungsliste. | Bedienzugang nachgewiesen; kein App-Fehler aus dem wirkungslosen Hintergrundklick abgeleitet. Die abgeschnittene Leeranleitung ist der bereits behobene Altbefund #565. |
| [04](evidence/exploration-39578485133/04-add-host.png) → [05](evidence/exploration-39578485133/05-host-form.png) | Plus öffnet „Add a host“. „Create a blank server“ erzeugt sichtbar einen Eintrag und öffnet dessen Formular neben der bestehenden Shell. „Save“ wird am rechten Formularrand abgeschnitten. | Host-Anlage auffindbar; neuer Breitenbefund D04. Das explizite „Create“ ist keine erst beim späteren Save erfolgende Anlage. |
| [06](evidence/exploration-39578485133/06-host-invalid.png) → [07](evidence/exploration-39578485133/07-host-save-invalid.png) → [08](evidence/exploration-39578485133/08-host-escape.png) | Eingabe `bad host name` ist sichtbar. Save zeigt „Host cannot be empty.“ Escape verlässt dieses dauerhafte Formular nicht. | Ungültiger Host wird abgewiesen, Erklärung ist falsch: D05. Kein erfolgreicher Save und kein Formularabbruch behauptet. |
| [09](evidence/exploration-39578485133/09-settings-menu.png) → [10](evidence/exploration-39578485133/10-appearance.png) → [11](evidence/exploration-39578485133/11-theme-choice.png) → [12](evidence/exploration-39578485133/12-theme-light-search.png) | Zahnrad öffnet einen Settings-Tab. Appearance → aktuelle Theme-Karte öffnet die Auswahl; Suchtext „Light“ zeigt passende Themes. | Einstellung und Auswahl aus sichtbaren Beschriftungen auffindbar. |
| [13](evidence/exploration-39578485133/13-theme-light-applied.png) → [14](evidence/exploration-39578485133/14-theme-escape.png) → [15](evidence/exploration-39578485133/15-theme-panel-closed.png) | Auswahl Light stellt die App sichtbar um; Escape schließt die Theme-Seitenleiste. Ein anschließender überflüssiger Klick an deren früherer X-Position fokussiert die Settings-Suche. | Theme-Wechsel und Escape-Rückweg bestanden. Der zusätzliche Klick ist kein Beleg für das Schließen. |
| [16](evidence/exploration-39578485133/16-return-workspace.png) | Klick auf den vorherigen SSH-Tab stellt Hostformular und Shellpane wieder dar. Der Fehlertext bleibt, aber die Eingabewerte sind nun fast weiß auf Hellgrau. | Arbeitsansichten erhalten; keine unabhängige Prüfung eines Shellvariablenwerts. Neuer Kontrastbefund D06. |
| [17](evidence/exploration-39578485133/17-add-again.png) → [18](evidence/exploration-39578485133/18-add-cancel.png) | Plus öffnet erneut die Host-Auswahl; Cancel schließt sie. Weiterhin genau ein sichtbarer Servereintrag, keine zusätzliche Pane. | Abbruch der Auswahl bestanden; nicht mit einem Abbruch des bereits angelegten Servers gleichsetzen. |
| [19](evidence/exploration-39578485133/19-add-for-light-check.png) → [20](evidence/exploration-39578485133/20-fresh-host-light.png) | Zur Eingrenzung wird ein zweites leeres Formular unter Light geöffnet. Neue Feldwerte sind dunkel/lesbar, das ältere Formular darunter bleibt sehr hell. | Stützt einen Fehler beim Aktualisieren bestehender Editorfarben nach Theme-Wechsel, keine allgemeine Unlesbarkeit frisch geöffneter Light-Formulare. |
| [21](evidence/exploration-39578485133/21-valid-host-focus.png) → [22](evidence/exploration-39578485133/22-valid-host-typed.png) → [23](evidence/exploration-39578485133/23-valid-host-save.png) | `example.invalid` wird eingegeben und Save angeklickt. Die letzte Ansicht zeigt weder eine gesicherte Speicherbestätigung noch einen bewiesenen Reload. | Gültiges Speichern bleibt ungeprüft. Kein Netzwerk-/Providerkontakt und kein Erfolg aus dem Klick allein abgeleitet. |

Peekaboo meldete teilweise Exit 1 beim Tippen, obwohl die nachfolgende Aufnahme
den vollständigen Text zeigte. Maßgeblich ist diese sichtbare Wirkung; es wurde
kein Text blind erneut abgeschickt. Diese Werkzeugmeldungen sind keine Appfehler.

## Neue Korrekturaufträge

- **D04 / P2 / [#593](https://github.com/byte5ai/zaplex/issues/593):** Formularheader muss Test, Connect und Save bei der tatsächlich
  durch „Create a blank server“ erzeugten schmalen Pane vollständig darstellen.
  `server_view.rs` baut Titel und drei Buttons in nicht umbrechenden Flex-Zeilen.
- **D05 / P2 / [#594](https://github.com/byte5ai/zaplex/issues/594):** Nichtleere ungültige Hosts brauchen eine passende Erklärung.
  `endpoint_validation_message` führt `EmptyHost` und `InvalidHost` auf denselben
  Text „Host cannot be empty.“ zusammen. Die Validierung selbst wurde nicht umgangen.
- **D06 / P1 / [#595](https://github.com/byte5ai/zaplex/issues/595):** Bestehende Editorfelder müssen beim Theme-Wechsel ihre Textfarben
  aktualisieren. `make_editor` übernimmt `text_colors_override` beim Erzeugen;
  der Vergleich von altem und neuem Formular ist der native Befund, die genaue
  Änderung im Editor-/Appearance-Pfad bleibt Implementierungsarbeit.

Die betroffenen Dateien `app/src/ssh_manager/server_view.rs` und `mod.rs` sind
zwischen App-Commit und dem am Prüfdatum live abgeglichenen `main` (`a00e12b1`)
unverändert. Das ist ein Codeabgleich, kein nativer Test eines neuen DMGs.

## Reichweite und nächster Prüfbedarf

C02 hat nun einen konkreten Fehler statt eines pauschalen „nicht geprüft“.
E01/X01/X05 haben Teilbelege; insbesondere Theme-Wechsel, Rückkehr zu denselben
Ansichten und Cancel der Host-Auswahl sind beobachtet. Vollständige Sprach-,
Mindestbreiten-, Tastatur-, Formular-, Shellzustands- und Fehlervarianten bleiben
ungeprüft. Es gab keine Remoteverbindung, echte Provideranmeldung oder vollständige
Kontrollinventur. #564/#562 erhalten dadurch keine Gesamtfreigabe.

Der lokale interaktive Zugang ist jetzt nachgewiesen. Der frühere pauschale Grund
„GUI-Zugang fehlt“ gilt nicht mehr; weitere lokale Aufgaben sind ausführbar.
Remote-Endpunkt und autorisierte Providerprofile bleiben separate Voraussetzungen.
