# Release Configurations

## Test-DMG: schneller Entwicklungszyklus

`test-dmg.yml` baut nach dem Signing-Vorabcheck die macOS-App und den
Linux-Daemon parallel. Ein abschließender macOS-Job übernimmt die vollständige,
unsignierte App, ergänzt den Daemon samt geprüftem SHA-256-Manifest und signiert,
notarisiert und verpackt das DMG. In diesem Job findet keine Kompilierung statt.

- `fast=false` (Standard): Release-Profile für Abnahmeprüfungen.
- `fast=true`: Debug-Profile für **App und Daemon**, für schnelle Funktionsprüfungen.
  Das Laufzeitverhalten und die Performance können vom Release-Build abweichen.
- Rust- und Daemon-Caches sind nach Profil getrennt. Rust-Caches werden auf allen
  Branches gespeichert; PR-Merge-Refs und Tags lesen vorhandene Caches.

Das Bundle-Skript verwendet dafür `--prepare-only` und `--package-only`, jeweils
mit demselben `--channel`, explizitem `--arch` und gegebenenfalls `--debug`.
Das normale Bundle-Kommando führt weiterhin alle Schritte aus. Die Skripttests
(`python3 script/test-macos-bundle`) prüfen den Übergang ohne echte Builds.

Für Zeitvergleiche die gesamte Workflow-Dauer und die getrennten Schritte für
App-Vorbereitung, Daemon-Build, Artefakttransfer und Signieren/Notarisieren erfassen.
Leere und gefüllte Caches sowie Debug- und Release-Profile getrennt vergleichen;
der erste Lauf mit den neuen Cache-Schlüsseln kann noch keine vorhandenen
profilspezifischen Rust-Caches nutzen.

## Release-Konfigurationsdatei

This README file documents the format of the `release_configurations.json` file located in this directory.  The file defines Zap's various release channels, and provides values for the various variables that are necessary to run the `create_new_releases.yml` GitHub workflow.

At some point, we may want to replace this document with a JSON schema file (which could be used to validate the correctness of the configuration as part of PR presubmit).

## Fields

* **channel**: The channel's unique identifier
* **type**: The release cadence.  At present, the valid values are "nightly" or "weekly".
* **is_prerelease**: If true, the GitHub release for this channel will be marked as prerelease.
* **is_autopush**: If true, this channel uses the "latest" keyword in `channel_versions.json` to automatically deploy new release candidates.  Non-autopush channels require a manual change in order to deploy them.
* **release_base_name**: The base name of GitHub releases created for this channel.
* **release_body_text**: The body text for GitHub releases created for this channel.
* **changelog_slack_channel**: The Slack channel where new changelogs will be posted whenever a new release candidates is cut.
* **gcs_cache_control_value**: The value of the cache-control response header for release DMGs.
  - **IMPORTANT!!**: the value of the cache-control header _must_ be all lowercase; uppercase values will not be respected by Cloud CDN.
