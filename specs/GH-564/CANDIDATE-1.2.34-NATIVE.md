# Kandidat 1.2.34: gezielte native Nachprüfung

App-Quellstand: `b0eff3ee326b7a4884de3681123cd84fad5ef45f`.
Der [Batch 40087073327](https://github.com/byte5ai/zaplex/actions/runs/40087073327)
enthält den Kandidaten mit den Korrekturen #672/#673/#674. Build, Sicherheits-
und Paketprüfungen (4430 bestanden), Signierung und Notarisierung sind bestanden.
Die installierte GUI-Suite besteht zunächst 14/15 Fälle. `testSplitDown` scheitert
bereits im Setup: Das Originalbild zeigt die richtige Nonce `149060`, Tesseract
liest `149960`. Das ist #680; der Split wird noch nicht erreicht.

Nach der reinen Testkorrektur #681 besteht ausschließlich der fehlgeschlagene
Fall im [Fortsetzungslauf 40092623416](https://github.com/byte5ai/zaplex/actions/runs/40092623416).
Auch die unabhängigen OCR-Gegenproben bestehen: echte Ausgabe, Ablehnung falscher
Nonce/falschen Zustands und des noch nicht abgeschickten Befehls sowie korrekte
Koordinaten. Testrevision: `4c1f203c1ce3c66d0923f950fd1d78770eacda94`.

Beide Läufe verwenden dasselbe unveränderte DMG mit SHA-256
`ec3ce86d7fdf78c5dc5852f5b8c717c3252428450199b5a1929cebe602ec5319`.
Damit sind alle 15 lokalen Pflichtfälle belegt, verteilt auf zwei Läufe; der
ursprüngliche Batch bleibt fehlgeschlagen und die Fortsetzung bleibt ein
Teillauf. Die kombinierte Herkunftsprüfung wird in #682 ergänzt. Die native
Exploration der drei Produktkorrekturen ist damit noch nicht nachgewiesen.

## Erzeugtes natives Menü-Fixture

Das [Originalbild](evidence/candidate-134/native-favorite-flyout.png) wurde
individuell betrachtet; Quellstand und alle 41 Workspace-Bildhashes stimmen
mit dem heruntergeladenen Artefakt 12557931513 überein. Nur dieses eine Bild
wird hier als visuell geprüft bewertet; das
[Manifest](evidence/candidate-134/fixture-review.json) hält die Zuordnung fest.

EN/Light, 900×640: Der vollständige Hinweis „More actions for
production-europe-west-development-0123456789“ bleibt über dem nach links
öffnenden Aktionsmenü lesbar und innerhalb des Fensters. Auf 1.2.32 wurde
er an dieser Stelle verdeckt. Der Hinweis liegt vorübergehend über der
ersten Aktionsbeschriftung; das Bild belegt weder Mausdurchlässigkeit noch
stabiles Hover-Verhalten oder Tastaturbedienung. Diese Punkte sowie die
Dialogfokus- und Sprachkorrektur werden am installierten DMG separat geprüft.
