# CI nach mehreren Merges

Schwere Prüfungen laufen ausschließlich nach Christians Freigabe als ein
`ci-batch.yml`-Lauf auf `main`. Alle wiederverwendbaren Workflows verwenden dabei
denselben Commit des Dispatchs; weitere Merges verändern diesen Lauf nicht.

| Workflow | Art und Auslösung |
| --- | --- |
| `no-new-cjk` | Leichter Linux-Check auf PRs. Dateipfade werden im Job gefiltert, damit der gleichnamige Pflicht-Status auch bei Dokumentationsänderungen erscheint. |
| `dependency-security` | Advisory-Prüfung ohne App-Build; zuerst im Batch, zusätzlich manuell aufrufbar. |
| `pr-check` | Schwere Linux-/macOS-Compile- und Regressionstests; nur aus dem Batch. |
| `test-dispatch` | Schwere Rust-Tests; nur aus dem Batch. |
| `live-sftp-safety` | Schwere SFTP-Integrationstests; nur aus dem Batch. |
| `cockpit-parity-audit` | Schwere Paritätsprüfung; nur aus dem Batch. Echte Zwei-Host-Abnahme wird nur mit `require_manual_runtime=true` verlangt. Ein automatischer grüner Lauf ersetzt diese Abnahme nicht. |
| `test-dmg` | Optionaler signierter Test-DMG-Build nach grünen Prüfungen, einschließlich Linux-Daemon. |
| `build-remote-server` | Optionaler separater Daemon-Build nach grünen Prüfungen; alternativ zum Test-DMG. |
| `stale` | Nur manueller Verwaltungsjob. |
| `release-tag-guard` / `zap_release` | Bestehende Tag-/Release-Auslösung unverändert; keine Release-Freigabe durch diese Umstellung. |

Es gibt keine zeitgesteuerten Workflows mehr. Ein neuer Batch auf demselben Ref
bricht einen noch laufenden Batch ab. Eine Auslösung auf einem anderen Branch
scheitert vor den schweren Jobs.

## Freigabe und Ablauf

1. Mehrere PRs nach den leichten Checks mergen.
2. Christian gibt genau den gewünschten Lauf frei:
   `release-approve "ci-batch.yml --ref main"`.
3. Erst danach `gho "$(getpat)" workflow run ci-batch.yml --ref main` ausführen.
   Ohne weitere Inputs entstehen keine DMG-/Daemon-Build-Artefakte.
   `artifacts=test-dmg` bzw. `artifacts=remote-server` muss Teil der Freigabe sein.
4. Bei rotem Batch erst korrigieren oder zurücknehmen, bevor weitere Änderungen
   gemergt werden. Ein Release darf nur von einem `main`-Commit mit grünem Batch
   erfolgen; die gesonderte Release-Freigabe bleibt erforderlich.

## Ruleset 18082412 – Christian ändert es selbst

- `cargo check (Linux x86_64)` aus den Pflicht-Checks entfernen.
- `strict_required_status_checks_policy` auf `false` setzen.
- `no-new-cjk` unverändert als Pflicht-Check behalten; nichts umbenennen.

Die Umstellung wird einmal gebündelt gepusht, erst nach Christians Zustimmung.
Dabei kann noch die bisherige PR-Konfiguration relevant sein; dieser Push ist
keine Freigabe für einen anschließenden Batch-Lauf.
