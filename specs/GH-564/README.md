# Native UI-/UX-Abnahme: Vorprüfung und gesicherte Altbuild-Belege

Stand: 19.10.2026. Auftrag [#564](https://github.com/byte5ai/zaplex/issues/564),
übergreifend [#562](https://github.com/byte5ai/zaplex/issues/562).

**Keine Gesamtfreigabe.** Das aktuelle DMG ist gebaut und installiert geprüft:
12 lokale GUI-Tests bestanden im ersten Lauf; die drei übrigen bestanden nach
Testkorrektur im gezielten Wiederholungslauf. Die vollständige
DMG-/Explorationsabnahme fehlt; Details im [Fortsetzungsnachweis](NATIVE-FIXTURES.md#gezielte-fortsetzung-und-installiertes-dmg).
Zusätzlich zu dieser Altbuild-Auswertung liegen jetzt
[native Integrationstest-Belege](NATIVE-FIXTURES.md) vor:
15 Originalaufnahmen auf `1201fc4f` sowie drei erneut geprüfte Aufnahmen auf
`cadcb191` (Paketversion 1.2.2). Beide visuellen Fehler bestehen weiterhin.
Im [Batch 39257382471](https://github.com/byte5ai/zaplex/actions/runs/39257382471)
sind alle Code-/Testprüfungen und der Linux-Daemon-Build erfolgreich. Der
macOS-App-Build verlor die Runner-Verbindung. Die gezielte Fortsetzung
[39295080993](https://github.com/byte5ai/zaplex/actions/runs/39295080993)
baute danach App und DMG erfolgreich mit vier Cargo-Jobs und übernahm den
vorhandenen Linux-Daemon. Es wurde kein neuer Gesamtbatch ausgeführt.
Die vor dem nächsten Lauf festgelegte [Prüfmatrix](MATRIX.md) enthält die
Sollzustände, Nutzeraufgaben, Varianten, benötigten Nachweise und die konkreten
Gründe für `not-run`. Freie Exploration wurde nicht durchgeführt.

Diese Altbuild-Auswertung hat vorhandene native Screenshots visuell bewertet und
mit den versionierten Sollvorgaben verglichen. Sie hat keine neue native
App-Interaktion ausgeführt. Die früheren deterministischen Funktionstests sind
keine explorativen Abläufe und werden hier nicht dazu umdeklariert.

## Prüfgegenstand und Herkunft

| Merkmal | Verifizierter Stand |
|---|---|
| Gewünschter aktueller Build | `main`, `6c956383ad560913dc31fa383104dc489104d561` |
| Tatsächlich aufgenommenes Artefakt | Unverändertes Release-DMG v1.2.0-rc1; Bundle-Version 1.2.0 |
| DMG SHA-256 | `4c7fae29904f46ec20f9fe1fdbb82c189ef8501d172984030b4b53ba873ab5bc` |
| GitHub-Asset-ID | `650971257`, live gegen Release-API und vorhandene Testberichte abgeglichen |
| Release-Quellcommit | `83db26188f2df30023e6c0d649d3b305730751cb`, live über annotierten Tag aufgelöst |
| Grenze der Build-Zuordnung | Alte `acceptance.json` enthält `app_source_sha: null`; kein unabhängig aufgezeichneter App-Quellcommit. Release-Zuordnung und Testrevision sind getrennt. |
| Referenzrevision | `6c956383ad560913dc31fa383104dc489104d561`, Einstieg `docs/ui/README.md`; kein aus App-Bildern erzeugtes Soll |
| Native Varianten in den Bildern | Englische Beschriftungen, dunkle Darstellung; keine nachgewiesene OS-Reduced-Motion-Einstellung |
| Normales Fenster | 1024 × 678 Punkte, Bildschirmaufnahme 1024 × 768 Pixel |
| Schmale Aufnahme | AX-Hauptfenster 513 × 680 Punkte, Bildschirmaufnahme 1024 × 768 Pixel; halber Desktop, **kein Nachweis der nativen Mindestbreite** |

Die frühere Angabe „512 Punkte“ wird für diese Auswertung durch den
zeitgleichen AX-Wert **513 Punkte** präzisiert. Das zweite Fenster hinter dem
schmalen Dateimanager ist ein separates Fenster, kein zweites Pane im selben Tab.

Die ursprüngliche API-Prüfung fand kein neueres verfügbares DMG für den gewünschten Stand.
Der damals letzte erfolgreiche CI-Batch
[39017883322](https://github.com/byte5ai/zaplex/actions/runs/39017883322)
lief auf `7f26510d087e5b6ca6ea4df2f5926ed6d49cd817`, nicht auf dem Prüfziel,
und übersprang den Test-DMG-Job. Der inzwischen gestartete Neubau auf `cadcb191`
ist im [aktuellen Build-Nachweis](NATIVE-FIXTURES.md#build-nachweis-und-verbleibende-grenze)
dokumentiert.

## Gesicherte Belege

Die elf PNGs unter [evidence/](evidence/) sind unveränderte Originalbytes aus
den XCTest-Artefakten. [manifest.json](evidence/manifest.json) enthält pro Bild
SHA-256, ursprünglichen Dateinamen, Testfall, Zeitpunkt, Testrevision,
Fensterrechteck samt Herkunft und Einschränkungen. Sie bleiben damit nach Ablauf
der 14-tägigen Actions-Aufbewahrung prüfbar. Vollständige Rohlogs/xcresult wurden
nicht ins Repository kopiert; die folgenden Links können später ablaufen.

| Lauf | Testrevision | Bedeutung |
|---|---|---|
| [39234535044](https://github.com/byte5ai/zaplex/actions/runs/39234535044) | `767a43779ce10a06a4fa3587a1ccca093096da07` | Navigation, Splits, erster Dateimanager-/Connections-Nachweis; Testlauf insgesamt nicht grün |
| [39235741317](https://github.com/byte5ai/zaplex/actions/runs/39235741317) | `4e87335139ae89ecadcd73d4bd4ec63ba54ed2b6` | Settings/Connections; damaliger schmaler Aufbau fehlgeschlagen |
| [39236377328](https://github.com/byte5ai/zaplex/actions/runs/39236377328) | `0b980268dcfe31f4a9fee27856cbfa9f411f7824` | Ein gezielter bestandener Fall mit schmalem Dateimanager |

## Visuelle Einzelbewertungen des Altbuilds

Die Statuswerte dieser Tabelle gelten **nur für die benannte Beobachtung in den
aufgeführten Bildern**, nicht für die gesamte Fläche, weitere Varianten oder `main`.
Die Prüfschritte und vollständigen Abnahmekriterien stehen in der Matrix.

| Beobachtung / unabhängiges Soll | Status | Beleg und Bewertung |
|---|---|---|
| Verbindungsliste: vollständiger nächster Schritt im Leerzustand (#564) | fail | [E01](evidence/connections-original.png), [E02](evidence/connections-empty.png): Anleitung am rechten Panelrand abgeschnitten; Plus-Hinweis fehlt. D01 / #565. |
| Leere Favoriten: vollständige Anleitung im Startmenü (#564, L2) | fail | [E03](evidence/split-menu-empty.png): Ende des Hinweises auf die Sidebar abgeschnitten. D02 / #566. |
| Dateimanager: alle acht beschrifteten Aktionen bei normaler Breite (C:F2) | pass | [E04](evidence/file-manager-normal.png): F2 Rename, F3 View, F4 Edit, F5 Copy, F6 Move, F7 Folder, F8 Delete und F10 Terminal sichtbar in einer Reihe. |
| Dateimanager: dieselben acht Aktionen in schmaler Ansicht (C:F2) | pass | [E05](evidence/file-manager-narrow.png): zwei Reihen mit je vier Zellen; keine fehlende Beschriftung. Vier Reihen und Mindestbreite nicht belegt. |
| Dateimanager: dauerhafte Aufforderung zur Zielwahl ohne gewähltes Ziel (C:F3) | pass | E04/E05: „Choose destination“ unmittelbar über der F-Leiste. Verständlichkeit/Bindung bei mehreren echten Zielen und Klickwirkung nicht belegt. |
| Dateimanager: lokale Host-/Verzeichnisidentität statt globalem Dateimanager-Tab (C:P3, F1) | pass | E04: Tab/Pane tragen „Local · zaplex-gui.eg8t2d“, Breadcrumb zeigt dasselbe erzeugte Testverzeichnis. Nicht auf Remote oder Restore übertragbar. |
| Einfache lokale Splits: neue Shell liegt in gewünschter Richtung (C:P2) | pass | [E07 rechts](evidence/split-right.png), [E08 links](evidence/split-left.png), [E09 unten](evidence/split-down.png), [E10 oben](evidence/split-up.png): frische Ausgabe 963 liegt relativ zur ursprünglichen Shell 741 richtig. Keine Aussage über Drag, Mehrhost-Routing oder verdeckte Kopfaktionen. |
| Settings: erkennbare eigene Ansicht mit sichtbarer Navigation (U) | pass | [E06](evidence/settings.png): Settings-Tab, ausgewählte Zaplex-Agent-Seite, Seitennavigation und Schalter erkennbar. Keine Aussage über Wirkung der Schalter, übrige Seiten oder Tastaturfokus. |
| Startmenüs Plus/vier Splits: vollständiger visueller Vergleich (L1–L6) | not-run | E03 zeigt nur ein leeres Split-Menü. [E11](evidence/menu-capture-too-early.png) trägt den Namen „split-launch-menu-Right“, zeigt aber noch kein Menü. Keine Freigabe anhand des Dateinamens. |
| Kopfaktionen, volle Typografie-/Kontrastparität und gesamte Fläche | not-run | Systembanner verdecken obere rechte App-Bereiche. Unterschiedliche Zustände und Fenstergrößen gegenüber dem Mockup erlauben keine pauschale Pixelbewertung. |

Die abgesetzte durchgehende Sidebar ist in E01/E02 erkennbar. Daraus folgt keine
Freigabe des gefüllten B+-Baums oder der Konten. Das ältere Gesamt-PNG der Referenz
zeigt Sessions und Konten zusammen; die ausdrücklich neuere Trennung ist maßgeblich.
Ebenso ist die native Acht-Aktionen-Leiste gemäß C:F2 zu bewerten, nicht gegen die
vier Aktionen im alten PNG.

## Priorisierte Korrekturaufträge

### D01 — P2: abgeschnittene Verbindungsanleitung

[Korrekturissue #565](https://github.com/byte5ai/zaplex/issues/565).
Reproduktion: frisches Profil ohne Hosts → Verbindungen → Sidebar um 250 Punkte.
Soll: Ordner- und Serveranlage vollständig erklärt. Ist: „No SSH servers yet.
Use the folder b…“, ohne lesbaren Plus-Hinweis. E01/E02 zeigen denselben Befund
in zwei Läufen; der Systembanner verdeckt diese Stelle nicht.

UX-Folge: Neue Nutzer erfahren nicht vollständig, wie sie den ersten Server
anlegen. Die Toolbar bleibt erreichbar; eine vollständige Sackgasse ist damit
nicht bewiesen. Aktuelles `render_tree` in `app/src/ssh_manager/panel.rs` verwendet
weiterhin `Text::new_inline` für den vollständigen Übersetzungstext. Das ist ein
passender Codebefund, kein neuer nativer Repro.

Testauftrag: neuer Build, DE/EN, Sidebar 250/320/480 Punkte; vollständiger
Handlungshinweis innerhalb der Fläche und tatsächlich bedienbare Plus-/Ordneraktion,
Abbruch und Fokusrückgabe. Ein OCR-Treffer nur auf dem ersten Satz reicht nicht.

### D02 — P2: abgeschnittene Favoritenanleitung

[Korrekturissue #566](https://github.com/byte5ai/zaplex/issues/566).
Reproduktion: Profil ohne Favoriten → Tab → Split Pane Right → sichtbares Startmenü.
Soll: verständlicher Hinweis auf Favorisieren eines Hosts in der Sidebar.
Ist: Der Text endet sichtbar nach „No favorite hosts yet — ★ a h…“ (E03).
Terminal und neuer Agent bleiben lesbar; keine vollständige Blockade nachgewiesen.

Die vollständige Übersetzung `workspace-favorites-empty` wird auf aktuellem
`main` weiterhin als deaktivierter Menüeintrag eingefügt; die genaue Ursache
der Breitenbegrenzung ist noch zu untersuchen. Testauftrag: Plus/vier Splits,
DE/EN, normale/Mindestbreite und Fensterränder; vollständige sichtbare Anleitung,
Abbruch ohne neue Pane und Fokus zurück. Ein fehlender Favorit benötigt dafür
keinen echten Remotezugang.

### D03 — Nachweislücke: Menü-Screenshot vor sichtbarem Menü

Konkreter Testauftrag an [#562](https://github.com/byte5ai/zaplex/issues/562):
In `NavigationTests::split` erfolgt `evidence("split-launch-menu-…")` vor dem
sichtbarkeitsprüfenden `clickVisible("Terminal")`. E11 zeigt den Effekt: ein
Terminalbild mit irreführendem Anhangsnamen. Der funktionale Split-Nachweis
bleibt davon getrennt gültig.

Vor der Menüaufnahme auf den gerenderten Menüinhalt warten, Aufnahme mit ihrer
gemessenen Fenstergeometrie verbinden, danach den Zielpunkt aus einem frischen
Bild bestimmen. Einen betroffenen Fall gezielt erneut ausführen und das Bild
ansehen. Erst sichtbare Menüs eignen sich zum Vergleich von Reihenfolge,
Randplatzierung und Favoriten-Flyouts. Keine Produktänderung aus dieser
Aufnahmelücke ableiten.

## Fortsetzung und Aussagegrenzen

Es gibt **keinen neu belegten Produkt-Showstopper in dieser begrenzten Bildauswahl**;
das bedeutet nicht, dass die App keine hat. Der bereits bekannte native
Schließen-Fehler des alten DMG und dessen Korrektur #558 bleiben beim aktuellen
DMG erneut zu prüfen (bestehender Auftrag #559/#562).

Der aktuelle Gesamtvergleich benötigt den noch nicht bereitgestellten
begrenzten explorativen GUI-Zugriff sowie
isolierte Remote-/Provider-Testzugänge. Der bestehende private Infrastrukturauftrag
wurde live gelesen und ist offen; hier werden weder private Endpunkte noch
Zugangsdaten veröffentlicht. Saubere, nicht durch Systembanner verdeckte
Aufnahmebedingungen gehören zu dieser Voraussetzung.

Die autorisierte Fortsetzung auf `cadcb191` ist ausgeführt. Das signierte und
notarisierte DMG enthält den vorhandenen Linux-Daemon; die erfolgreichen
Codeprüfungen wurden übernommen. Alle 15 lokalen GUI-Fälle wurden in
[39303447063](https://github.com/byte5ai/zaplex/actions/runs/39303447063)
ausgeführt, mit 12 bestandenen und drei fehlgeschlagenen Fällen. Die drei Fälle bestehen nach Testkorrektur in
[39329863382](https://github.com/byte5ai/zaplex/actions/runs/39329863382) am
unveränderten DMG; die beiden Testrevisionen sind getrennt dokumentiert.
Kein neuer Gesamtbatch und
keine Release-Veröffentlichung wurden gestartet.

Auch ein grüner neuer 15-Fälle-Lauf erledigt diese Matrix nicht. Danach folgen
die eigentliche native Sichtprüfung, das vollständige Kontrollinventar und
die unabhängigen explorativen Aufgaben mit den benötigten Testzugängen.
Offene notwendige Fälle verhindern die Gesamtfreigabe; #564 und #562 bleiben offen.
