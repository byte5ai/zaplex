# Native UI-/UX-Prüfmatrix

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).
Diese Matrix ist vor dem **nächsten nativen Lauf** festgelegt. Sie macht die
bereits früher ausgeführten Funktionstests nicht rückwirkend zu UX-Tests.

Prüfziel und Referenzstand: `6c956383ad560913dc31fa383104dc489104d561`.
Für die vollständige DMG-/Explorationsabnahme liegt noch kein Ergebnis vor;
die Fallspalte bleibt dafür `not-run`. Die getrennte
[Altbuild-Auswertung](README.md) verwendet v1.2.0-rc1.
Ergänzend dokumentiert die [native Fixture-Auswertung](NATIVE-FIXTURES.md)
Teilbelege und Fehler auf `1201fc4f3cafcbab7464343cad8b715d8cf3db2e`
sowie die erneute Sichtprüfung der beiden Fehler auf
`cadcb19130bbe0630f39bdd61b4e183ff154981a` (Paketversion 1.2.2).
Die [Nachprüfung der Workspace-Korrekturen](NATIVE-FIXTURES.md#workspace-korrekturen-nativ-bestätigt-am-20102026)
auf `1800ec77` (1.2.4) bestätigt vier native Fälle mit 30 visuell geprüften
Originalaufnahmen. Der [separate Agentenfall](NATIVE-FIXTURES.md#agentenkorrektur-nativ-bestätigt-am-20102026)
besteht nach PR #589 auf `513647be` (1.2.6), mit 24 Zustands-/Größenkombinationen
und drei visuell geprüften Originalaufnahmen. Die fünf erfolgreichen Fälle
stammen aus zwei unterschiedlichen Quell-/Testständen.
Die [SSH-Formular-Nachprüfung](SSH-FORM-2026-10-21.md) auf `dd2ce0f9` (1.2.8)
bestätigt die Korrekturen #593/#594/#595 in zwei nativen Fällen (DE/EN), mit
22 visuell geprüften Originalaufnahmen. Die benachbarte schmale Terminal-Pane
zeigte abgeschnittene Einführungstexte (#601). Die Korrektur ist mit PR #603
gemergt; der Nachprüfungslauf 39753074448 erreichte wegen des 30-Minuten-Buildlimits
keine Tests. Der [gezielte native Nachweis](TERMINAL-ENTRY-2026-10-22.md) auf `75227a1` (1.2.15) besteht inzwischen in DE/EN, Hell/Dunkel und 1024/760 Punkten einschließlich Ausblenden; zwölf Originalbilder enthalten zusätzlich den erfolgreichen lokalen Header-Klick. Die [lokale Linux-Diagnose](LINUX-DIAGNOSIS-2026-10-22.md) bestätigt inzwischen DE/EN in Hell/Dunkel bei 1024/760 Punkten samt Ausblenden nach explizitem Terminal-Fokus; sie ersetzt den macOS-Nachweis nicht.
Die [Dateimanager-Exploration vom 22.10.](EXPLORATION-2026-10-22.md) am
unveränderten 1.2.2-DMG ergänzt Tastatureinstieg, Unterordner-Navigation und
Umbenennen-Dialog; der wirkungslose Ordner-Klick wird in #604 untersucht.
Die [abgeschlossene Dateiaktionsprüfung](FILE-ACTIONS-2026-10-22.md) ergänzt F2-Abbruch, F7, Elternnavigation und F4 mit Inhaltsvergleich; sie findet den pfadabhängigen Rename-Fehler #614 sowie #615/#616/#617.
Die [Settings-Exploration](SETTINGS-2026-10-22.md) ergänzt Suchtreffer/Leerzustand, Dark→Light, gültige Schriftgrößenänderung und erhaltenen Shellmarker; ungültige Schriftgröße ohne Rückmeldung ist #622. Die Korrektur aus PR #625 besteht lokal in DE/EN mit zwölf kontrollierten Originalbildern ([Nachweis](FONT-VALIDATION-2026-10-22.md)); macOS besteht ebenfalls in DE/EN ([30 native Bildnachweise für Dateiaktionen und Schriftgröße](NATIVE-ACTIONS-FONT-2026-10-22.md)).
Die [Pane-Exploration vom 23.10.](PANES-2026-10-23.md) ergänzt rechte/untere lokale Splits, unabhängige Shells, CWD-/Titelwechsel und erhaltene Quellshell; Escape im Kopfzeilenmenü ist #629.
Die [Sichtprüfung auf 1.2.27](CANDIDATE-1.2.27-NATIVE.md) umfasst 44 einzeln
betrachtete native Originale auf `70d6b1f8f`: Der Filterhinweis bleibt nach
Dark→Light in DE/EN lesbar (#644), der lokale inaktive Header-Einstieg erhält
die Nachbar-Pane (#645). Echte Remote-Varianten, eingegebene Editorentwürfe
über Themewechsel und die installierte DMG-Abnahme sind davon nicht abgedeckt.
Weder Altbuild noch Fixtures ersetzen die vollständigen Fallkombinationen.
Die [aktuelle Sichtprüfung auf 1.2.23](CANDIDATE-1.2.23-NATIVE.md) umfasst 42 einzeln
betrachtete Originalaufnahmen desselben Quellstands: Agentenzustände, gefüllte und
leere Verbindungen, Favoriten, Dateimanager DE/EN in Hell/Dunkel und Pane-Drag.
Die [installierte DMG-Abnahme und reale Exploration von 1.2.23](CANDIDATE-1.2.23-EXPLORATION.md) ergänzen 15 gemeinsam bestandene Pflichtfälle, Remote-Wiederverbindung, beide CLI-Testprofile und eine inhaltlich bestätigte Remote→Lokal-Kopie. Sie finden zusätzlich die Alias-Störung #639 und die veraltete Zielansicht #640; vollständige Fallkombinationen werden daraus nicht abgeleitet.
Die [native Sichtprüfung des Kandidaten 1.2.25](CANDIDATE-1.2.25-NATIVE.md)
ordnet 42 einzeln betrachtete Originalaufnahmen demselben Quellstand zu. Sie
ergänzt den Themewechsel-Befund #644. Derselbe Kandidat besteht anschließend
alle 15 installierten GUI-Pflichtfälle. Die reale Exploration bestätigt die
Alias-Verbindung und eine unmittelbar aktualisierte Transferzielansicht, findet
aber zusätzlich #645 (falsche Pane-/Host-Zuordnung) und #646 (Download über einen
lokalen Verzeichnisalias). Details und Aussagegrenzen stehen im Kandidatenbericht.

Die [Abnahme auf 1.2.32](CANDIDATE-1.2.32-NATIVE.md) ergänzt einen vollständig
erfolgreichen CI-Batch mit 15/15 installierten Pflichtfällen, DE/EN-Teilnachweise
für Theme-Fokusrückgabe und die sichere Verzeichnisübernahme bei einer lokal
belegten Shell. Sie findet den initialen F2/F7-Dialogfokusfehler #673; die
Gesamtmatrix bleibt ohne vollständige Fallkombinationen offen.

Die [installierte Nachprüfung von 1.2.34](CANDIDATE-1.2.34-NATIVE.md) ergänzt
alle 15 lokalen GUI-Pflichtfälle am selben DMG aus Original- und gezieltem
Fortsetzungslauf, Dialog-Anfangsfokus in EN/DE und Hell/Dunkel sowie lange
Favoritenhinweise in beiden Öffnungsrichtungen (EN/Light und DE/Hell/Dunkel).
Der deutsche Erkennungshinweis besteht nach vollständigem Neustart in beiden
Themes. #684 (erster Escape bei ausgewähltem Namen) ist lokal korrigiert und
mit PR #687 als 1.2.35 gemergt, aber noch nicht nativ nachgeprüft. #685 wurde
nach erfolgreichem horizontalem Scrollen bei 350/500 Punkten auf die praktisch
unbedienbare erreichbare Größe 100×100 eingegrenzt. Weitere englische Texte
in deutschen Agent-Einstellungen sind #688. Die Gesamtmatrix bleibt offen.

## Referenzen und Bewertung

- **C**: [Cockpit-Spezifikation](../../docs/ui/cockpit-sidebar-connections.html#spec),
  ergänzt durch [GH-160 PRODUCT](../GH-160/PRODUCT.md) und [TECH](../GH-160/TECH.md).
- **L/J/N**: [Startmenü, Pane-Start und Tab-Zusammenführung](../../docs/ui/pane-launch-and-tab-join.html#spec).
- **B+**: [Sessionbaum](../../docs/ui/cockpit-sessions-tree.html).
- **U**: Aufgaben und Verständlichkeitskriterien aus #564; für allgemeine
  Einstellungen gibt es in diesen Referenzen keine eigene freigegebene Detailansicht.
  Dort wird Bedienbarkeit bewertet, keine erfundene Pixelparität.

Die Vorränge aus [docs/ui/README.md](../../docs/ui/README.md) gelten für den
Sollvergleich: getrennte Sessions/Konten und B+ vor dem älteren Gesamtbild;
explizite F2-Vorgabe mit acht Dateiaktionen vor den vier Aktionen im alten PNG.
Konkrete Demofarben, Demokonten und simulierte Daten sind keine Sollwerte.

`pass` bedeutet: die **gesamte Erwartung einer konkreten Fall-/Variantenkombination**
wurde beobachtet. `fail` benötigt Soll/Ist, Reproduktion, Beleg und Korrekturissue.
`not-run` benötigt einen Grund. Sichtprüfung, Funktion und explorative Bewertung
werden getrennt erfasst. Verdeckte Pixel, Quellcode, OCR-Erfolg oder ein alter
Screenshot können fehlende native Beobachtung nicht ersetzen.

## Varianten und Belegformat

Grunddurchlauf: Deutsch, dunkles Theme, normale Fensterbreite; tatsächliches
Fensterrechteck, Sidebar- und Pane-Breiten sowie Screenshot-Auflösung protokollieren.
Die native Mindestbreite erst messen, nicht mit einer halben Bildschirmbreite
gleichsetzen. Anschließend alle erreichbaren Flächen in folgenden Varianten:

| Variante | Unabhängige Erwartung |
|---|---|
| Englisch | Beschriftungen, Tooltips, Fehlermeldungen und Tastaturzugang bleiben verständlich. |
| Hell / Dunkel / Kontrast | Hierarchie, Fokus, Auswahl und deaktivierte Aktionen bleiben unterscheidbar; keine unlesbaren Texte. |
| Native Mindestbreite | Kein Verlust von Aktionen oder Anleitungen; alle Dialoge und Menüs bleiben erreichbar. |
| Sidebar 250 / 320 / 480 Punkte, DE und EN | Lange und fehlende Titel, Recovery-Metadaten und feste Aktionsslots gemäß C:A5. |
| Ein Pane / zwei Panes / verschachtelte Panes | Fokus und Hostkontext eindeutig; F-Leiste mit einer, zwei und vier Reihen tatsächlich erreichen. |
| Lange Identitäten und Texte | Unterschiedliche lange Host-, Projekt-, Session-, Konto- und Dateinamen; volle Identität über Tooltip erreichbar. |
| Reduced Motion aus / an | Wartepuls über mindestens zwei vollständige Zyklen aufnehmen; bei aktivierter OS-Einstellung statischer Halo, kein Puls. |
| Tastatur / Maus | Fokus sichtbar, Reihenfolge sinnvoll, Enter/Escape/Zurück und Fokusrückgabe wirksam; Hover ohne Geometriesprung. |

Jede Variantenkombination erhält beim Lauf einen eigenen Datensatz
`Fall-ID / Variante / status / Grund / Beleg-IDs`; keine Sammelfreigabe aus einem
einzigen Theme. Belege enthalten App-Version, DMG-SHA256, Build-Commit,
Test-/Referenzrevision, Zeitpunkt, Fensterrechteck, Locale, Theme und
Reduced-Motion-Zustand. Pro Interaktion: sichtbarer Zustand vorher, ausgeführte
Aktion, sichtbarer Zustand nachher, Erwartung und Beobachtung. Videos für
Animationen, Drag, Fokusübergänge und Recovery. Keine persönlichen Profile.

## Fälle

Gründe: **V** = vollständige Fall-/Variantenkombination noch nicht geprüft; **G** =
interaktive Durchführung noch nicht erfolgt (Zugang seit 21.10.2026 praktisch nachgewiesen); **R** = weitere Remote-Fallvarianten
noch nicht durchgeführt; **P** = Provider-/Kontovarianten noch nicht geprüft,
bei Abo-Funktionen sind weiterhin echte Logins erforderlich.
Der isolierte Remote-Zugang und beide API-Testprofile sind seit der
[Remote-Teilabnahme](REMOTE-2026-10-21.md) praktisch nachgewiesen.
Das aktuelle DMG ist vorhanden und installiert geprüft (zwölf lokale Tests im ersten Lauf und die drei übrigen nach Testkorrektur bestanden; siehe [Nachweis](NATIVE-FIXTURES.md#gezielte-fortsetzung-und-installiertes-dmg)).
Diese Teilbelege erfüllen keine gesamte Fall-/Variantenkombination; deshalb V.
Die [erste direkte Exploration](EXPLORATION-2026-10-21.md) liefert Teilbelege und neue Fehler.
G bezeichnet jetzt ausstehende Durchführung, keine fehlende lokale Infrastruktur.
Fehlende Testvoraussetzungen sind keine Produktfehler.

| ID | Sollquelle | Nutzeraufgabe / Zustand | Erwartete Darstellung und Interaktion | Nachweis | Aktueller Status / Grund |
|---|---|---|---|---|---|
| V01 | C:C3, A6; U | Ohne eingerichteten Host starten | Arbeitsfläche, vorhandene horizontale Navigation und nächster Schritt erkennbar; keine überdeckten Texte oder dekorativen Farbränder | Gesamtansicht und Navigation | Teilnachweis pass — Einführung nach #601 auf 1.2.15 nativ DE/EN, Hell/Dunkel, 1024/760 samt Ausblenden geprüft ([Beleg](TERMINAL-ENTRY-2026-10-22.md)); Gesamteinstieg/übrige Varianten V, G |
| V02 | C:A4, A6; U | Alle sichtbaren Icons erkunden | Verständliche lokalisierte Tooltips, stabile Geometrie, klare aktive/inaktive Zustände | Hover-/Fokusfolge pro Icon | Teilnachweis — installiertes 1.2.27 DE/dunkel: Verbindungen- und Stern-Tooltip pass; fehlende Toolbar-/Registry-Tooltips fail #664, vollständige Folge und weitere Varianten V, G ([Beleg](CANDIDATE-1.2.27-CONNECTIONS.md)); aktueller Teilnachweis: 1.2.32 DE/Light: Ordner-/Server-/Ausklapp-Tooltips und Favoriten-Stern geprüft; vollständige Iconfolge bleibt ausstehend ([1.2.32](CANDIDATE-1.2.32-NATIVE.md)) |
| V03 | U | Per Tastatur navigieren und zurückkehren | Sinnvolle Fokusreihenfolge; sichtbarer Fokus; Escape/Zurück und Rückgabe zum Auslöser | Tastaturablauf | Historischer Teilbefund #616 auf 1.2.2; Korrektur 1.2.19 nativ DE/EN pass ([Beleg](NATIVE-ACTIONS-FONT-2026-10-22.md)); übrige Flächen/Varianten V, G |
| C01 | C:C1; U | Leere Verbindungsliste verstehen | Vollständige Anleitung einschließlich Ordner- und Plus-Aktion, auch schmal | Leerzustand DE/EN, 250/320/480 | Teilnachweis pass — native 1.2.4, DE/EN, Hell/Dunkel, 250/320/480 (#565); installiertes DMG der Korrektur V |
| C02 | C:C1; U | Ordner und Host anlegen/bearbeiten | Eindeutige Feldnamen, Validierung, Speichern/Abbrechen; keine doppelte Konfiguration | Formular leer/ungültig/gültig | Teilnachweis pass — installiertes 1.2.27 DE/dunkel: Ordner/Host anlegen, ungültiger Host, Korrektur, gültiges Speichern, bestehenden Editor wiederverwenden ohne Duplikat; Formular-Abbruch und übrige Varianten V ([Beleg](CANDIDATE-1.2.27-CONNECTIONS.md)) |
| C03 | C:C1, A1, A2 | Konfigurierten Host finden/favorisieren | Identität priorisiert; feste Stecker-/Sternslots; Offline-Host bleibt hier sichtbar | Lange Registry-Zeilen, Sternwechsel | Teilnachweis — installiertes 1.2.27 DE/dunkel: Sternwechsel und Favoriten/Weitere-Hosts-Projektion pass; fehlender vollständiger Registry-Tooltip fail #664, übrige Varianten V, G ([Beleg](CANDIDATE-1.2.27-CONNECTIONS.md)); aktueller Teilnachweis: 1.2.32 DE/Light: vollständige lange Registry-Identität und Sternwechsel bestanden ([1.2.32](CANDIDATE-1.2.32-NATIVE.md)) |
| C04 | C:C1, C4; U | Verbinden, Fehler erkennen, abbrechen | Lade-/Fehlerzustand mit verständlichem nächstem Schritt; kein festhängender Dialog | Echte Verbindung und definierter Fehler | Teilnachweis — installiertes 1.2.27: EN/dunkel persistente Remote-Shell/SFTP ([Remote](CANDIDATE-1.2.27-REMOTE.md)); DE/dunkel echter Verbindungsfehler, Mausabbruch/Retry/Rückweg pass, angezeigtes Escape fail #661; übrige Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-CONNECTIONS.md)); aktueller Teilnachweis: 1.2.32 DE/Light: Enter erreicht nach echtem Connection refused einen unabhängig bestätigten Listener; Escape zeigt den Abbruchzustand ([1.2.32](CANDIDATE-1.2.32-NATIVE.md)) |
| C05 | C:C4–C7, A5 | Vorhandene Session wiederfinden | Recovery-Hierarchie, aktuelle Identität, Metadaten, Offen-Markierung und Reihenfolge stimmen | Inventar und Klick auf Session | not-run — V, G, R |
| C06 | C:C6; U | Session beenden / Löschdialog abbrechen | Ziel und laufender Agent verständlich; Abbruch erhält Session; Bestätigung wirkt nur auf das benannte Ziel | Isolierte Session, Dialogfolge | not-run — V, G, R, P |
| S01 | C:C3, T2 | Zwischen KI-Sessions und KI-Konten wechseln | Getrennte Einträge und Scrollbereiche, volle Sidebarhöhe; lokaler Leerzustand bleibt ehrlich | Beide Ansichten, leer und gefüllt | not-run — V, G, P |
| S02 | C:T1, T3, T5; B+ | Projekt und Agent finden | B+-Verdichtung für 1/mehrere PTYs und 1/mehrere Agents; keine leeren Ebenen | Alle vier Baumkonstellationen | not-run — V, G, P |
| S03 | C:T6–T8; B+ | Lange/gleiche Sessiontitel unterscheiden | Einrückung, gekürzter gemeinsamer Präfix, mindestens acht unterscheidende Zeichen, voller Tooltip, Managed-Marke | Lange Geschwistertitel, Fokus, Hover | not-run — V, G, P |
| S04 | C:T4, S3 | Auf einen Agenten zugreifen | Jeder sichtbare/countbare Ziel-Agent ist erreichbar; Auswahl und kollabierter Vorfahr behalten Akzent | Klick, Kollaps, Inbox/Palette-Abgleich | not-run — V, G, P, R |
| S05 | C:S1–S3 | Wartenden Agenten erkennen | Nur wartende Agents amber; Zahlen konsistent; Puls 1,6 s, Ring bis etwa 2×; Reduced Motion mit statischem Halo | Video und Zustandsfolge gesehen/ungesehen/blockiert | not-run — V, G, P |
| A01 | C:I1, I2, P5 | Konto-Details öffnen und erneut wählen | Provider/Konto einmalig; ergänzt/fokussiert Details, übrige Panes bleiben erhalten | Vorher/Nachher und erneuter Klick | not-run — V, G, P |
| A02 | C:I2, A3 | Verbrauch und Herkunft verstehen | 5 h/Woche untereinander, Reset direkt unter Spur; Schätzlegende und Herkunft; wartende Session dezent markiert | Echte Kontodaten mit isoliertem Profil | not-run — V, G, P |
| A03 | U; GH-160 | Agent/Konto/Modell auswählen | Ohne CLI, ohne Anmeldung und bei mehreren Konten ehrliche Zustände und gangbarer nächster Schritt | Startkarte leer/geladen/Fehler/Abbruch | not-run — V, G, P |
| L01 | L1, L2, L5 | Plus und vier Split-Menüs öffnen | Gleiche gemeinsame Einträge/Reihenfolge; Tabzusätze nur bei Plus; Menü vor Screenshot sichtbar | Fünf offene Menüs | Teilprüfung — Plus, Rechts/Unten sichtbar; historisch gekürzter Favoritenhinweis #566 im 1.2.2-DMG; übrige Einstiege/Varianten V, G ([Beleg](PANES-2026-10-23.md)) |
| L02 | L2; U | Leere Favoriten verstehen | Vollständige Anleitung zur Favorisierung sichtbar, auch am Fensterrand | DE/EN, normale/schmale Breite | Teilnachweis pass — native 1.2.4, DE/EN, Hell/Dunkel (#566); übrige Einstiege, Randlagen und DMG V |
| L03 | L3, L4; C:C2 | Favorit bzw. weiteren Host wählen | Name verbindet einmal; nur Nichtfavoriten im Untermenü; Stern-Änderung konsistent | Registry + Menü + echte Verbindung | Teilnachweis pass — installiertes 1.2.27 DE/dunkel: Favorisieren/Entfernen projiziert denselben Host zwischen Favoriten und Weitere Hosts; Weitere Hosts öffnet einen neuen Verbindungsversuch bis zum tatsächlichen Connection refused. Direkter Favoritenstart, erfolgreiche Verbindung/übrige Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-CONNECTIONS.md)); aktueller Teilnachweis: 1.2.32 DE/Light: primärer Favoritenname startet separat einen echten Verbindungsversuch; Aktionsmenü verdeckt weiterhin den Identitätshinweis (#672) ([1.2.32](CANDIDATE-1.2.32-NATIVE.md)) |
| L04 | L4; C:C2 | Separates Favoriten-Flyout bedienen | Ellipse verbindet nicht; seitliches Flyout hält Elternmenü sichtbar; Randplatzierung/Fokus/Zurück stimmen | Maus und Tastatur an beiden Rändern | Teilnachweis — installiertes 1.2.27 DE/dunkel: Ellipse öffnet seitliches Flyout bei sichtbarem Elternmenü; Bearbeiten fokussiert bestehenden Editor. Voller Hosttooltip teilweise vom Flyout überdeckt; Randlagen, Tastatur/Fokus/Zurück und übrige Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-CONNECTIONS.md)) |
| L05 | L6; C:P2 | Split per Escape/Außenklick abbrechen | Keine neue Pane/Session; ursprüngliche Shell und Fokus bleiben | Vorher/Nachher mit Shellzustand | Teilnachweis pass — 1.2.23 EN/dunkel: Escape schließt das tatsächliche Kopfzeilenmenü, ursprüngliche PID/Marker/CWD bleiben; Außenklick/weitere Varianten V, G ([Beleg](CANDIDATE-1.2.23-EXPLORATION.md)) |
| L06 | L7–L9 | Agent in erfasster Pane starten | Genau eine Session an erfasster Position; Mehrfachkonto und verschwundenes Ziel mit Erklärung, kein falscher Start | Kontowahl, Fokuswechsel, ungültiges Ziel | not-run — V, G, P, R |
| P01 | C:P2 | Lokal rechts/links/oben/unten teilen | Neue unabhängige Shell in jeder gewünschten Richtung; Quellshell bleibt | Vier Geometrien und unabhängige Ausgabe | Teilnachweis pass — 1.2.23 EN/dunkel: alle vier Geometrien der automatisierten DMG-Fälle visuell geprüft, neue und ursprüngliche Shellmarker verschieden; weitere Varianten und direkte Vier-Richtungs-Exploration V, G ([Beleg](CANDIDATE-1.2.23-EXPLORATION.md)) |
| P02 | C:P2, P3 | Mehrhost-Panes nebeneinander nutzen | Host/CWD/Startziel pro Pane, kein Routing über zufälligen Fokus | Zwei echte Hosts, asynchroner Start | not-run — V, G, R |
| P03 | C:P3; N1 | CWD, Fokus und Tabtitel wechseln | Host·Verzeichnis korrekt; expliziter Tabtitel gewinnt, Entfernen stellt Automatik wieder her | Wechsel und kollidierende Namen | Teilnachweis pass — lokale CWD-/Fokus-/automatische Titelwechsel sowie unterscheidende Suffixe bei gleichem CWD; expliziter lokaler Titel gewinnt bei CWD-Wechsel, Löschen stellt Automatik wieder her; Remote/übrige Varianten V, G, R ([bisheriger Beleg](PANES-2026-10-23.md), [native Ergänzung](CANDIDATE-1.2.27-PANES.md)) |
| P04 | C:P4, P5 | Pane verschieben/abtrennen | Dieselbe Pane ohne neue PTY; gültige Drop-Zonen; Abbruch erhält Ausgangslayout | Header-Drag in vier Richtungen/Tab | Teilnachweis pass — unverändertes 1.2.27 EN/dunkel: Headerfokus, alle vier Umordnungsrichtungen und Abtrennen vor/nach Tab-Join mit frischen Shell-PID-/Marker-/CWD-Nachweisen. #665 war eine Operatorsequenz ohne vorheriges move; Abbruch, ungültige Ziele, Remote/übrige Varianten V, G, R ([Gegenprobe](CANDIDATE-1.2.27-HEADERS.md)) |
| P05 | J1–J8 | Ganzen Tab als Split einfügen | Ziel bleibt beim Drag sichtbar; Layout/Identitäten bleiben, Quelltab entfällt; Selbst-/ungültiger Drop wirkungslos | Drag/Abbruch, verschachtelte Panes | Teilnachweis pass — einfacher und verschachtelter Tab-Drag, Quellfokus/-entfall, drei Shell-Identitäten und Selbst-Drop; Sichtbarkeit während Drag, Abbruch, ungültige Ziele/übrige Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-PANES.md)) |
| P06 | C:P5, R1 | Fenster/Session wiederherstellen | Geometrie, Fokus, Modus, Entwurf und richtige Remote-Identität erhalten; provisorischer Start nicht ins andere Fenster verschiebbar | Neustart/Restore + Drag während Start | not-run — V, G, R |
| F01 | C:F1, P3 | Dateimanager öffnen/zur Shell zurück | Pane-Kontext und Shellzustand bleiben; F10/Leistenaktion/Pane-X kehren zurück | Drei Rückwege, frischer Shellmarker | Teilnachweis pass — installiertes 1.2.27 EN/dunkel: inaktiver Remote-Header öffnet eigenen Host/CWD neben unverändertem lokalem Dateimanager; F10 erhält lokale und Remote-PID/Marker/CWD. Andere Rückwege/Varianten V, G ([Beleg](CANDIDATE-1.2.27-REMOTE.md)) |
| F02 | C:F2 | Dateiaktionen bei wenig Platz finden | F2–F8/F10 mit kurzer Beschriftung; eine/zwei/vier gleich breite Reihen; keine versteckten Aktionen | Breitenfolge, alle acht Zellen | not-run — V, G |
| F03 | C:F2, F5 | Fokus zwischen Dateipanes wechseln | Nur kompatible fokussierte Pane aktiviert Aktionen; Cursor und Markierung unterscheidbar | Zwei Panes mit gleichem Dateinamen | not-run — V, G |
| F04 | C:F4 | In Elternverzeichnis zurückgehen | Verlassener Ordner wird nach Identität ausgewählt und sichtbar gescrollt | Verschachtelte, sortierte Liste | Teilnachweis pass — 1.2.2 EN/Dark: Doppelklick hinein, Hoch-Button zurück wählt verlassenen Ordner; Enter nach Dialogabbruch auf 1.2.19 nativ DE/EN korrigiert (#616), Scroll-/Sortier-/weitere Varianten V, G ([Beleg](FILE-ACTIONS-2026-10-22.md)) |
| F05 | C:F3 | Transferziel bewusst wählen | Dauerhafte Zielzeile; eindeutiger Host/Pfad/Pane; eigener Tab zuerst; Quelle bei F5/F6 erkennbar | Mehrere mögliche Ziele, gleiche Namen | Teilnachweis pass — 1.2.23 EN/dunkel: explizite lokale Zielpane in anderem Tab für echte Remote-Kopie gewählt; gleiche Namen und weitere Varianten V, G, R ([Beleg](CANDIDATE-1.2.23-EXPLORATION.md)) |
| F06 | C:F3 | Ziel-Pane ändern/schließen | Gewähltes Ziel folgt deren Verzeichnis; nach Schließen/Moduswechsel neue Wahl, nie stiller Ersatz | Ziel ändern, dann Copy/Move | Teilnachweis pass — installiertes 1.2.27 EN/dunkel: Ziel folgt Verzeichniswechsel; Rückkehr des Ziel-Dateimanagers zur Shell invalidiert die Wahl trotz dritter kompatibler Dateipane. F5 verlangt explizite neue Wahl; Abbruch lässt Alternative leer. Tatsächliches Tab-Schließen/weitere Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-LOCAL-MULTI.md)) |
| F07 | C:F5 | Dateien/Ordner markieren | Alle definierten Tastatur-/Mauswege, Identität bei Sortieren/Refresh, sichtbare Zählung; `..` nicht markierbar | Gemischte Auswahl und Liste ändern | Teilnachweis pass — installiertes 1.2.27 EN/dunkel: Space markiert Ordner/Datei, Cmd+A lässt Elternverzeichnis aus; Zählung, Markierungen und Cursor bleiben bei Sortierung/Refresh korrekt. Aktueller Maus-Teilnachweis auf 1.2.32 EN/dunkel: Cmd-Klick markiert Ordner, einfacher Klick erhält Markierung, Shift-Klick ergänzt Dateibereich, Escape hebt Markierungen auf; Zählung und unabhängige Dateihashes korrekt ([Mausbeleg](CANDIDATE-1.2.32-NATIVE.md#ergänzte-mausauswahl)). Cmd-Entmarkieren/Pfeiltasten-/Insert-/weitere Varianten V, G ([früherer Beleg](CANDIDATE-1.2.27-LOCAL-MULTI.md)) |
| F08 | C:F1, F5; U | F2 Umbenennen / F3 Ansehen / F4 Bearbeiten | Cursorziel klar, Aktion ausführbar; ungültiger Name/Fehler erklärbar; Rückweg erhält Auswahl | Isolierte Dateien, gültig/ungültig/Abbruch | Historischer Befund #614/#617 auf 1.2.2; F2 über Alias-Pfad und verständliche F3-Details auf 1.2.19 nativ DE/EN pass ([Beleg](NATIVE-ACTIONS-FONT-2026-10-22.md)); F4 Bearbeiten/Speichern/Rückweg mit Inhaltsvergleich pass. Weitere Varianten V, G ([Beleg](FILE-ACTIONS-2026-10-22.md)) |
| F09 | C:F3, F5; U | F5 Kopieren / F6 Verschieben | Quelle/Ziel klar, Mehrfachauswahl berücksichtigt; Konflikt, Fortschritt, Abbruch und Fehler wieder auflösbar | Echte lokale/Remote-Transfers, Inhaltsvergleich | Teilnachweis pass — installiertes 1.2.27 EN/dunkel: Datei und verschachtelter Ordner Remote→lokaler Alias kopiert, Quellen erhalten; separate Datei lokaler Alias→Remote verschoben, Quelle entfernt; unabhängige Hashes korrekt, Ansichten automatisch aktualisiert. Bestehender Ordner sichtbar übersprungen. Zusätzlich lokale Mehrfachkopie (Ordner + zwei Dateien) in verändertes Zielverzeichnis unabhängig gehasht; Zielwahlabbruch ohne Seiteneffekt. Doppelte Transferanzeige fail #658 ([lokaler Beleg](CANDIDATE-1.2.27-LOCAL-MULTI.md)). Laufender Transferabbruch/Fehler/weitere Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-REMOTE.md)); aktueller Teilnachweis: 1.2.32 EN/dunkel: laufender 1-GiB-Remote→Lokal-Transfer zeigt genau eine Anzeige beim Dateimanager-/Terminal-/Tab-/Nachbarpane-Wechsel (#658); Pause und Abbruch sichtbar, Quelle unabhängig gehasht unverändert, unvollständiges Ziel entfernt ([Remote 1.2.32](CANDIDATE-1.2.32-REMOTE.md)) |
| F10 | C:F5; U | F7 Ordner / F8 Löschen | Name/Fehler verständlich; Bestätigung zählt Ordner samt Inhalt und Dateien getrennt; Abbruch ohne Verlust | Ausschließlich künstliche Testdaten | Teilnachweis pass — F7 Anlage/Abbruch samt Dateisystemkontrolle; Cursorfehler #615 auf 1.2.19 nativ DE/EN korrigiert ([Beleg](NATIVE-ACTIONS-FONT-2026-10-22.md)). F8 auf installiertem 1.2.27 EN/dunkel: getrennte Zählung, Löschabbruch mit gleichen Hashes und bestätigte Teilmenge ohne Änderung unmarkierter Datei pass ([Beleg](CANDIDATE-1.2.27-LOCAL-MULTI.md)); weitere Varianten V, G ([Beleg](FILE-ACTIONS-2026-10-22.md)) |
| F11 | C:F1 | Dateimanager bei laufender Shell schließen | CWD-Übernahme oder ehrlicher Hinweis; kein blindes cd bei unklarer Identität, kein Entwurfsverlust | Freier/belegter Prompt, verschachtelte Shell | Teilnachweis pass — 1.2.32 EN/Light, lokale Shell in `read`: F10 meldet aufgeschobenen CWD-Wechsel; `read` erhält nur die explizite manuelle Eingabe, danach wechselt dieselbe Shell in das Zielverzeichnis. PID, nicht exportierter Marker und Dateihashes erhalten. Verschachtelte/Remote-Shells und übrige Varianten V, G, R ([Beleg](CANDIDATE-1.2.32-NATIVE.md)) |
| R01 | C:R1 | Offline/Reconnect abwarten | Sperre und Ursache sichtbar; gleiche Route/Generation, freigegebene Eingabe erst nach Ready | Echte Trennung, Vorher/Nachher + Video | Teilnachweis pass — 1.2.23: eigener SSH-ControlMaster beendet, identische Remote-PID/nicht exportierter Marker/CWD nach automatischer Wiederverbindung; zusätzlich auf 1.2.27 aktiver Claude-Vordergrundprozess samt Shellwerkzeug über Transportabbruch erhalten, Ergebnis unabhängig geprüft ([Agentenbeleg](CANDIDATE-1.2.27-AGENT-THEME.md)); API-Testprofil, Codex-Werkzeug umgebungsblockiert. Offline-Anzeige/Generation/Video und weitere Varianten V, G, R ([früherer Beleg](CANDIDATE-1.2.23-EXPLORATION.md)); aktueller Teilnachweis: 1.2.32 EN/dunkel: installierte zsh5.9 reicht native `ssh -O check/exit` durch (#657); tatsächlicher Zaplex-ControlMaster beendet, nach Wiederverbindung gleiche Remote-Shell-PID/nicht exportierter Marker/CWD ([Remote 1.2.32](CANDIDATE-1.2.32-REMOTE.md)) |
| R02 | C:R1, R2 | Timeout, Retry, Abbruch, einfacher Terminalmodus | Verständlicher Ausgang; keine lokale Ersatzshell; Corrupt ohne tote Aktionen; einfacher Modus mit dauerhaftem Hinweis | Je echter Zustand und Folgeaktion | Teilnachweis pass — 1.2.32 DE/Light: Enter-Retry nach echter Verbindungsverweigerung unabhängig durch Listener-Accept belegt; Escape zeigt ausdrücklichen Abbruch. Erfolgreicher Handshake, andere Recoveryzustände/einfacher Modus und übrige Varianten V, G, R ([Beleg](CANDIDATE-1.2.32-NATIVE.md)) |
| E01 | U | Einstellungen öffnen, ändern, zurück | Als Tab erkennbar; Änderung sichtbar; Zurück zur bestehenden Shell ohne Zustandsverlust | Einstellung vorher/nachher + Shellmarker | Teilprüfung: Theme-Wechsel/Ansichten-Rückkehr pass; D06 auf 1.2.8 in DE/EN korrigiert, Entwurf/Fokus/Auswahl erhalten ([Nachweis](SSH-FORM-2026-10-21.md)); Shellmarker/Schließen in EN mit Dark→Light auf 1.2.2 [bestätigt](SETTINGS-2026-10-22.md); auf 1.2.27 bleiben Filter-/Rename-Entwürfe und Auswahl beim Theme-Wechsel lesbar, Filterfokus nach Settings-Rückkehr fail #656 ([Beleg](CANDIDATE-1.2.27-AGENT-THEME.md)); übrige Varianten V; aktueller Teilnachweis: 1.2.32 DE/EN: Filter, explizit fokussiertes Rename und Neuer Ordner erhalten Entwurf/Auswahl über Dark/Light-Wechsel; erste rohe Eingabe nach Rückkehr landet ohne Feldklick korrekt. Initialer Dialogfokus ist separat #673 ([1.2.32](CANDIDATE-1.2.32-NATIVE.md)) |
| E02 | U | Settings suchen und Unterseiten bedienen | Suchtreffer/Leerzustand, Toggles, Felder, Dropdowns, Speichern/Abbrechen und Dateizugang verständlich | Inventar unten, jede erreichbare Kontrolle | Teilnachweis: Suche/Leerzustand/Reset und Schriftänderung pass; ungültige Schriftgröße im alten DMG fail (#622), [EN Dark/Light](SETTINGS-2026-10-22.md); Korrektur 1.2.20 lokal und nativ macOS DE/EN pass ([Nachweis](FONT-VALIDATION-2026-10-22.md)); übrige Kontrollen/Varianten V, G |
| E03 | U | Weitere Sidebar-Werkzeuge nutzen | Tooltip/Bezeichnung, Leer-/Lade-/Fehlerzustand, Auswahl und Rückweg ohne Sackgasse | Inventar unten, echte statt Platzhalterzustände | not-run — V, G, R, P |
| E04 | U | Native Menüs, Kontextmenüs, Palette, Dialoge erkunden | Sinnvolle Benennung, Shortcuts, Fokus und Abbruch; keine wirkungslosen sichtbaren Aktionen | Kontrollinventar mit Zustandsfolge | Teilprüfung: Theme-Auswahl/Schließen und Settings-Tab-Rückweg pass ([Beleg](SETTINGS-2026-10-22.md)); übrige Menüs/Varianten V, G |
| X01 | U | Einstieg ohne Host frei lösen | Tester findet aus sichtbarer UI selbst Terminal, Host-Anlage und Rückweg; Hindernisse protokolliert | Exploratives Aktionsprotokoll | Teilprüfung 21.10.: siehe Explorationsbericht; übrige Aufgaben/Varianten V, G |
| X02 | U | Lokales Projekt mit mehreren Panes bearbeiten | Tester wählt selbst nächste Schritte; Kontext bleibt nachvollziehbar; unerwartete Zustände werden untersucht | Exploratives Aktionsprotokoll | Teilprüfung — lokale Rechte-/Unten-Splits, Shellmarker und Rückweg; Menüfokusfehler #629. Weitere Aufgaben/Varianten V, G ([Beleg](PANES-2026-10-23.md)) |
| X03 | U | Host/Favorit und Agent/Konto finden | Echte Ziele ohne Codewissen finden, starten, wiederfinden und Fehler auflösen | Exploratives Aktionsprotokoll | not-run — V, G, R, P |
| X04 | U | Dateien mit eindeutigem Ziel bearbeiten | Auswahl, Transfer, Konflikt/Abbruch und Rückkehr aus sichtbarer UI verständlich | Exploratives Aktionsprotokoll | Teilnachweis pass — installiertes 1.2.27 EN/dunkel: Alias-Verbindung, eigene Remote-Pane, Datei-/Ordnerkopie, Move und Shellrückweg aus sichtbarer UI; Inhaltsvergleich unabhängig. Konfliktabbruch und weitere Varianten V, G, R ([Beleg](CANDIDATE-1.2.27-REMOTE.md)) |
| X05 | U | Einstellung ändern und Fehler beheben | Tester findet Einstellung und Rückweg; korrigiert eine absichtlich ungültige Eingabe | Exploratives Aktionsprotokoll | Teilprüfung: Theme-Wechsel/Rückweg pass; D06 gezielt auf 1.2.8 korrigiert ([Regression, keine neue Exploration](SSH-FORM-2026-10-21.md)); ungültige Schriftgröße fail (#622), gültige Korrektur und Shellrückkehr pass ([EN Light](SETTINGS-2026-10-22.md)); anschließende Regression der Fehlerbehandlung lokal DE/EN pass ([Nachweis](FONT-VALIDATION-2026-10-22.md)), keine neue Exploration; übrige Varianten V, G |

## Kontrollinventar für den nächsten Lauf

Dies ist ein **Quellinventar als Ausgangspunkt**, keine Behauptung, alle nativen
Elemente bereits erfasst zu haben. Jede sichtbare Kontrolle erhält im Lauf eine
Zeile mit Oberfläche, Beschriftung/Tooltip, Zustand, Auslöser, Ergebnis und Beleg.
Neu entdeckte Dialoge/Untermenüs werden aufgenommen; ausgeblendete Feature-Flächen
als nicht erreichbar mit der tatsächlich beobachteten Konfiguration dokumentiert.

| Oberfläche | Zu inventarisierende Kontrollen / Unteransichten | Ausgangspunkt |
|---|---|---|
| Fenster/Tabs/Panes | Ampel, Sidebar-Schalter, Plus/Pfeil, Tabkontext, Titel, Pane-Menü/X/Dateimanager, Divider, Drop-Zonen, Such-/Inbox-/Settings-Zugänge | C:P1–P5, L/J; native Menüs |
| Toolbelt | KI-Sessions, KI-Konten, Verbindungen, Project Explorer, globale Suche, Konversationsliste, Skills; Server File Browser und Drive soweit tatsächlich erreichbar; Schließen/Zurück | `app/src/workspace/view/left_panel.rs`, `ToolPanelView` |
| Verbindungen | Ordner/Plus, Gruppen-Chevrons, Hostzeile, Stecker, Stern, Kontextmenü, Bearbeiten-/Löschen-Dialog, Recovery/Refresh/Details/Session-Aktionen | `app/src/ssh_manager/panel.rs` |
| Cockpit/Konten | Baum-Chevrons, Titel/Tooltips, Auswahl, Scrollbereiche, Account-Karten, Verbrauch, Sessionsuche/-filter/-menü, neuer Agent, Aufmerksamkeit/Inbox | C:T/S/I |
| Dateimanager | Breadcrumb, Zurück/Vorwärts/Hoch/Refresh, Upload/Plus/Versteckte, Filter, Sortierung, Zeile/Cursor/Markierung, Zielwahl, alle acht F-Aktionen und Folge-Dialoge | C:F1–F5; E04/E05 im Belegmanifest |
| Settings/Agents | Zaplex Agent, Profiles, MCP servers, Knowledge, Third party CLI agents; alle Schalter, Felder, Listenaktionen und Dialoge der jeweils erreichten Seite | `SettingsSection`, `SettingsSection::ai_subpages` |
| Weitere Settings | Code, Appearance, Features, Keyboard shortcuts, Zaplexify, Cloud Sync, About; Network nur falls freigeschaltet; Suche, Gruppen, „Open settings file“ | `app/src/settings_view/mod.rs` |
| Native Menüs/Overlays | Zaplex, File, Edit, View, Tab, Blocks, AI, Window, Help; jeweils erreichbare Untermenüs, Befehls-/Agent-Palette, Kontextmenüs, Bestätigungen, Fehler | Native Menüleiste und AX-Aufzeichnung |

Exploration erhält nur Aufgaben und Sicherheitsgrenzen, keine feste Klickfolge.
Nach jeder Aktion wird neu gesehen und entschieden. Fehlende Anmeldung wird als
Voraussetzung dokumentiert; erfundene Accounts oder simulierte Recovery-Zustände
ergeben keinen Pass. Eine agentische Bewertung ersetzt keine Studie mit Nutzern.
