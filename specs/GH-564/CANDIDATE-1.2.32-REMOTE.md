# Remote-Nachprüfung auf installiertem 1.2.32

[Exploration 40083814872](https://github.com/byte5ai/zaplex/actions/runs/40083814872)
prüft das unveränderte DMG aus dem erfolgreichen [Batch 40073495704](https://github.com/byte5ai/zaplex/actions/runs/40073495704).
App-Quelle: `c513867d323ad393a8cdfe951eff37a4a35ddcfc`; GUI-Harness:
`17b339b0c15a2619d7c4f0181bf82be1bc5ac745`.
DMG-SHA256: `d950c9db3cf0d17bdbc2505ed9b8b78d77e3d707dad72f0cdf5609d3ccf412f8`.
EN, Zaplex Dark, 1024×678 Punkte, Fensterursprung (0,30), 24.10.2026.
Die Erwartungen für #657/#658 wurden vor dem Lauf festgelegt; nächste Aktionen
folgen den jeweils frisch betrachteten Zuständen. Kein neuer Build, Release oder Tag.

Alle **53 Originalaufnahmen** wurden einzeln betrachtet und direkt aus der VM
abgerufen. 001–051 wurden zusätzlich bytegenau mit einem vor dem Abschluss
heruntergeladenen VM-Archiv abgeglichen; 052–053 entstanden nach dessen Erstellung.
Das [Manifest](evidence/candidate-132-remote/manifest.json) enthält die Einzelhashes.
Die Bilder und das vollständige Archiv bleiben wegen der Remote-Details privat.
Der Archivhash wurde sowohl in der VM als auch nach dem Download geprüft:
`c8a318d6462b727670c91536cfe2cc241237cea36aaf6eb309d54baecf54aa88`.
Installationsmetadaten im Archiv bestätigen App-, Harness- und DMG-Zuordnung.
Der Workflow wurde ausdrücklich beendet; beide Jobs und die Prozessbereinigung
sind erfolgreich.

| Fall | Beobachtung und Urteil | Aufnahmen |
|---|---|---|
| #657, eigener nativer ControlMaster | Installierte zsh-Integration führt bloßes `ssh -O check` und `ssh -O exit` aus. Eigener Master meldet PID2847; beide Rückgabecodes 0. Socket und Prozess verschwinden, lokale Shell1298/Marker/CWD bleiben erhalten. Kein Abfangen als neue Zaplex-Verbindung. | 011–015 |
| #657, tatsächlicher persistenter Transport | SSH-Eintrag aus der vorhandenen Konfiguration importiert, persistente Sitzung geöffnet. Lokale zsh5.9 erkennt genau einen Zaplex-ControlMaster, `check` meldet PID3748, `exit` bestätigt den Abbruch; beide Rückgabecodes 0. | 019–025 |
| Persistente Shell nach Transportabbruch | Frischer Befehl nach automatischer Wiederverbindung zeigt weiterhin PID8652, nicht exportierten Marker `PERSIST_40083814872=kept` und ursprüngliches Verzeichnis. | 022, 026–028 |
| SFTP-Vertrauen | Angezeigter ECDSA-Fingerabdruck wurde vor Zustimmung über die bereits verifizierte SSH-Verbindung abgeglichen. Erst danach wird der Dateimanager verbunden. | 029–034 |
| #658, laufender Remote→Lokal-Transfer | Künstliche 1-GiB-Datei, explizite lokale Zielpane in anderem Tab. Genau eine Fortschrittsanzeige innerhalb des aktiven Dateimanagers; Zielzeile und Funktionen bleiben sichtbar. | 035–036 |
| #658, Terminal und Rückweg | F10 zeigt genau eine globale Anzeige. Rückkehr per CmdShiftE stellt die Anzeige innerhalb der Dateipane wieder her. | 037–038 |
| #658, andere Pane/anderer Tab | Kleine lokale Zielpane zeigt nur eine kompakte Transferkopfzeile; Ziel und acht Funktionen bleiben sichtbar. Fokus auf die benachbarte Shell verschiebt die einzelne Anzeige nach global; die inaktive Dateipane zeigt kein Duplikat. Rückkehr zur großen Quellpane zeigt die vollständige Anzeige. | 039–041 |
| Pause und Abbruch | Sichtbar `Paused`, danach `Cancelling`, anschließend `Cancelled`. Abgebrochener Download erscheint nicht als Erfolg. | 042–044 |
| Unabhängige Dateikontrolle | Quelle weiter exakt 1.073.741.824 Bytes; SHA256 stimmt mit unabhängig berechnetem Hash dieser künstlichen Nulldatei überein. Unvollständige Zieldatei fehlt nach Abbruch. Lokale Shell1298 und ursprüngliche klassische Remote-Shell7592 behalten ihre Marker/CWD. | 048–050 |

Quelldatei-SHA256:
`49bc20df15e412a64472421e13fe86ff1c5165e18b2afccf160d4dc19fe68a14`.
Die verbliebene interne Verzeichnisreservierung im Ziel wird nicht als Nutzdatei
oder als erfolgreiche Übertragung gezählt. Der eigene zusätzliche ControlMaster
war mit 120 Sekunden Lebensdauer begrenzt und wurde ausdrücklich beendet.

002 enthält eine fehlerhafte Operator-Eingabe: Die Typing-Funktion interpretierte
eine Escape-Sequenz als Zeilenumbruch. Der Rest wurde abgebrochen; 003–004 prüfen
den tatsächlichen Shellzustand neu. 016 zeigt die ehrliche Ablehnung eines
Dateimanagerzugangs über eine noch nicht registrierte klassische SSH-Identität.
Der folgende Transfer verwendet den explizit importierten Eintrag. 046 ist eine
abgeschnittene Terminalausgabe und kein Dateinachweis; 050 liefert den separaten,
vollständig sichtbaren Nachweis. Diese Operator-/Kontextfälle werden nicht als
neue Produktfehler eingeordnet.

Diese Ergebnisse gelten für EN/dunkel und die beschriebenen Abläufe. Native
Bash-/Fish-Steuerbefehle, weitere Sprach-/Themevarianten, die Transferanzeige bei
getrennter SFTP-Verbindung und aktive Agenten während dieses Transportabbruchs
wurden hier nicht geprüft. Echte Abonnements/Kontorouten sind nicht Bestandteil
dieses Laufs. Standbilder belegen keine kontinuierlichen Übergänge. Die
vollständige Produktmatrix bleibt offen; 1.2.33 und spätere Änderungen erben
keine pauschale native Freigabe aus diesem 1.2.32-Nachweis.
