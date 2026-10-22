# Ungültige Terminal-Schriftgröße: lokaler Korrekturbeleg

Die Settings-Exploration am installierten DMG 1.2.2 fand in [#622](https://github.com/byte5ai/zaplex/issues/622)
eine ungültige Schriftgröße ohne Rückmeldung. [PR #625](https://github.com/byte5ai/zaplex/pull/625)
korrigiert dies in **1.2.20**, Merge `15d6cc582f77fdbabbc9f8488433c04980194f4b`.
Der hier geprüfte Quellstand ist `6e4991baa`; die unveränderten Originalbilder
und Schrittprotokolle sind im [Manifest](evidence/font-validation-linux-2026-10-22/manifest.json) verzeichnet.

**Beide lokalen Linux-GUI-Fälle bestanden**, jeweils 24 Schritte auf einem echten
X11-Display mit Software-Rendering, Dark, 1024×640. Alle zwölf Originalbilder
wurden einzeln angesehen. Dies ist kein macOS- oder DMG-Nachweis.

| Erwartung vor Ausführung | Beobachtung | Belege EN / DE |
|---|---|---|
| Gültige Größe 15 übernehmen | Feld und wirksame Einstellung zeigen 15; kein Fehler | [EN](evidence/font-validation-linux-2026-10-22/en/font-valid-before.png), [DE](evidence/font-validation-linux-2026-10-22/de/font-valid-before.png) |
| Ungültigen Text erklären, Einstellung erhalten | `abc` zeigt Rahmen und vollständigen lokalisierten Hinweis; wirksame Größe bleibt 15 | [EN](evidence/font-validation-linux-2026-10-22/en/font-invalid-text.png), [DE](evidence/font-validation-linux-2026-10-22/de/font-invalid-text.png) |
| Bereich prüfen, auch beim Fokuswechsel | `0` nach Klick ins sichtbare Suchfeld und `121` nach Enter bleiben unwirksam; Fehler erscheint | [EN](evidence/font-validation-linux-2026-10-22/en/font-invalid-range.png), [DE](evidence/font-validation-linux-2026-10-22/de/font-invalid-range.png); Fokuswechsel im Schrittprotokoll |
| Escape verwirft den ungültigen Entwurf | Feld wieder 15, Rahmen und Fehlermeldung entfernt | [EN](evidence/font-validation-linux-2026-10-22/en/font-escape-restored.png), [DE](evidence/font-validation-linux-2026-10-22/de/font-escape-restored.png) |
| Korrektur auf denselben gültigen Wert genügt | Erneutes `abc` → `15` entfernt den Fehler auch ohne Änderung der wirksamen Einstellung | [EN](evidence/font-validation-linux-2026-10-22/en/font-same-value-corrected.png), [DE](evidence/font-validation-linux-2026-10-22/de/font-same-value-corrected.png) |
| Settings schließen erhält die Shell | Größe 13 übernommen, nur Settings geschlossen, nicht exportierter Marker 2231 erhalten; `FONT-RETURN-PASS` | [EN](evidence/font-validation-linux-2026-10-22/en/font-shell-preserved.png), [DE](evidence/font-validation-linux-2026-10-22/de/font-shell-preserved.png) |

Die Assertions prüfen Editorinhalt, wirksame Einstellung und die Position der
Fehlermeldung im tatsächlich gerenderten Frame. Ein erster Escape-Nachweis
scheiterte an einer über Frames gespeicherten Position; der Messpunkt wurde auf
den aktuellen Frame begrenzt. Der lokale Shellstart überschritt zuvor 20 Sekunden;
unter Diagnose war er nach rund 40 Sekunden abgeschlossen. Nur dieser
Vorbereitungsschritt hat nun eine 60-Sekunden-Grenze. Die fachlichen Erwartungen
wurden nicht abgeschwächt.

Die gezielte [macOS-Nachprüfung 39828670969](https://github.com/byte5ai/zaplex/actions/runs/39828670969)
ist in DE/EN bestanden; alle zwölf Originalbilder wurden einzeln geprüft ([nativer Nachweis](NATIVE-ACTIONS-FONT-2026-10-22.md)). Andere Settings, Themen, Breiten, die Tab-Navigation und die
aktuell installierte DMG-Version sind durch diese beiden Fälle nicht geprüft.
Die Teilbelege ergänzen E01/E02/X05; #564/#562 erhalten dadurch keine Gesamtfreigabe.
