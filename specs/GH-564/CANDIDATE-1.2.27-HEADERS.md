# Native Gegenprobe der Pane-Kopfzeilen

Die [Exploration 40068622076](https://github.com/byte5ai/zaplex/actions/runs/40068622076)
korrigiert den Fehlerbefund aus [#665](https://github.com/byte5ai/zaplex/issues/665).
Die unveränderte installierte App 1.2.27 kann Pane-Kopfzeilen fokussieren,
in alle vier Richtungen umordnen und als Tab abtrennen. Vor dem Klick oder
Drag wurde der Mauszeiger diesmal ausdrücklich mit `move` zum Header bewegt.
Die früheren wirkungslosen Automationssequenzen ließen diesen Schritt aus;
sie belegen keinen Produktfehler. Eine Produktänderung war nicht erforderlich.

App-Quelle: `b270ee372f8548b83363916dc8fbddc1ee7b5884`; Harness:
`a16ef5424c5092bcfe8af10bcd5704a646be4642`. DMG aus
[Batch 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378),
SHA256 `fcb082266c6b4f61fb8bb0c3d871e9ccafb8c229225e528f288bafb36f800644`.
Englisch, dunkles Theme, Fenster 1024×678 Pixel. Die Erwartungen für Fokus,
Abtrennung und Shell-Erhalt wurden vor den Aktionen festgehalten.

| Ablauf | Ergebnis | Aufnahmen |
|---|---|---|
| Normaler Rechts-Split | Zwei unabhängige Shells; BASE in `/tmp`, RIGHT in `/Users`. | 003–007 |
| Header ohne vorheriges `move` | Klick/Drag bleiben wirkungslos. Koordinaten sind korrekt; diese Operatorsequenz bildet das Heranführen der Maus nicht ab. | 008–009 |
| Header mit vorherigem `move` | RIGHT wird als zweiter Tab abgetrennt; frischer Befehl bestätigt dieselbe Shell. | 017–019 |
| Erneuter Tab-Join und Headerfokus | RIGHT wird wieder als rechte Pane eingefügt; Klick auf die linke Kopfzeile fokussiert BASE und aktualisiert den Tabtitel. | 020–022 |
| Vier Richtungen | RIGHT wird über die Kopfzeile unter, links neben, über und rechts neben BASE angeordnet. | 023–026 |
| Erneute Abtrennung | Drag vom freien Header-Hintergrund trennt RIGHT erneut als Tab ab. Frische Ausgaben in beiden Tabs bestätigen erhaltene Shells. | 027–031 |

Vorher/nachher: RIGHT hat PID `2232`, die nicht exportierte Variable
`RIGHT_40068622076` und CWD `/Users`; BASE behält PID `1301`,
`BASE_40068622076` und `/tmp`. Die frischen Befehle nach den Bewegungen
schließen eine bloße Wiederanzeige alten Scrollbacks als Nachweis aus.

Alle 32 Aufnahmen wurden einzeln betrachtet und stimmen bytegenau mit dem
GitHub-Artefakt `12550644497` überein. Das
[Manifest](evidence/candidate-127-headers/manifest.json) bindet ihre Hashes.
Der eigene Abschlussbefehl wurde vor Ausführung geprüft (032); der Workflow
einschließlich Cleanup ist erfolgreich.

Diagnostische Grenzen: Die Eingabe in 010 kam nicht an; nach explizitem
Eingabefokus erschien sie in 011. Die erwartete App-Logdatei war nicht vorhanden
(012), daher gibt es keinen Lognachweis. Der Ordnerbutton zeigt nach `move`
seinen Tooltip (014); 015–016 zeigen ein inaktives Fenster und zählen nicht
als Fokusnachweis. 021 klickt eine bereits fokussierte Pane; erst 022 belegt
den Fokuswechsel. Diese Beobachtungen werden nicht als Produktfehler gewertet.

P04 hat damit einen nativen lokalen Teilnachweis. Escape-Abbruch, ungültige
Ziele, kontinuierliche Sichtbarkeit während des Drags, Remote-Panes und die
übrigen Sprach-/Theme-/Fenstervarianten sind hier nicht nachgewiesen.
