# Prüfstand 1.2.53

Quelle `3fe124f00d7615398309753415639465d42b1450`, Version 1.2.53.
[Batch 41119368489](https://github.com/byte5ai/zaplex/actions/runs/41119368489)
ist vollständig erfolgreich. Alle acht Repository-/Modalfälle liefen genau einmal;
27 positive Ergebnisblöcke enthalten 822 bestandene Ausführungen ohne Fehler oder
ignorierte Fälle. Die teilweise überlappenden Filter bilden keinen disjunkten Gesamtbestand.
[App-Nachweis](evidence/candidate-153-native/repo-picker-ci.json).

Die Mac-Paketsuite entspricht exakt den vorher eingefrorenen 4.403 unterschiedlichen
Fallkennungen: keine Fehler, Skips oder aufgezeichneten Wiederholungsfehler.
[Unabhängiger JUnit-/Archivvergleich](evidence/candidate-153-native/package-tests.json).
Das unveränderte installierte DMG besteht alle 16 eingefrorenen GUI-Fälle; Quelle,
Version, Signatur und tatsächlicher DMG-Hash wurden separat geprüft.
[Installations-/GUI-Nachweis](evidence/candidate-153-native/installed-gui.json),
[vollständiger Batch](evidence/candidate-153-native/completed-batch.json).

Die zusätzliche reale Bedienung in Englisch/dunkel, Normalbreite, bestätigt das
sichtbare aktuelle Repository und einen tatsächlich eingegebenen Suchtext ohne Treffer.
Zwei Klicks in die Zeile „Add new repo“ schlossen jedoch die Liste, ohne einen
nativen Ordnerdialog zu öffnen. Ordnerwahl, Ordnerabbruch und anschließendes
Worktree-Anlegen sind in dieser Sitzung nicht ausgeführt. Der tatsächliche Maustest bestätigt inzwischen #753: links wird die Aktion
ausgelöst, in der Mitte nur das Menü geschlossen. Die Klickfläche wurde für
1.2.54 auf die volle Zeilenbreite erweitert; die native Gegenprobe läuft separat.

Der Modalabbruch erhält die ursprüngliche Shell (PID 1455, nicht exportierter
Marker und Arbeitsverzeichnis). Archivierte Git-Schnappschüsse davor/danach sind
bytegleich: sauberer Status, gleicher HEAD und genau ein Worktree. Diese Rückkehr
ist durch archivierte Ausgabe belegt; ein frisches Rückkehrbild wurde nicht mehr
abgerufen. 13 Originalbilder wurden tatsächlich angesehen und gegen die
Workflow-Artefakte bytegenau geprüft.

Die Sitzung wurde ausdrücklich beendet; die eigene VM ist beendet. Das vorher
angesetzte Bedienzeitbudget von weniger als 600 Sekunden wurde mit 650,304503
Sekunden überschritten. Der strenge Archivprüfer scheiterte daran; der separate
Teilnachweis hält diesen Fehler ausdrücklich fest und erteilt keinen nativen
Gesamtpass. Workflow-Erfolg ist hier nur ein Infrastrukturbeleg.
[Teilnachweis mit Budgetfehler](evidence/candidate-153-native/native-en-dark-partial.json).

Keine Freigabe für Release oder Tag und kein Gesamtpass für die Produktmatrix.
