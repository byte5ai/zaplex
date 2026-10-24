# Kandidat 1.2.34: gezielte native Nachprüfung

App-Quellstand: `b0eff3ee326b7a4884de3681123cd84fad5ef45f`.
Der [Batch 40087073327](https://github.com/byte5ai/zaplex/actions/runs/40087073327)
baut den Kandidaten mit den Korrekturen #672/#673/#674. Ein vollständiger
Batch-/DMG-Abnahmebeleg liegt zum Stand dieses Zwischenberichts noch nicht vor.

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
