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
keine Tests. Der native Nachweis bleibt ausstehend.
Die [Dateimanager-Exploration vom 22.10.](EXPLORATION-2026-10-22.md) am
unveränderten 1.2.2-DMG ergänzt Tastatureinstieg, Unterordner-Navigation und
Umbenennen-Dialog; der wirkungslose Ordner-Klick wird in #604 untersucht.
Weder Altbuild noch Fixtures ersetzen die vollständigen Fallkombinationen.

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
| V01 | C:C3, A6; U | Ohne eingerichteten Host starten | Arbeitsfläche, vorhandene horizontale Navigation und nächster Schritt erkennbar; keine überdeckten Texte oder dekorativen Farbränder | Gesamtansicht und Navigation | Teilbefund fail — 1.2.8: Einführungstexte in schmaler Terminal-Pane abgeschnitten (#601); Gesamteinstieg/übrige Varianten V, G |
| V02 | C:A4, A6; U | Alle sichtbaren Icons erkunden | Verständliche lokalisierte Tooltips, stabile Geometrie, klare aktive/inaktive Zustände | Hover-/Fokusfolge pro Icon | not-run — V, G |
| V03 | U | Per Tastatur navigieren und zurückkehren | Sinnvolle Fokusreihenfolge; sichtbarer Fokus; Escape/Zurück und Rückgabe zum Auslöser | Tastaturablauf | not-run — V, G |
| C01 | C:C1; U | Leere Verbindungsliste verstehen | Vollständige Anleitung einschließlich Ordner- und Plus-Aktion, auch schmal | Leerzustand DE/EN, 250/320/480 | Teilnachweis pass — native 1.2.4, DE/EN, Hell/Dunkel, 250/320/480 (#565); installiertes DMG der Korrektur V |
| C02 | C:C1; U | Ordner und Host anlegen/bearbeiten | Eindeutige Feldnamen, Validierung, Speichern/Abbrechen; keine doppelte Konfiguration | Formular leer/ungültig/gültig | Teilnachweis pass — 1.2.8, DE/EN: Titel/Aktionen und ungültiger Host nach #593/#594 korrigiert ([Nachweis](SSH-FORM-2026-10-21.md)); historische D04/D05 auf 1.2.2; gültiges Speichern/übrige Varianten V |
| C03 | C:C1, A1, A2 | Konfigurierten Host finden/favorisieren | Identität priorisiert; feste Stecker-/Sternslots; Offline-Host bleibt hier sichtbar | Lange Registry-Zeilen, Sternwechsel | not-run — V, G |
| C04 | C:C1, C4; U | Verbinden, Fehler erkennen, abbrechen | Lade-/Fehlerzustand mit verständlichem nächstem Schritt; kein festhängender Dialog | Echte Verbindung und definierter Fehler | Teilnachweis pass — 1.2.2, EN: echter Key-Host-Import und persistente Remote-Shell; Fehler/Abbruch und Varianten V, G, R |
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
| L01 | L1, L2, L5 | Plus und vier Split-Menüs öffnen | Gleiche gemeinsame Einträge/Reihenfolge; Tabzusätze nur bei Plus; Menü vor Screenshot sichtbar | Fünf offene Menüs | not-run — V, G; Altbelege teilweise zu früh |
| L02 | L2; U | Leere Favoriten verstehen | Vollständige Anleitung zur Favorisierung sichtbar, auch am Fensterrand | DE/EN, normale/schmale Breite | Teilnachweis pass — native 1.2.4, DE/EN, Hell/Dunkel (#566); übrige Einstiege, Randlagen und DMG V |
| L03 | L3, L4; C:C2 | Favorit bzw. weiteren Host wählen | Name verbindet einmal; nur Nichtfavoriten im Untermenü; Stern-Änderung konsistent | Registry + Menü + echte Verbindung | not-run — V, G, R |
| L04 | L4; C:C2 | Separates Favoriten-Flyout bedienen | Ellipse verbindet nicht; seitliches Flyout hält Elternmenü sichtbar; Randplatzierung/Fokus/Zurück stimmen | Maus und Tastatur an beiden Rändern | not-run — V, G, R |
| L05 | L6; C:P2 | Split per Escape/Außenklick abbrechen | Keine neue Pane/Session; ursprüngliche Shell und Fokus bleiben | Vorher/Nachher mit Shellzustand | not-run — V, G |
| L06 | L7–L9 | Agent in erfasster Pane starten | Genau eine Session an erfasster Position; Mehrfachkonto und verschwundenes Ziel mit Erklärung, kein falscher Start | Kontowahl, Fokuswechsel, ungültiges Ziel | not-run — V, G, P, R |
| P01 | C:P2 | Lokal rechts/links/oben/unten teilen | Neue unabhängige Shell in jeder gewünschten Richtung; Quellshell bleibt | Vier Geometrien und unabhängige Ausgabe | not-run — V |
| P02 | C:P2, P3 | Mehrhost-Panes nebeneinander nutzen | Host/CWD/Startziel pro Pane, kein Routing über zufälligen Fokus | Zwei echte Hosts, asynchroner Start | not-run — V, G, R |
| P03 | C:P3; N1 | CWD, Fokus und Tabtitel wechseln | Host·Verzeichnis korrekt; expliziter Tabtitel gewinnt, Entfernen stellt Automatik wieder her | Wechsel und kollidierende Namen | not-run — V, G, R |
| P04 | C:P4, P5 | Pane verschieben/abtrennen | Dieselbe Pane ohne neue PTY; gültige Drop-Zonen; Abbruch erhält Ausgangslayout | Header-Drag in vier Richtungen/Tab | not-run — V, G, R |
| P05 | J1–J8 | Ganzen Tab als Split einfügen | Ziel bleibt beim Drag sichtbar; Layout/Identitäten bleiben, Quelltab entfällt; Selbst-/ungültiger Drop wirkungslos | Drag/Abbruch, verschachtelte Panes | not-run — V, G, R |
| P06 | C:P5, R1 | Fenster/Session wiederherstellen | Geometrie, Fokus, Modus, Entwurf und richtige Remote-Identität erhalten; provisorischer Start nicht ins andere Fenster verschiebbar | Neustart/Restore + Drag während Start | not-run — V, G, R |
| F01 | C:F1, P3 | Dateimanager öffnen/zur Shell zurück | Pane-Kontext und Shellzustand bleiben; F10/Leistenaktion/Pane-X kehren zurück | Drei Rückwege, frischer Shellmarker | Teilbeobachtung 22.10.: Tastatureinstieg mit richtigem CWD pass; Ordner-Klick ohne sichtbare Wirkung (#604); Rückwege und Shellmarker nach Rückkehr not-run — V, G |
| F02 | C:F2 | Dateiaktionen bei wenig Platz finden | F2–F8/F10 mit kurzer Beschriftung; eine/zwei/vier gleich breite Reihen; keine versteckten Aktionen | Breitenfolge, alle acht Zellen | not-run — V, G |
| F03 | C:F2, F5 | Fokus zwischen Dateipanes wechseln | Nur kompatible fokussierte Pane aktiviert Aktionen; Cursor und Markierung unterscheidbar | Zwei Panes mit gleichem Dateinamen | not-run — V, G |
| F04 | C:F4 | In Elternverzeichnis zurückgehen | Verlassener Ordner wird nach Identität ausgewählt und sichtbar gescrollt | Verschachtelte, sortierte Liste | not-run — Rückkehr nach oben nicht zugestellt (Bedienzeit abgelaufen); nur Öffnen des Unterordners am 22.10. beobachtet — V, G |
| F05 | C:F3 | Transferziel bewusst wählen | Dauerhafte Zielzeile; eindeutiger Host/Pfad/Pane; eigener Tab zuerst; Quelle bei F5/F6 erkennbar | Mehrere mögliche Ziele, gleiche Namen | not-run — V, G, R |
| F06 | C:F3 | Ziel-Pane ändern/schließen | Gewähltes Ziel folgt deren Verzeichnis; nach Schließen/Moduswechsel neue Wahl, nie stiller Ersatz | Ziel ändern, dann Copy/Move | not-run — V, G, R |
| F07 | C:F5 | Dateien/Ordner markieren | Alle definierten Tastatur-/Mauswege, Identität bei Sortieren/Refresh, sichtbare Zählung; `..` nicht markierbar | Gemischte Auswahl und Liste ändern | not-run — V, G |
| F08 | C:F1, F5; U | F2 Umbenennen / F3 Ansehen / F4 Bearbeiten | Cursorziel klar, Aktion ausführbar; ungültiger Name/Fehler erklärbar; Rückweg erhält Auswahl | Isolierte Dateien, gültig/ungültig/Abbruch | Teilbeobachtung 22.10.: F2 öffnet passenden Dialog, neuer Name sichtbar; Bestätigung/Abbruch und F3/F4 not-run — V, G |
| F09 | C:F3, F5; U | F5 Kopieren / F6 Verschieben | Quelle/Ziel klar, Mehrfachauswahl berücksichtigt; Konflikt, Fortschritt, Abbruch und Fehler wieder auflösbar | Echte lokale/Remote-Transfers, Inhaltsvergleich | not-run — V, G, R |
| F10 | C:F5; U | F7 Ordner / F8 Löschen | Name/Fehler verständlich; Bestätigung zählt Ordner samt Inhalt und Dateien getrennt; Abbruch ohne Verlust | Ausschließlich künstliche Testdaten | not-run — V, G |
| F11 | C:F1 | Dateimanager bei laufender Shell schließen | CWD-Übernahme oder ehrlicher Hinweis; kein blindes cd bei unklarer Identität, kein Entwurfsverlust | Freier/belegter Prompt, verschachtelte Shell | not-run — V, G, R |
| R01 | C:R1 | Offline/Reconnect abwarten | Sperre und Ursache sichtbar; gleiche Route/Generation, freigegebene Eingabe erst nach Ready | Echte Trennung, Vorher/Nachher + Video | Teilnachweis pass — SSH-ControlMaster beendet; gleiche Shell-PID, Marker und CWD nach automatischer Wiederverbindung; Offline-Anzeige/Generation/Video und Varianten V, G, R |
| R02 | C:R1, R2 | Timeout, Retry, Abbruch, einfacher Terminalmodus | Verständlicher Ausgang; keine lokale Ersatzshell; Corrupt ohne tote Aktionen; einfacher Modus mit dauerhaftem Hinweis | Je echter Zustand und Folgeaktion | not-run — V, G, R |
| E01 | U | Einstellungen öffnen, ändern, zurück | Als Tab erkennbar; Änderung sichtbar; Zurück zur bestehenden Shell ohne Zustandsverlust | Einstellung vorher/nachher + Shellmarker | Teilprüfung: Theme-Wechsel/Ansichten-Rückkehr pass; D06 auf 1.2.8 in DE/EN korrigiert, Entwurf/Fokus/Auswahl erhalten ([Nachweis](SSH-FORM-2026-10-21.md)); Shellmarker/übrige Varianten V |
| E02 | U | Settings suchen und Unterseiten bedienen | Suchtreffer/Leerzustand, Toggles, Felder, Dropdowns, Speichern/Abbrechen und Dateizugang verständlich | Inventar unten, jede erreichbare Kontrolle | not-run — V, G |
| E03 | U | Weitere Sidebar-Werkzeuge nutzen | Tooltip/Bezeichnung, Leer-/Lade-/Fehlerzustand, Auswahl und Rückweg ohne Sackgasse | Inventar unten, echte statt Platzhalterzustände | not-run — V, G, R, P |
| E04 | U | Native Menüs, Kontextmenüs, Palette, Dialoge erkunden | Sinnvolle Benennung, Shortcuts, Fokus und Abbruch; keine wirkungslosen sichtbaren Aktionen | Kontrollinventar mit Zustandsfolge | not-run — V, G |
| X01 | U | Einstieg ohne Host frei lösen | Tester findet aus sichtbarer UI selbst Terminal, Host-Anlage und Rückweg; Hindernisse protokolliert | Exploratives Aktionsprotokoll | Teilprüfung 21.10.: siehe Explorationsbericht; übrige Aufgaben/Varianten V, G |
| X02 | U | Lokales Projekt mit mehreren Panes bearbeiten | Tester wählt selbst nächste Schritte; Kontext bleibt nachvollziehbar; unerwartete Zustände werden untersucht | Exploratives Aktionsprotokoll | not-run — V, G |
| X03 | U | Host/Favorit und Agent/Konto finden | Echte Ziele ohne Codewissen finden, starten, wiederfinden und Fehler auflösen | Exploratives Aktionsprotokoll | not-run — V, G, R, P |
| X04 | U | Dateien mit eindeutigem Ziel bearbeiten | Auswahl, Transfer, Konflikt/Abbruch und Rückkehr aus sichtbarer UI verständlich | Exploratives Aktionsprotokoll | not-run — V, G, R |
| X05 | U | Einstellung ändern und Fehler beheben | Tester findet Einstellung und Rückweg; korrigiert eine absichtlich ungültige Eingabe | Exploratives Aktionsprotokoll | Teilprüfung: Theme-Wechsel/Rückweg pass; D06 gezielt auf 1.2.8 korrigiert ([Regression, keine neue Exploration](SSH-FORM-2026-10-21.md)); ungültige Einstellung/übrige Varianten V, G |

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
