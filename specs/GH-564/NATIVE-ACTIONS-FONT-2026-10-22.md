# Dateiaktionen und Schriftgröße: native Nachprüfung

Die gezielten macOS-Läufe bestehen in Deutsch und Englisch, Zaplex Dark,
1024×640 Punkten. Alle **30 unveränderten Originalbilder** wurden einzeln
visuell geprüft; SHA-256 und Schrittprotokolle stehen in den Manifesten.
Diese Integrations-Fixtures bestätigen die genannten Korrekturen; sie sind
keine vollständige Abnahme eines neu installierten DMG.

| Lauf | Quellstand / Version | Umfang | Ergebnis |
|---|---|---|---|
| [39822990130](https://github.com/byte5ai/zaplex/actions/runs/39822990130) | `3a770404cb2e5e8fbd0b3e10d541b26acb753ef6`, 1.2.19 | DE/EN, je 27 Schritte, 18 Bilder | #614–#617 nativ bestätigt |
| [39828670969](https://github.com/byte5ai/zaplex/actions/runs/39828670969) | `15d6cc582f77fdbabbc9f8488433c04980194f4b`, 1.2.20 | DE/EN, je 24 Schritte, 12 Bilder | #622 nativ bestätigt |

## Dateiaktionen

Vor Ausführung festgelegte Erwartungen: zulässiger Alias-Pfad beim Rename;
Cursor bleibt nach F7 am ursprünglichen Objekt; Escape aus F7 gibt den
Tastaturfokus zurück; F3 benennt die tatsächliche Metadatenaktion; F10 erhält
den nicht exportierten Shellmarker. Die lokalen Ergebnisse und die Fix-PRs
#620/#621/#623 lagen vor dem macOS-Lauf vor.

Die sichtbare Auswahl bleibt nach Anlage von `CREATED` auf `ORIGINAL.txt`.
Nach Abbruch von `CANCELED` öffnen Down/Enter ohne erneuten Mausklick `nested`.
F2 benennt durch `native-alias` nach `RENAMED.txt` um. F3 heißt in beiden
Sprachen „Details“, mit lokalisiertem Metadaten-Tooltip; der Dialog zeigt 5 B.
Die abschließende Shellprüfung bestätigt `ALPHA`, erhaltenen Marker 2111,
vorhandenes `CREATED`, fehlendes `CANCELED` und fehlenden Originalnamen.
`FILE-ACTIONS-PASS` ist in beiden Aufnahmen sichtbar.

[Manifest und sämtliche Originale](evidence/native-file-actions-39822990130/manifest.json).

| Zustand | Englisch | Deutsch |
|---|---|---|
| Ausgangsauswahl | [EN](evidence/native-file-actions-39822990130/en/file-actions-selected.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-selected.png) |
| F7 erhält Cursor | [EN](evidence/native-file-actions-39822990130/en/file-actions-created-cursor.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-created-cursor.png) |
| F7 vor Escape | [EN](evidence/native-file-actions-39822990130/en/file-actions-cancel-before.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-cancel-before.png) |
| Tastatur nach Escape | [EN](evidence/native-file-actions-39822990130/en/file-actions-keyboard-after-cancel.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-keyboard-after-cancel.png) |
| F2 Eingabe | [EN](evidence/native-file-actions-39822990130/en/file-actions-rename-before.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-rename-before.png) |
| F2 Ergebnis | [EN](evidence/native-file-actions-39822990130/en/file-actions-rename-after.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-rename-after.png) |
| F3 Tooltip | [EN](evidence/native-file-actions-39822990130/en/file-actions-details-tooltip.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-details-tooltip.png) |
| F3 Dialog | [EN](evidence/native-file-actions-39822990130/en/file-actions-details-dialog.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-details-dialog.png) |
| Shell und Dateisystem | [EN](evidence/native-file-actions-39822990130/en/file-actions-shell-preserved.png) | [DE](evidence/native-file-actions-39822990130/de/file-actions-shell-preserved.png) |

## Schriftgröße

Die vorab definierten Erwartungen aus [dem lokalen Korrekturbeleg](FONT-VALIDATION-2026-10-22.md)
bestehen jetzt auch nativ: 15 wird übernommen; `abc`, 0 bei Fokuswechsel und
121 ändern die wirksame Größe nicht und zeigen einen lokalisierten Fehler.
Escape stellt 15 wieder her. Auch Korrektur auf denselben gültigen Wert entfernt
die Meldung. Danach wird 13 übernommen, nur Settings geschlossen und der
nicht exportierte Shellmarker 2231 mit `FONT-RETURN-PASS` bestätigt.

[Manifest und sämtliche Originale](evidence/native-font-validation-39828670969/manifest.json).

| Zustand | Englisch | Deutsch |
|---|---|---|
| Gültige Größe 15 | [EN](evidence/native-font-validation-39828670969/en/font-valid-before.png) | [DE](evidence/native-font-validation-39828670969/de/font-valid-before.png) |
| Ungültiger Text | [EN](evidence/native-font-validation-39828670969/en/font-invalid-text.png) | [DE](evidence/native-font-validation-39828670969/de/font-invalid-text.png) |
| Außerhalb des Bereichs | [EN](evidence/native-font-validation-39828670969/en/font-invalid-range.png) | [DE](evidence/native-font-validation-39828670969/de/font-invalid-range.png) |
| Escape stellt Wert wieder her | [EN](evidence/native-font-validation-39828670969/en/font-escape-restored.png) | [DE](evidence/native-font-validation-39828670969/de/font-escape-restored.png) |
| Korrektur auf unveränderten Wert | [EN](evidence/native-font-validation-39828670969/en/font-same-value-corrected.png) | [DE](evidence/native-font-validation-39828670969/de/font-same-value-corrected.png) |
| Shell erhalten | [EN](evidence/native-font-validation-39828670969/en/font-shell-preserved.png) | [DE](evidence/native-font-validation-39828670969/de/font-shell-preserved.png) |

## Reichweite

F5/F6/F8, Remote-Dateiaktionen, weitere Themes/Breiten, übrige Settings und die
vollständigen Aufgaben aus #564/#562 wurden durch diese gezielten Läufe nicht
geprüft. Die langen generierten Fixture-Pfade sind im Breadcrumb abgeschnitten;
der vollständige Rename-Pfad ist im Detailsdialog sichtbar. Das ist kein Nachweis
für die übrigen Pfad-/Tooltip-Varianten. Die Fehlerkorrekturen sind integriert und
nun nativ bestätigt; die Gesamtmatrix erhält keine pauschale Freigabe.
