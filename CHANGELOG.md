# Changelog

This document records key changes: the Zap/Warp release history inherited before the zaplex fork (translated from the original Chinese), followed by zaplex's own work on top of it. Only functional commits are listed; internal dev/stable rolling tags are omitted.

## [1.1.1] — 2026-09-23

- Die CI-Compilefehler im SSH-Dateipicker, der Einstellungsübernahme und den Bootstrap-/Terminalbild-Tests sind korrigiert (#472). Schwache View-Referenzen, Fehlermeldungen und vorhandene Testaussagen bleiben erhalten.

- Der gemeinsame Dialogtest (#472) prüft Cockpit- und SSH-Dialoge mit echten Presenter-Ereignissen, erhaltenen Eingabeentwürfen und gesperrten Hintergrundaktionen. Der Auditrunner-Test berücksichtigt kanonische temporäre Pfade.

- Die Lockdatei entfernt einen verwaisten ndarray-Eintrag aus der ORT-Aktualisierung (#472). Die festgeschriebene Abhängigkeitsauflösung funktioniert wieder ohne Versionsänderungen.

- Die vorbereitete Coverage-CI (#472) archiviert zusätzlich vollständige LLVM-Funktions-/Regionsdaten und Abhängigkeitsdiagnostik. Exportfehler bleiben sichtbar fehlgeschlagen; die bestehende Abnahme wird nicht gelockert.

- Die Coverage-Quellprojektion (#472) schließt testabhängige Match-Arme mitsamt ihrem ausgeführten Körper aus. Makroauflösung, generierte Derives und unbekannte Featurebedingungen bleiben ohne zusätzliche Belege ausdrücklich unbewiesen.

- Die Coverage-Quellprojektion (#472/#245) verarbeitet bekannte Rust-Patternattribute und Trait-Lifetime-Schreibweisen ohne Verlust von Positionen oder Originalhashes. Testabhängige Feldwerte bleiben ausgeschlossen; andere unklare Regionen verhindern weiterhin eine unbelegte Gesamtquote.

- Gespeicherte Drive-Seitenleisten (#245) bleiben nach der Umbenennung erhalten. Alte und neue Tabnamen werden gelesen; Auswahl, Pane-Zuordnung und Breite gehen beim Wiederherstellen nicht verloren.

- SFTP-Integrationsprüfungen (#245) erreichen die behaupteten Schreib- und Ersetzungsfehler, statt schon an fehlenden Streaming-Methoden zu scheitern. Der Speichern-unter-Fall verschiebt den ursprünglichen Dateieintrag tatsächlich; ein Unix-spezifischer Löschfall erhält den passenden Plattformfilter.

- SFTP-Prüffälle (#245) erzeugen die benötigten Overlay-Dateien und erzwingen bei Sortierung und Aktualisierung einen tatsächlichen Zeilenwechsel. Dadurch erkennen sie verlorene Cursor- und Markierungszuordnungen.

- Der Fish-SSH-Wrapper (#245) aktiviert die vorhandene Aufräummeldung entfernter Bash-/Zsh-Shells. Windows-Prozessgruppen halten ihre ursprüngliche Prozessidentität bis zum Ende der Bereinigung fest; ein gleichzeitiger Abbruch kann keine wiederverwendete Prozessnummer treffen.

- Website-Installationspfade (#245) führen zu den aktuellen Zaplex-Releases und zur Installationsanleitung. Historische Entwürfe und Übergabepläne sind als solche gekennzeichnet; die SSH-Diagnose nennt die tatsächlich ausgewertete Debug-Variable.

- Theme-Farben (#245) weisen ungültige Unicode-Hexwerte als Eingabefehler zurück, statt beim Parsen abzustürzen. Das Löschen eines importierten Themes erhält Bilddateien außerhalb des eigenen Theme-Verzeichnisses, auch hinter Symlinks.

- Gespeicherte Einführungstipps (#245) behalten alte Drive-Namen und die übrigen erledigten Tipps. Die eigenständige Workspace-Vorschau bietet ihre Theme- und Recovery-Optionen auch ohne externe Entwurfssteuerung an.

- Cockpit-Abnahmeberichte (#160/#169/#245) akzeptieren nur Test- und Laufzeitbelege des geprüften Commits. Screenshot-Prüfungen erkennen auch CRC-gültige, aber beschädigte PNG-Bilddaten und begrenzen die Dekompression. Der geerbte Windows-Installer verwendet denselben Sitzungs-Mutexnamen wie die App.

- Umbenannte SSH- und Editoreinstellungen (#245) erhalten frühere Abschaltungen, Sperrlisten und Editorwahlen. Neue explizite Werte gewinnen; ein Zurücksetzen reaktiviert keine alten Werte. Native Lesefehler brechen die Erstübernahme ab, ohne Einstellungen zu überschreiben.

- Die Reviewansicht (#245) zeigt Fehler beim Lesen eines Git-Repositories an, statt ein unlesbares Repository als unverändert darzustellen. Zaplex erkennt seine eigene macOS-App als Editor.

- `/plan` (#245) aktiviert bei unterstütztem Claude Code einen eingeschränkten nativen Planmodus und erhält die Sitzung für die nächste normale Anfrage. Der sichtbare Befehl wird nicht nochmals an die CLI weitergereicht; nicht zuverlässig abgesicherte Codex-Planaufrufe werden ausdrücklich abgewiesen.

- Die Signiervorbereitung (#245) erhält vorhandene Keychain-Pfade samt Leerzeichen und stellt die Suchliste wieder her. Temporäre Signierdateien liegen in einem privaten Verzeichnis; geerbte Pfade werden beim Aufräumen nicht angefasst. Der Changelog-Generator behandelt einen fehlgeschlagenen Release-Abruf nicht als leere Historie.

- CLI-Agenten (#245) erhalten aufgerufene Skill-Inhalte, benannte Anhänge und Reviewkommentare mit Diff-Kontext. Globale Regeln werden unter dem bestehenden Einwilligungsschalter übertragen; gelöschte Regeln bleiben ausgeschlossen.

- Spracheingabe (#245) lässt sich nur vom zugehörigen Editor abbrechen. Verspätete Aufnahmeergebnisse verändern keine neuere Sitzung; korrigierte Suchtestdaten verwenden die tatsächlichen Bytepositionen.

- Drive-Importe (#245) verwerfen verspätete Dateiauswahlen nach Schließen oder Zielwechsel. Sammelexporte lehnen vorhandene Symlinks als Zielunterordner ab und erhalten bestehende Dateien.

- Die Bereinigung alter Linux-Zugangsdaten (#245) akzeptiert frühere Dateirechte nach Prüfung und Reparatur. Eine doppelte Testmodul-Deklaration und ein verfälschter Unicode-Erwartungswert sind korrigiert.

- Headless-Agenten (#245) erhalten ausgewählten Text, Dateikontext, Referenzen und laufende Befehlsausgabe zusammen mit der Anfrage. Bilder gehen als native Bilddaten an Claude/Codex; die angezeigte Anfrage bleibt unverändert.

- Aufeinanderfolgende Logdatei-Generationen (#245) verlieren ihre Ausgabe nicht mehr durch verspätetes Schließen des Vorgängers. ONNX-Aufrufe passen zum vorhandenen ORT-Lockstand; veraltete Windows-Pfade, Lexerpositionen und ein verwaister WASM-Modulfilter sind korrigiert.

- Die Freigabematrix (#245) bindet alle Laufzeitbelege an dasselbe Artefakt und prüft die Protokollversion. Der Übersetzungscheck erkennt mehrzeilige Literale und hält seine temporären Dateien in einem eigenen Verzeichnis.

- Audit-Verifikation (#245) verlangt die archivierten Originalprotokolle und zum Commit passende Quellen. Wiederaufnahmen prüfen bestehende Belege erneut; ausgeschlossene sensible Dateien bleiben als offene Prüflücke sichtbar. Die Freigabematrix weist doppelte Zeilen und fehlerhafte Abschnittsgrenzen zurück.

- Terminal-Dateilinks (#245) öffnen während Remote-Reconnect oder nach dem Entfernen einer Session keine gleichnamige lokale Datei. Verzeichnislinks verwenden die tatsächliche Shell; Unicode-Suchfälle sind in den bestehenden Testdaten wiederhergestellt.

- SSH-Installationsfehler (#245) liefern gültige Statusmeldungen auch bei Sonderzeichen im Befehl. Fish übergibt das eingebettete Skript unverändert; portable tmux-Wrapper behandeln den Home-Pfad als Daten.

- Fork-Auditbelege (#245) prüfen gespeicherte Reviewartefakte einschließlich ihrer Quellblobs statt nur Hashformate. Als binär markierte UTF-8-Dateien behalten ihre tatsächlichen Fork-Hunks im Inventar.

- Terminalbilder (#245) erhalten pro Übertragung einen eigenen Cache-Schlüssel. Gleiche Kitty-Bildnummern in verschiedenen Terminals überschreiben einander nicht; verspätetes Aufräumen entfernt keine neu übertragenen Bilder.

- Automatische SSH-Starts (#245) erhalten den aktuellen Eingabeentwurf sowie ausgewählte Workflows und Umgebungsdaten. Nur der zugehörige Systembefehl darf den Entwurf beim Abschluss erhalten.

- SSH-Dateiuploads (#245/#464) geben SFTP-Fehler zuverlässig als Fehlerstatus weiter und verwenden eine passende native Shell. PowerShell übergibt die UTF-8-Batchdatei direkt an SFTP.

- SSH-Automatik (#245) wartet auf den zugehörigen Verbindungsstart. Abbruch, Verbindungsende oder ein neuer Befehl verwerfen wartende Passwort- und Startbefehle; Askpass-Dateien bleiben bis zur Anmeldung erhalten und werden bei Abbruch entfernt.

- SFTP-Arbeitskopien (#245/#464) bleiben bei ausstehenden oder fehlgeschlagenen Uploads nach dem Schließen erhalten; Fehlermeldungen nennen den Wiederherstellungspfad. Erfolgreich übertragene Kopien werden wie bisher aufgeräumt.

- Ordnerauswahlen (#245) bleiben an den ursprünglichen Startdialog und Host gebunden. Agent-Neustarts prüfen vor dem Beenden die Wiederaufnahmefähigkeit; verspätete Antworten entfernen keine inzwischen neu gestartete Sitzung.

- Terminal-Testdaten (#245/#472) stimmen wieder mit ihren Regex-Mustern und URL-Zellgrenzen überein; negative Punktprüfungen treffen tatsächliche Satzzeichen.

- SSH-Formulare (#245) sperren Änderungen während laufender Speicherung und verwerfen veraltete Dateiauswahlen. Tailscale-Importe rollen bei einem Fehler den gesamten Import zurück; alle sieben Servermenüaktionen behalten ihren Mauszustand.

- Kitty-Grafiken (#245) löschen nur die angeforderten Platzierungen und erhalten weitere Referenzen, Scrollback, virtuelle Platzierungen und iTerm-Bilder. Bildschirmkoordinaten berücksichtigen gescrollte Ausgabe; auch ersetzte Animationsframes müssen innerhalb des Speicherlimits bleiben.

- Fish-Agentenstarts (#245) erhalten Backslashes und Apostrophe in Konto-, Programm- und Sitzungsargumenten.

- SSH-Starts (#245) binden Konten nur an tatsächlich geöffnete Terminals. Verspätete Startergebnisse öffnen geschlossene oder neu konfigurierte Startdialoge nicht erneut. Der klassische Verbindungstest lehnt widerrufene Hostschlüssel auch nach einer Bestätigung ab und prüft fehlende Passwörter vor jedem Netzwerkzugriff.

- CLI-Agenten (#245) erhalten unter PowerShell die ursprüngliche Konto- und API-Umgebung sowie ihren Exitstatus. Fremde Anbieterereignisse verändern keine bestehende Sitzung. Fish-Dateipfade bleiben bei Backslashes und Apostrophen korrekt gequotet; Standardpfade für entfernte Shell-Historien behalten die Home-Expansion.

- SFTP-Konfliktdialoge (#245/#459) umbrechen übersetzte Aktionen entsprechend dem verfügbaren Platz.

- Startvorlagen (#245) behalten den ausgewählten Tab auch dann, wenn ein späterer Tab angepinnt wird. Leere oder vollständig ungültige Vorlagen öffnen ein nutzbares Fenster statt beim fehlenden aktiven Tab abzubrechen.

- Dateiübertragungen (#245/#464) erhalten leere Ordner und halten laufende Rückrufe beim Aufräumen abgeschlossener Transfers gültig. Lokale Pfade bleiben in ihrem nativen Format erhalten; Windows-Downloads weisen Laufwerkswechsel und alternative Datenströme in entfernten Dateinamen zurück.

- Mehrhost-Terminals (#245/#456) übernehmen Ausgabe, Sitzungsende und Hinweise nur von ihrer eigenen Daemon-Verbindung, auch wenn ein anderer Host dieselbe PTY-Kennung verwendet.

- Lokale Dateiansichten unter Windows (#245) erhalten absolute Laufwerks- und UNC-Pfade; abgeschottete Testdateisysteme bleiben getrennt. Escape schließt im fokussierten Dateimanager zuerst das Kontextmenü, anschließend den Dialog. SFTP-Wiederherstellung behält mehrere offene Vorgänge am selben Pfad und bereits erreichte Wiederherstellungsergebnisse über spätere Fehler und erneute Versuche hinweg.

- Der Startdialog (#245) prüft die Startfreigabe auch bei Enter, reserviert Ziele vor asynchroner Prüfung und verwirft verspätete Starts nach Auswahlwechsel oder Schließen. Ordnerhistorien mehrerer Workspaces bleiben erhalten; mehrdeutige Hostnamen werden nicht automatisch zugeordnet. Stop-/Kill-Bestätigungen nehmen am gemeinsamen Modal-Fokus und Schließen teil.

- Der Themeeditor (#245) verwirft verspätete Bildverarbeitung nach Vorlagenwechsel oder Schließen der Vorschau. Die Command-Palette bietet GitHub-Analysen nur bei einem tatsächlich unterstützten lokalen Claude-Konto an.

- Pane-Ziele (#245/#460/#462) werden nach Verschieben, Ausblenden und Dateimanager-Wechsel ungültig. Cockpit- und Dateimanager-Kopfzeilen leiten ihre Drag-/Tab-Aktionen an die Pane-Gruppe weiter; Remote-Wiederverbindung erhält die gemeinsame Titel- und Hostkonfiguration.

- Der Daemon (#245) erhält Agentenzuordnungen bei Inventarabfragen älterer Clients. Ein veralteter Projektpfad einer anderen Session blockiert neue, unabhängige Starts nicht mehr.

- Root-Passwortbestätigungen (#245) bleiben an den ursprünglichen Befehl und die Sitzung gebunden. Abbruch, Blockende und Schließen verwerfen auch noch ausstehende Antworten aus dem Schlüsselspeicher; diese können kein geschlossenes Menü erneut öffnen.

- SSH-Vertrauen (#245): `@revoked`-Hostschlüssel werden vor Anmeldung und manueller Bestätigung abgewiesen. Aliase, Hashes, Hostmuster und Portgrenzen bleiben berücksichtigt.

- Einstellungen (#245) speichern die gewählte Seite mit sprachunabhängigen Schlüsseln und lesen frühere Namen weiter. Externe SSH-Einstellungen aktualisieren die Dropdown-Bedienbarkeit sofort; bei mehreren CLI-Agenten verspricht der Reparaturknopf keinen bereits gewählten Anbieter.

- Kontrollschnittstelle und Quellidentität (#245): Bereits vorhandene Worktrees müssen zum angeforderten Repository gehören. Gepackte Git-Referenzen werden für die eingebettete Commit-ID mitbeobachtet; der folgende Commit bleibt damit als Änderung erkennbar.

- Subscription-Agenten (#245) behalten bei geänderten Anzeigenamen ihre exakte Kontoauswahl. Ein Abbruch während Discovery oder Sitzungsinitialisierung beendet den Start, bevor ein weiterer Prompt zugestellt wird.

- SFTP-Speichern (#245) verwendet die bestätigte POSIX-Rename-Erweiterung des Servers statt eines Shell-Move mit Verzeichnisrennen. Wiederherstellung nach Überschreiben behält alle verifizierten Backup- und verdrängten Zieldateien bis zum erfolgreichen Aufräumen.

- CI-/Release-Korrekturen (#245/#468) lassen den Pflichtcheck bei fehlgeschlagenen Vorprüfungen scheitern, binden fertige Daemon-Caches an den Quellcommit und verlangen tatsächlich ausgeführte Live-SFTP-Tests. Vorhandene Release-Tags müssen zum gebauten Commit gehören; Fehler beim Abruf des letzten Releases werden nicht mehr als leere Release-Historie gewertet.

- Cockpit-Audit (#245): Fehler beim Lesen der Nutzungshistorie sperren die automatische Auswahl scheinbar freier Konten. OAuth-Limits bleiben an Konto und Zugangsdaten gebunden; der erste vollständige Scan erzeugt keinen Startton. Bare-Git-Repositories bleiben getrennte Projekte, und unbekannte Prozesszustände gelten nicht mehr als beendete Sessions.

- GitHub-Analysen (#245) binden Diff, Bestätigung und Review an konkrete Commits; ein Merge verlangt weiterhin denselben Head. Die Analyse nutzt eine getrennte lokale Claude-Sitzung mit ausschließlich Lesewerkzeugen und ohne geerbte Erweiterungen. Codex steht für diese Analyse vorerst nicht zur Auswahl, solange externe Schreibwerkzeuge nicht sicher abgeschaltet werden können.

- Weitere Cockpit-Korrekturen (#245) erhalten Konten-Aliase beim Bearbeiten, zeigen wartende Sessions zuerst und wenden Einstellungen sofort an. Lokale Scanfehler und nicht belegte RAM-Werte bleiben sichtbar unvollständig. Auch der CLI-Export meldet bei lokalen Sessions ohne Kontoinventar nur Teilerfolg.

- Dateiübertragungen (#245) behalten nach bereits veröffentlichtem Ziel bei späteren Lese- und Prüfungsfehlern ihren tatsächlichen Zustand und die benötigten Wiederherstellungspfade. Ein fehlgeschlagener Aufräum-Retry verliert seinen übernommenen Eigentumsnachweis nicht mehr.

- Unvollständige Prozessmessungen zeigen keinen vermeintlich vollständigen Speicherverbrauch. Beim Aufräumen eigener Remote-Dateien bleibt eine nicht eindeutig zuordenbare Löschung als Wiederherstellungsfall sichtbar (#245).

- Worktree-Bereinigung (#245) erhält ungemergte lokale Commits auch bei fehlendem Remote-Branch, bricht nach Fetchfehlern ab und prüft das Ziel nach der Bestätigung erneut. Der Commit-Schutz erkennt Hauptverzeichnisse mit Leerzeichen im Pfad korrekt.

- Remote-Speicherstände (#245) erfassen Host-, Daemon- und PTY-Identität, Wiederherstellungsfehler und den temporären Dateimanager gemeinsam mit der Pane. Späteres Schließen oder Wechseln der Pane kann diese bereits vorgemerkten Daten nicht mehr verändern. Ein Rollback der Remote-Pane-Migration entfernt zugehörige Dateimanager-Knoten vollständig und erhält Terminalgeschwister.

- SSH-Wiederaufnahme (#456): Der Daemon sichert vollständige Shell-Startmeldungen unabhängig vom Client. Bestehende Sitzungen mit verlorenem Startpräfix bleiben über einen gekennzeichneten einfachen Terminalmodus bedienbar; Prozesse, PTY und Eingabeentwurf bleiben erhalten, automatische Befehle bleiben gesperrt.

- Aufgelistete Remote-Sessions behalten ihre bestätigte Host- und Daemon-Version beim Wiederöffnen ohne aktive Verbindung. Wiederherstellbare Sessions lassen sich während des Attach-Vorgangs abbrechen und danach erneut verbinden. Auch ein neuer Verbindungsversuch vor bestätigter PTY kann lokal abgebrochen werden, ohne den Entwurf zu verlieren oder entfernte Arbeit zu beenden (#456).

- Sessionzeilen ohne eigenen Titel zeigen Host und tatsächliches Arbeitsverzeichnis; der vollständige Pfad bleibt im Tooltip sichtbar (#455). Die Auswahl nach `..` ist zusätzlich über Maus-/Tastaturereignisse und veraltete oder fehlgeschlagene Verzeichnisantworten abgesichert (#458).

- Issue-Nachprüfung: Der Split-Picker kennzeichnet den aktuellen Host und meldet Registry-Ausfälle (#460); Favoriten verwechseln Lesefehler nicht mit entfernten Hosts (#463). Dateiaktionen sind nur bei tatsächlichem Fokus in der Dateimanager-Pane aktiv (#464). Verbindungs-, Fehler- und Wiederherstellungshinweise bleiben außerhalb des Terminalrasters (#470). Pfad-Completion funktioniert auch mit dem Legacy-SSH-Fallback nach übersprungener oder fehlgeschlagener Remote-Server-Einrichtung (#471).

- Dateimanager-Rückkehr (#469) übernimmt das besuchte Verzeichnis in die zugehörige Shell und erhält Eingabeentwürfe; laufende Prozesse bleiben unberührt. Kollidierende Pane-Titel bleiben anhand von Pfad und stabiler Session-Kennung unterscheidbar (#461). Ungültige Drag-Ziele verändern weder Layout noch Undo-Zustand (#462). Transfers prüfen nach Konfliktdialogen ihr Ziel erneut und überschreiben nachträglich angelegte Dateien nicht ohne Zustimmung (#464).

- Review-Korrekturen zu #467/#468: SQLite-Migrationen übernehmen einen konsistenten Datenstand einschließlich WAL und bleiben nach Fehlern wiederholbar. Fehler bei der Datenbankinitialisierung brechen den Start mit Diagnose ab, statt Arbeit ohne Speicherung zuzulassen. CI-Buildschritte erhalten keine unnötigen Repository-Schreibrechte; neue Release-Tags werden an den gebauten Commit gebunden.

- UI-Korrekturen zu #459: Sessionbaum und Konten scrollen unabhängig; Sidebar-Typografie, Theme-Kontraste und Favoriten-Flyouts sind vereinheitlicht. Automatische Terminaltitel behalten Host und Verzeichnis auch bei laufenden Agenten. SFTP-Dialoge geben übersetzten Aktionen genügend Platz und bieten nach Verbindungsfehlern einen erneuten Versuch. Start- und Reconnect-Meldungen erscheinen erst nach tatsächlicher Shell-Bereitschaft.

## [1.1.0] — 2026-09-21

- **Premium-Workspace für UI #459:** Terminal- und Dateimanager-Panes bewahren ihre Host-Identität in Mehrhost-Tabs; Cockpit und Verbindungen zeigen die zugehörigen Sessions. Ein Klick auf einen Favoriten-Host öffnet die Verbindung im neuen Tab, während das separate Aktions-Flyout neue Agenten sowie Bearbeiten und Entfernen anbietet. Restore erhält Host-, Pane- und Dateimanager-Modus; Shell-Bereitschaft und Eingabeentwürfe bleiben beim Wiederverbinden sichtbar und sicher, Dateioperationen an das gewählte Ziel gebunden.

## [1.0.29] — 2026-09-11

- Das Cockpit kennzeichnet KI-Sessions eindeutig, verlinkt Shell-Sessions unter Verbindungen und aktualisiert lokale sowie entfernte Inventare unabhängig. Hängende Remote-Abfragen blockieren andere Hosts nicht mehr.
- Sichtbare Aufklapp-/Refresh-Aktionen und Tastaturbedienung verbessern die Verbindungsliste; feste Statusplätze verhindern springende Sessionzeilen. Wiederverbundene Sessions zeigen den tatsächlichen Verbindungs- und Installationsfortschritt.
- Subscription-Agenten verlassen fehlerhafte Startzustände bedienbar, prüfen Ziele vor jedem Prompt erneut und behalten ihre Registry-Identität über Daemon-Neustarts. Änderungen an CLI-Version oder Modellidentität verlangen eine explizite neue Unterhaltung und verwerfen den bisherigen Kontext nicht still. Remote-CLI-Starts verwenden die bestätigte SSH-Verbindung und den tatsächlichen CLI-/Login-PATH.
- Die SFTP-Schlüsselprüfung verarbeitet große bekannte Hostlisten linear und bewahrt gehashte Hosts, kurze Aliase sowie Port- und Algorithmusgrenzen.

- Die Verbindungsliste hält Sessiontitel, Zusatzangaben und Aktionen auch in schmalen Seitenleisten lesbar; vorhandene Sessions bleiben bei fehlerfreier Aktualisierung sichtbar. Nach einem Fehler bleibt dessen Meldung bis zur erfolgreichen Wiederholung stehen. Eine vollständige Inventarabfrage endet spätestens nach 30 Sekunden mit einem Fehlerhinweis; eine erforderliche Erstinstallation erfolgt über das Verbinden des Hosts.
- Ein Klick auf eine bereits geöffnete Zaplex-Session fokussiert ihre vorhandene Shell, auch hinter dem Dateimanager. Abgewiesene Attach-Versuche beenden den Startzustand sichtbar; das erstmalige Wiederanbinden wartet höchstens 60 Sekunden und beendet keine Remote-Session.

- Das Cockpit zeigt verbundene Remote-Hosts sofort an. Regelmäßige Aktualisierungen verwerfen laufende Abfragen nicht mehr; getrennte Hosts bleiben auch bei verspäteten Antworten entfernt.

- Der Dateimanager bevorzugt bereits bekannte SSH-Schlüsseltypen und unterscheidet zusätzliche Schlüsseltypen von echten Schlüsseländerungen. Beim bestätigten Ersetzen bleiben andere Hosts, Ports und Schlüsseltypen erhalten.

- Die Remote-Daemon-Installation akzeptiert gültige SHA-256-Prüfsummen mit 64 Zeichen; fehlerhafte oder abweichende Prüfsummen werden weiterhin vor dem Entpacken abgewiesen.

- Neue Zaplex-App-Icons und Installationsgrafiken werden in die Plattformpakete übernommen.
- Subscription-Routing ersetzt verschwundene ausgewählte Agenten oder Konten nicht mehr still durch andere Ziele; mehrdeutige Konten verlangen eine explizite Auswahl.
- Persistente Remote-Sessions bleiben nach App-Neustart und Versionswechsel auffindbar: ältere Daemons werden ausschließlich für ihre bestehenden Sessions wiederverbunden, neue Arbeit bleibt beim aktuellen Dienst. Attach, Stop, Transkripte und sessiongebundene Dateioperationen behalten ihre exakte Daemon-Identität.
- Die Verbindungs-Sidebar zeigt und aktualisiert Zaplex-Sessions automatisch, bevorzugt Agent und Aufgabe als Titel und wahrt bewusst zugeklappte Abschnitte. Mehrere Daemons desselben Hosts bleiben getrennt geroutet; ihre unabhängigen Ring-Speicherlimits werden nicht als gemeinsamer Host-Cap dargestellt.
- Gehärtete Dependency-Policy: CI prüft den vollständigen Cargo-Deny-Vertrag und erzeugt die auslieferbare Lizenzattribution; aktualisierte Makro-, SVG- und Font-Abhängigkeiten entfernen überholte Pfade, während verbleibende Ausnahmen exakt dokumentiert und statisch abgesichert sind.
- Der Subscription-Agent-Footer hält Identität, lange Account-, Host-, Modell- und Verzeichnisangaben sowie alle Lifecycle-Aktionen bei normaler und schmaler Fensterbreite sichtbar und ohne Überlagerungen.

## [1.0.28] — 2026-09-11

- Der Subscription-Agent wendet erkannte Modelle nur noch auf das exakt ausgewählte Konto an, überspringt Auth-Fehler anderer Konten und erlaubt frei eingegebene Remote-Verzeichnisse im laufenden Agent-Footer.

## [1.0.27] — 2026-09-11

- Der Agent-Einstieg routet Claude Code und Codex jetzt konto-, host-, verzeichnis-, modell- und CLI-versionsgenau; dynamische Modelldiscovery, sichere Wechselzustände und explizite Session-Aktionen ersetzen statische oder stille Fallbacks.

## [1.0.26] — 2026-09-11

- Subscription-Agenten prüfen lokale und entfernte Claude-/Codex-Konten jetzt anhand stabiler Provider-IDs und decken den vollständigen Prozesslebenszyklus mit realen Testprozessen ab.

## [1.0.25] — 2026-09-11

- BYOP vollständig entfernt; Claude- und Codex-Subscription-Routing isoliert Provider-Zugangsdaten und bereinigt Legacy-Einstellungen sowie Secrets.

## [1.0.24] — 2026-09-11

- **Gebundene Root-Passwortfreigabe:** Nur ein lokal vom Nutzer gestarteter Root-su-Befehl autorisiert genau die nächste Passwortbestätigung; Remote-Ausgabe, spätere Befehle, Abbruch, Sessionwechsel und Zeitablauf löschen die Freigabe.

## [1.0.23] — 2026-09-11

- **Sichere tmux-Installation:** SSH-Zaplexify behandelt Home-Verzeichnisse mit Leerzeichen korrekt und installiert tmux ohne vollständige Paketaktualisierung; ausführbare Asset-Tests sichern Pfade, Argumente, Integrität und macOS-Längenvertrag ab.

## [1.0.22] — 2026-09-11

- **Eingegrenzte Skill-Pfade:** Direkte Skill-Verweise werden vor dem Parsen kanonisiert und müssen innerhalb ihres vertrauenswürdigen Wurzelverzeichnisses bleiben; absolute Pfade, Elternsegmente und Symlink-Ausbrüche werden abgewiesen.

## [1.0.21] — 2026-09-11

- **Korrekte SFTP-Breadcrumbs:** Absolute und Root-Pfade behalten ihren führenden Slash; Rerenders verwenden stabile Klickziele und navigieren exakt zum dargestellten Segment.

## [1.0.20] — 2026-09-11

- **Gehärtete Entwicklungscontainer:** Linux-Container verwenden keinen SSH-Dienst oder Shared Login mehr; Toolchain-Downloads, Images und Hilfsquellen sind unveränderlich gepinnt und vor Ausführung kryptografisch geprüft.

## [1.0.19] — 2026-09-11

- **Sicheres Download-Überschreiben:** Bestätigte lokale Ziele werden atomar verdrängt und anhand des tatsächlich ersetzten Dateisystemobjekts geprüft; konkurrierend erneuerte Dateien werden wiederhergestellt statt überschrieben.

## [1.0.18] — 2026-09-11

- Lokale native Absturzdiagnose startet bei Opt-in idempotent, beendet und reapet den Minidump-Dienst bei Opt-out und wird durch einen echten Linux-Smoke-Test abgesichert.

## [1.0.17] — 2026-09-11

- OSS-Einstellungsschema verwendet den expliziten OSS-Kanal und die tatsächlich kompilierten Laufzeit-Features; ungültige Kanäle brechen ab.

## [1.0.16] — 2026-09-11

- **Atomare Release-Versionen:** Ein einziges validiertes Skript aktualisiert
  App-, Lockfile-, Bundle-, Installer- und Release-Dokumentation gemeinsam;
  Pull Requests prüfen dieselben Invarianten vor teuren Builds.

## [1.0.15] — 2026-09-10

- **Geschlossene Dependency-Lücken:** Betroffene HTTP/2-, TLS-, XML-, Git-
  und Nebenabhängigkeiten sind auf korrigierte Stände aktualisiert; der alte
  Hyper-/Rustls-Pfad des AWS-Clients wurde entfernt.
- **Dauerhafte Sicherheitsprüfung:** Pull Requests und ein wöchentlicher Lauf
  prüfen RustSec-Advisories; die immer laufende Vorprüfung erzwingt synchronen
  Lizenzumfang, exakte Git-Quellen und die bereinigte Dependency-Baseline.

## [1.0.14] — 2026-09-10

- **Verbindliche Clippy-Policy:** Build-relevante Pull Requests verweigern nun
  sämtliche Clippy-Warnungen; portable Zeitmessung und der gemeinsame
  Prozess-Wrapper beseitigen die zuvor geduldeten Zaplex-Verstöße.

## [1.0.13] — 2026-09-10

- **Effizientere Cockpit-Aktualisierung:** Wachsende Codex- und
  Claude-Transkripte werden für Sitzungs- und Aufgabenstatus nur noch ab dem
  geprüften Append-Offset verarbeitet; Truncate, Ersetzung und geänderte
  Prüfsummen lösen sicher einen vollständigen Neuaufbau aus.
- **Zwischengespeicherte Claude-Nutzung:** Unveränderte Transkripte werden für
  die Nutzungsanzeige nicht erneut eingelesen; ein begrenzter LRU-Cache hält
  ausschließlich destillierte Nutzungsdaten und wendet Zeitfenster bei jedem
  Refresh neu an.

## [1.0.12] — 2026-09-10

- **Reaktionsfähige Remote-Dateitransfers:** Datei-Chunks laufen außerhalb des
  Model-Threads mit begrenzter Parallelität und Warteschlange; Abbrüche räumen
  wartende Arbeit auf, während Schreibreihenfolge und Offsets pro Datei stabil
  bleiben.

## [1.0.11] — 2026-09-10

- **Verständlicher SSH-Fallback:** ControlMaster-Fehler erscheinen ohne
  doppeltes Präfix; nach einem fehlgeschlagenen oder inkompatiblen
  Remote-Server-Setup zeigt der Prompt den aktiven Standard-SSH-Modus statt
  dauerhaft „Shell wird gestartet…“.

## [1.0.10] — 2026-09-10

- **Speicherschonende Uploads:** Der Server-Dateibrowser streamt lokale Dateien
  unter Transport-Backpressure in begrenzten Chunks und erkennt Änderungen an
  der geöffneten Upload-Quelle vor dem Remote-Commit.

## [1.0.9] — 2026-09-10

- **Zuverlässigerer Start:** Das vorgewärmte SQLite-Ergebnis wird direkt über
  den Worker-Handle übernommen und kann auch bei sehr schnellem Abschluss nicht
  mehr durch ein Scheduling-Rennen verloren gehen.

## [1.0.8] — 2026-09-10

- **Flüssigeres Rendering:** Der Agent-Input hält bei der UI-Komposition keinen
  ungenutzten Terminal-Lock mehr; intrinsische SVGs werden nur einmal pro Größe
  gerastert und anschließend pointer-stabil wiederverwendet.

## [1.0.7] — 2026-09-10

- **Filemanager nach Serverwechsel:** Zeigt bei geänderter SSH-Serveridentität
  den neuen Fingerabdruck an und erlaubt erst nach ausdrücklicher Bestätigung,
  ausschließlich den Schlüssel dieses Hosts zu ersetzen.

## [1.0.6] — 2026-09-01

- **Agenten in SSH-Terminals:** `/agent` startet Claude Code oder Codex auf dem
  aktiven SSH-Host; lokale Installationen werden auch aus den üblichen
  macOS-GUI-Pfaden zuverlässig erkannt.
- **Sichere Sitzungswiederherstellung:** Nicht erreichbare, noch laufende
  Sessions bieten direkt im Hinweis eine fingerprint-geprüfte Stop-Aktion,
  ohne versehentlich eine zweite Sitzung zu starten.

## [1.0.5] — 2026-09-01

- **Verlässliche Einstellungen:** Schreibfehler werden bis zur UI propagiert;
  validierte Werte sind im Speicher, auf Disk und nach Neustart identisch.
- **Windows-Prozessabbruch:** Lokale Hilfskommandos laufen in eigenen Job
  Objects, sodass Abbruch auch Kind- und Enkelprozesse beendet.
- **Release-Tags:** Stable- und Prerelease-Tags nutzen eine gemeinsame,
  getestete Klassifikation; nur Stable benötigt Main und wird als Latest
  veröffentlicht.
- **Cockpit-Refresh:** Fehlende optionale Prozesssuche degradiert Nicht-Linux
  nicht mehr; überlappende Scans und OAuth-Abfragen werden zusammengeführt.
- **SSH-Verbindungsstart:** Pro Host läuft höchstens ein Bootstrap bis zum
  echten Lifecycle-Ende; Zsh-Prompts und absolute Shell-Ready-Fristen werden
  berücksichtigt.
- **Vertrauenswürdige HTTP-Header:** Client-, Release- und Systemmetadaten sowie
  Integration-Header bleiben auf der konfigurierten Zaplex-Origin begrenzt.
- **AWS-Credentials:** Nur der neueste, noch zur ausgewählten Konfiguration
  passende Refresh darf den aktiven Bedrock-Credential-Zustand verändern.
- **Begrenzte Portal-Flows:** Wayland-Screenshots und MCP-OAuth warten nicht
  mehr unbegrenzt und räumen abgelaufene Requests retry-fähig auf.
- **Atomare Linux-Updates:** AppImages werden auf demselben Dateisystem
  vorbereitet, vollständig synchronisiert und erst dann per Rename ersetzt.
- **Import-Isolation:** Drive-Importe tragen eindeutige Generationen; Reset und
  Retry können keine verspäteten Events eines früheren Imports mehr übernehmen.
- **Sichere Upload-Commits:** Remote-Pfadfehler schlagen geschlossen fehl und
  Staging-Objekte werden identity-geprüft atomar übernommen oder zur
  Wiederherstellung erhalten.
- **Sichere Downloads:** Downloads landen zunächst in exklusiven Sidecars;
  Symlinks und Spezialdateien werden sichtbar abgelehnt und bestehende Ziele
  erst nach vollständiger Prüfung atomar ersetzt.
- **Ehrliche Kontenerkennung:** Ein ausstehender oder fehlgeschlagener Scan wird
  nicht mehr als `KI-KONTEN 0` dargestellt; eine Null ist nur nach einem
  erfolgreichen leeren Scan sichtbar.
- **Cockpit-Abnahme:** Alle verbindlichen GH-160-Regressionen sind direkte,
  matrixgebundene Tests, einschließlich Verbindungen → Favoriten → Tab-Menü,
  Tree-Lebenszyklus, Statusdarstellung und Claude-Historie.
- **Laufzeit-Gate:** Sanitierte Zwei-Host-Belege können sicher an den exakten
  Build-Commit gebunden, von CI validiert und als prüfbares Artefakt bewahrt
  werden; ungültige oder mehrdeutige Quellen schlagen geschlossen fehl.

## [1.0.4] — 2026-08-20

- **Cockpit-Parität:** GitHub-Flows, Session-Restart und -Umbenennung,
  Command-Palette, hostbezogene Startverläufe und sichere Transcript-Ansichten
  schließen die verbliebenen Funktionslücken zu claudeplex und
  claudeplex-desktop.
- **Remote-Konten:** Claude- und Codex-Konten werden pro Host über opake
  Identitäten geroutet und vor Start, Lifecycle-Aktion und Transcript-Zugriff
  erneut geprüft.
- **Managed Agents:** Dauerhafte daemonverwaltete Agent-Sessions erhalten
  Start/Attach/Stop/Restart, kontrollierte RAM-Headroom-Prüfung sowie begrenzte
  und datensparsame Exit-Diagnosen.
- **Qualitätssicherung:** Ein ausführbarer Paritäts-Gate prüft lokale und
  entfernte Provider-, Host-, Konto-, Status-, Lifecycle- und Transcript-Fälle
  gegen frisch synchronisierte Referenz-Repositories.

## [1.0.3] — 2026-08-20

- **Cockpit-Sidebar:** Lokale Sessions bleiben immer sichtbar; verbundene
  Remote-Hosts erscheinen als Host–Projekt–Session–Agent-Tree und verschwinden
  wieder, sobald ihre letzte Zaplex-Verbindung geschlossen wird.
- **Verbindungen:** Host-Konfiguration, Favoriten und Connect/Disconnect bleiben
  ein eigenständiges kompaktes Sidebar-Element ohne doppelte Session-Anzeige.
- **KI-Konten:** Claude und Codex sind in Sidebar und Detailansicht als Provider
  eindeutig erkennbar; Konto- und Session-Erkennung ist host- und
  account-spezifisch abgesichert.
- **Parität:** Ein fail-closed Audit vergleicht Cockpit-Verhalten und Mockups mit
  frisch synchronisierten claudeplex- und claudeplex-desktop-Referenzen.
- **Remote-Startup:** Vertrauliche Startup-Befehle laufen nicht mehr als
  beobachtbare Eingabe durch die PTY.

- **Cockpit:** Account-Panes mit vorhandenen Sessions stürzen beim Layout der
  virtualisierten Sitzungstabelle nicht mehr ab.
- **Sidebar-Navigation:** Die aus „Verbindungen“ geöffnete Hostverwaltung zeigt
  im Panel-Header einen sichtbaren Rückweg zu „Verbindungen“; das Schließen des
  Panels bleibt davon klar getrennt.

## [1.0.1] — 2026-08-13

- **Sidebar:** Host-, Daemon- und Multiplexer-Zeilen reservieren die flexible
  Breite für ihre Identität; wiederholte Sekundäraktionen erscheinen als feste
  Icons mit lokalisierten Tooltips statt als platzraubende Textbuttons.
- **UI-Qualität:** Ein gemeinsamer `CompactRowAction`-Baustein, ein verbindliches
  HTML-Artefakt, Entwicklungsrichtlinien und ein billiger PR-Frühcheck schützen
  die migrierten kompakten Sidebar-Zeilen mechanisch vor Regressionen.

## [1.0.0] — 2026-07-30

- **Remote-Sitzungen:** versionsgebundener Host-Dienst, persistente PTYs,
  exaktes Reattach, Replay und idempotenter Shell-Bootstrap für Bash, fish und
  PowerShell.
- **Cockpit:** Host–Projekt–Session-Spine, mehrere Claude-/Codex-Konten,
  belastbare Nutzungs- und Kostendarstellung, explizite Loading-/Fehlerzustände
  und sichere Sitzungsaktionen.
- **SSH:** atomarer und referenzsicherer Credential-Lifecycle, OneKey-Editor
  mit Save/Discard/Cancel sowie strikte Endpunkt- und Host-Key-Prüfung.
- **Dateimanager:** MC-Tastatursteuerung, stabile Dateiidentitäten, sichere
  lokale und entfernte Operationen sowie eine gestreamte, fortsetzbare
  Transfer-Queue für lokale, entfernte und hostübergreifende Transfers.
- **Dateimanager-Sicherheit:** Remote SFTP-Mutationen behalten ihre Operation-ID
  über Verbindungsabbrüche, werden idempotent wiederaufgenommen und bewahren die
  Quelle, bis der daemonseitige Commit ausdrücklich bestätigt ist.
- **Oberfläche:** englische und deutsche Kernoberflächen, gemeinsame
  Modal-/Status-Komponenten und responsive Cockpit- und Dateimanager-Panes.
- **Markdown-Viewer:** extern geöffnete Markdown-Dokumente starten in einem
  eigenen Fenster ohne Sidebar und behalten am Dokumentende sichtbaren Abstand.
- **Agenten:** Antigravity ersetzt die eingestellte Gemini CLI; Claude Code,
  Codex, Grok und DeepSeek/CodeWhale liefern über lokale, selbst verwaltete
  Integrationen verlässliche Arbeits-, Freigabe- und Abschlusszustände.
- **Distribution:** konsistente Version `1.0.0`; das Apple-Silicon-DMG wird mit
  Developer ID signiert und über Apple notarisiert.

Work landed on top of the [Zap fork](https://github.com/zerx-lab/zap), starting from the native session-daemon merge ([PR #16](https://github.com/byte5ai/zaplex/pull/16)). Grouped by area; PR numbers are representative, not exhaustive — see `git log` for the full history.

- **Remote-session daemon**: persistent session IDs across app/SSH restarts, byte-exact replay ring buffer, multi-session support, idle-session garbage collection under a host-wide RAM ceiling (PR #16).
- **Cockpit / Conductor**: multi-account Claude + Codex discovery; real cost/heat from the Anthropic OAuth usage endpoint (#25); cross-host **Host ▸ Project ▸ Session** tree with "needs-me" bubbling (#60, #61, #78, #79); guardrails — pause/stop/kill per agent and stop-all (#83); review loop — diff → approve/redirect/commit/PR (#82); attention model with ambient dock badge + inbox (#80); spawn card with model/effort launch attribution (#81, #84); transcript viewer (parser + Markdown, `◇ log` verb) and live transcript watch (#48, #49, #63, #69); favorites and per-project grouping (#46, #67); account overrides — rename/recolor/reorder/hide (#47, #62); visual design pass unifying the cockpit language (#85); Conductor tree + favorites + spawn-card + CI gate integrated into the UX spine (#98).
- **Launch & routing**: launch-on-freest-account routing from the new-session menu, including on remote SSH hosts (C4-1…C4-4, #51–#54); session fork and fork-into-worktree — try another approach without disturbing the original session (#26).
- **File manager**: MC-style keyboard navigation + function-key bar, in-slot terminal⇄file-manager toggle (#27); F3/F4 view/edit over SSH, local and remote (#32, #33); F5/F6 copy/move across connections — local↔remote and remote↔remote via a local relay, with overwrite-conflict handling and a destination picker when more than one other pane is open (#28–#37).
- **GitHub flows**: quick-issue draft, PR review, and triage reachable from the launcher; the agent drafts the exact `gh` command, the user confirms before anything runs (C5, #45, #57, #66).
- **Host discovery**: Tailscale peers show up as ready-to-add SSH hosts; per-session RAM display in the daemon session list (#47, #68).
- **Fix/ask with your agent**: problem banners route to the user's own CLI agent instead of Zap's built-in AI ("Oz-repurpose" P1).
- **Localization**: German (`de`) locale added and completed for cockpit, sessions, launcher, SSH hosts, settings, and common UI chrome; English remains the fallback for the rest (#50, #71–#75).
- **Rebrand**: product identity renamed Zap → zaplex across the app (app name, install paths under `~/.zaplex/`, permission dialogs, panic/crash strings, CI quarantine hints).
- **CLI/UX polish**: CLI-agent detection reaches the UI reliably, self-update repoints to the zaplex release channel, uppercase Attention-Inbox hotkey (`cmd/ctrl-shift-O`), consistent launch-effort keys, waiting-state transitions keyed by stable host identity (#93–#97, #109).

## Zap — [Unreleased] (inherited at fork time)

- **AI / BYOP**: ported opencode's `applyCaching`, enabling prompt caching; `write_to_long_running_shell_command` now rejects embedded LF in line mode; the BYOP LRC monitor fallback moved to a silent subtask; fixed a sender leak in the 50 ms window of `cancel_execution` (#134 follow-up, #137)
- **Cloud strip-out, phase 1–2**: added a `cloud-disabled` channel predicate; removed billing/pricing, referral/reward, and cloud-sharing dialog UI; unsubscribed the RTC `UpdateManager`; retired the notebook/folder sync queue
- **Platform**: fixed a panic when launching macOS via Spotlight/Finder/Launchpad; `run_shell_command` stdout now falls back to the command grid
- **Infrastructure**: `.gitattributes` now forces LF; added a stale-issue bot and a Claude Code GitHub workflow
- **Editor**: code/Markdown viewers gained syntax highlighting for 15 more languages (Dart, Zig, SCSS, R, Julia, OCaml, Erlang, Nix, Groovy, Solidity, GraphQL, Protobuf, Clojure, Elm, CMake)

## [v2026.05.06.preview] — 2026-05-06

- **AI**
  - Integrated the DeepSeek CLI agent; improved LSP install reliability
  - LSP moved to a global `enabled_lsp_servers` setting; removed the `/index` command and the codebase-indexing runtime
  - `/plan` now faithfully reproduces Plan Mode (system prompt + hard tool guardrails)
  - Agent dynamic tool whitelist, `persist_conversations` setting, `ask_user_question` always asks under auto-approve
  - BYOP supports provider extra headers
- **Fixes**
  - `apply_file_diffs` schema changed from `const` to `enum` to accommodate Gemini
  - Root-caused the SSE stutter — genai gzip was off by default + workflow was split
  - Plan-folder notebooks are now created immediately in cloud-free environments
- **Branding**: logo and icons switched to a white background; BYOP mode hides the credits/billing UI

## [v2026.05.04.preview] — 2026-05-04

- **SSH Manager**: data layer + persistence + keychain landed; full UI/UX integration (panel + central pane + drag-and-drop + collapse + Connect + Command Palette)
- **AI**: distinguished the model's "no suggestion" output and refined the prompt system; BYOP history multimodal support extended to PDF/audio, opencode-style ERROR replacement; `UserQuery.context.images` kept alive end-to-end
- **UI**: title-bar search box can now be hidden; fixed contrast for keybinding-settings edit state and shortcut badges
- **i18n**: localized the remaining fixed strings in the main UI to Chinese; `/model` now defaults to `alt-shift-/`
- **Fixes**: Anthropic adapter now sends the 1M-context beta header by default; BYOP ToolCall emits a placeholder card on the first frame; the OpenAI-strict provider no longer echoes back `reasoning_content`
- **Infrastructure**: CI fix for the `.deb` build; enabled PR tests

## [v2026.05.03.preview(.2/.3/.4)] — 2026-05-03

- **Upstream sync**: merged a large batch of warp-upstream commits (cross-window tab drag, shell-script detection, IME cursor, remote-server init refactor, SSH remote-server auto-upgrade, cross-window tab drag, etc.); established `rerere` + a `zap-ours` merge driver; added a blocklist doc
- **AI / BYOP**: added a coercion layer for type-mismatched tool-parameter output; tightened the suspicious-backslash scan to eliminate false positives on `ls`/`diff`
- **i18n**: completed remaining Chinese localization (settings panel, etc.)
- **Website**: unified the GitHub URL to `zerx-lab/warp`; fixed mobile horizontal overflow
- **Fixes**: aligned the Windows taskbar ICO with the upstream format; restored NLD-in-terminal defaulting to true so Chinese input auto-routes to AI

## [v2026.05.02.preview] — 2026-05-02

- **AI / BYOP**
  - Completed the conversation-compaction loop — the `byop_compaction` module, settings persistence, auto-prune, overflow pass-through — a 1:1 port of opencode's behavior
  - Moved reasoning effort from provider settings to the input-box picker
  - Wired multimodal attachment support into the BYOP path
  - Local BYOP webfetch/websearch integrated with Exa
  - System-prompt templates now selected by model identifier; added several new templates
- **Privacy / cloud strip-out**
  - Physically removed P4 easily-strippable dead code (`anonymous_id` / `EXPERIMENT_ID_HEADER` / settings sync / `app_focus`)
  - Cut four closed-source outbound channels: telemetry, Sentry, `anonymous_id`, settings sync
  - Flipped three privacy toggles' defaults from true to false
  - Two cleanup passes on `cloud_conversations` (UI / privacy / FeatureFlag / AIClient / cargo feature)
- **Refactor**: removed blocklist AI-response scoring and its tracking; removed `agent_attribution` and the Oz changelog toggle
- **CI**: weekly builds now cut a formal release with normalized tags

## [v2026.05.01.preview] — 2026-05-01

- **Cloud strip-out**: physically removed 6 cloud LLM tools + `child_agent` + orchestration; physically removed the share-modal trio and the billing-denied modal; website switched to a monochrome logo
- **AI**
  - Wired Workflow Autofill into BYOP one-shot
  - BYOP LRC keeps injecting context on later turns + hardened sanitization + control-key tokens
  - Chat stream now surfaces remote-login session hints and reasoning pass-through
  - Refined genai error mapping into Stream / Other variants
  - Chat-stream adapter: fixed `ToolCall` `None` handling
- **Platform**: `warpui_core` avoids rescanning system fonts; sync commands now unconditionally disable the pager, using `PAGER=cat` to preserve the real exit code
- **Website**: full site component and i18n refactor, synced with Tailwind and global styles

## [v2026.04.30.oss] — 2026-04-30

- **CI**: renamed the `preview` channel to `oss`; fixed Windows/macOS build failures
- **Refactor**: removed leftover `cloud_mode` code and settings

## [v2026.04.30.preview] — 2026-04-30

First preview release of the Zap community fork.

- **Branding & positioning**: renamed Zap, redesigned the logo, community-fork README
- **BYOP**
  - Replaced `async-openai` with `genai`, supporting 5 natively-bound protocols
  - Providers sub-page + a models.dev data source + a quick-add search box
  - Trimmed the prompt templates
- **Decentralization cleanup**: removed the `UseComputer` / `RequestComputerUse` tools, the Drive "Create team" / "Join team" entry points, and referral-related code
- **i18n**: Fluent infrastructure + 12 translated `settings_view` files; completed i18n for the ai / features / teams pages
- **Website**: new BYOP landing page (Astro + Tailwind, bilingual EN/ZH); responsive improvements
- **AI**: CJK input classification, reasoning split out, BYOP `tool_call` diagnostics, LRC tag-in synthesizes a virtual subagent + floating spawn flow
- **CI**: Release workflow explicitly declares `contents: write` permission, fixing a 403

[Unreleased]: https://github.com/byte5ai/zaplex/compare/v1.0.12...HEAD
[1.0.12]: https://github.com/byte5ai/zaplex/compare/v1.0.11...v1.0.12
[1.0.11]: https://github.com/byte5ai/zaplex/compare/v1.0.10...v1.0.11
[1.0.10]: https://github.com/byte5ai/zaplex/compare/v1.0.9...v1.0.10
[1.0.9]: https://github.com/byte5ai/zaplex/compare/v1.0.8...v1.0.9
[1.0.8]: https://github.com/byte5ai/zaplex/compare/v1.0.7...v1.0.8
[1.0.7]: https://github.com/byte5ai/zaplex/compare/v1.0.6...v1.0.7
[1.0.6]: https://github.com/byte5ai/zaplex/compare/v1.0.5...v1.0.6
[1.0.5]: https://github.com/byte5ai/zaplex/compare/v1.0.4...v1.0.5
[1.0.4]: https://github.com/byte5ai/zaplex/compare/v1.0.3...v1.0.4
[1.0.3]: https://github.com/byte5ai/zaplex/compare/v1.0.1...v1.0.3
[1.0.1]: https://github.com/byte5ai/zaplex/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/byte5ai/zaplex/releases/tag/v1.0.0
[v2026.05.06.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.04.preview...v2026.05.06.preview
[v2026.05.04.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.03.preview.4...v2026.05.04.preview
[v2026.05.03.preview(.2/.3/.4)]: https://github.com/zerx-lab/warp/compare/v2026.05.02.preview...v2026.05.03.preview.4
[v2026.05.02.preview]: https://github.com/zerx-lab/warp/compare/v2026.05.01.preview...v2026.05.02.preview
[v2026.05.01.preview]: https://github.com/zerx-lab/warp/compare/v2026.04.30.oss...v2026.05.01.preview
[v2026.04.30.oss]: https://github.com/zerx-lab/warp/compare/v2026.04.30.preview...v2026.04.30.oss
[v2026.04.30.preview]: https://github.com/zerx-lab/warp/releases/tag/v2026.04.30.preview
