# Daemon-Versions-Selbstheilung (RC-Paket, 2026-07-19)

**Problem (RC-Abnahme):** Ein laufender Alt-Daemon wird vom Proxy versionslos
wiederverwendet (`proxy.rs`: PID-File lesen, `kill(pid,0)`, an dessen Socket
bridgen). Daemon-seitige Fixes wirken dann „nicht gefixt", und weder Client
noch UI können den Skew sehen. User-Auftrag: erkennen **bevor** man es sieht,
selbst heilen — ohne das Persistenz-Versprechen (Sessions überleben) zu
brechen.

**Baustand-Befund (gegen Code verifiziert, nicht geraten):**
- `InitializeResponse.server_version` existiert im Protokoll; der Daemon füllt
  es bereits (`ChannelState::app_version()`, leer für Source-Builds —
  `server_model.rs handle_initialize`).
- Der Client PRÜFT bereits (`manager.rs version_is_compatible`) — aber
  `should_enforce_remote_version_check` schaltet die Prüfung für
  `Channel::Oss` (= zaplex) pauschal AB. Historischer Grund: Source-Builds
  ohne Tag + wiederverwendetes Upstream-Release-Binary → Delete/Reinstall-
  Schleife. Für Release-DMGs (Tag vorhanden, versionierte Binary-Slots) gilt
  der Grund nicht.
- Der Mismatch-Pfad löscht das **Binary auf Platte** („stale binary") — die
  Recovery-Semantik der Upstream-Channels. Bei zaplex-Release-Builds ist das
  On-Disk-Binary per Konstruktion das richtige (Install-Slot und Socket-Name
  keyen auf denselben Tag); stale ist der **laufende Prozess**. Löschen wäre
  das falsche Objekt.
- **Idle-Exit existiert vollständig:** `GRACE_PERIOD` (10 min) bei
  0 Verbindungen + 0 Sessions (`deregister_connection`/`start_grace_timer`),
  plus GC-Sweep alle 5 min (`gc_sessions` + `maybe_arm_grace_after_gc`,
  unconditional pro Tick) mit 24-h-Reap für verlassene Sessions und
  Ring-RAM-Cap. Lücke: die direkten Session-Ende-Pfade
  (`handle_close_session`, `on_session_reader_eof`) armieren den Timer nicht
  selbst — Retirement passiert erst über den nächsten GC-Tick (≤ 5 min
  später). Kein Leak, aber unnötige Latenz.

## Mechanik (drei Teile, alle klein)

### 1. Versionierter Rendezvous-Pfad (der strukturelle Fix)
`server-<tag>.sock` / `server-<tag>.pid` statt `server.sock`/`server.pid` —
EINE gemeinsame Namensquelle `setup::daemon_runtime_filename(ext)`, benutzt
von Proxy (bind/lookup), Daemon (bind/cleanup) und den Client-Accessors in
`ssh_transport.rs`. Keying: `Channel::Oss` **und** `app_version() = Some(tag)`
→ versioniert; sonst (Source-Builds, Nicht-Oss-Channels) unverändert legacy —
der `deploy_remote_server`-Dev-Loop und Upstream-Verhalten bleiben unberührt
(bewusst enger gekeyt als `remote_server_binary`, das die meisten Nicht-Oss-
Binary-Pfade versioniert). Das Versions-Segment ist **längenbegrenzt** (max.
20 Zeichen; längere oder unsaubere Tags kollabieren deterministisch zu
`prefix-<fnv64>`): der Socket-Pfad muss in `sockaddr_un.sun_path` (104/108
Bytes) passen — codex-Fund im Review.

Wirkung: Ein v0.rc3-Client KANN keinen v0.rc2-Daemon mehr erwischen — der
Proxy (selbst das versionierte Binary) findet nur den arteigenen Socket und
spawnt sonst frisch. Der Alt-Daemon bleibt nur unter seinem Alt-Namen
erreichbar (also für Clients seines eigenen Releases) und stirbt über
Grace/GC, sobald seine letzte Session endet.

### 2. Handshake-Enforcement einschalten (Gürtel + Hosenträger)
`should_enforce_remote_version_check`: `Oss → app_version().is_some()`
(Release erzwingt, Source-Build überspringt weiter). Mismatch-Pfad: für Oss
KEIN Binary-Löschen (Upstream-Zweig unverändert), sondern Connect-Fehler mit
klarer Meldung → läuft über den bestehenden `SessionConnectionFailed`-Pfad in
den Tab (`on_connect_failed`) und den Classic-SSH-Fallback. Mit Teil 1 ist
das ein Assert auf „unmöglich" — feuert es doch, war es ein Pfad-Bug, und
zaplex sagt es, statt still alten Code zu servieren.

### 3. Idle-Exit-Härtung (Latenz raus)
`maybe_arm_grace_after_gc` → `maybe_arm_grace_when_idle`; zusätzlich zu GC
auch von `handle_close_session` und `on_session_reader_eof` aufgerufen (ctx
wird durchgereicht; beide Call-Sites haben es). Verhalten unverändert, nur
sofort statt ≤ 5 min später.

## Übergang / bewusste Grenzen
- Der HEUTE laufende Alt-Daemon (v0.rc-cockpit, Legacy-Socket) wird von
  neuen Clients schlicht nie mehr gefunden; er räumt sich nach Ende seiner
  letzten Session über seine eigene Grace/GC-Logik weg. Für die laufende
  RC-Abnahme bleibt der einmalige manuelle Kill der schnellste Weg.
- Alt-Sessions eines Alt-Daemons sind für den neuen Client **unsichtbar**
  (er spricht nur seinen eigenen Daemon). Sichtbarkeit + Ein-Klick-Migration
  („N Sessions laufen noch auf vX — beenden & upgraden") = eigenes
  Follow-up-Paket (Cockpit-UI), hier bewusst NICHT enthalten.
- PTY-Handover zwischen Daemon-Versionen (FD-Passing): explizit out of scope.

## Abnahme
1. Neuer Daemon + neuer Client → Socket heißt `server-<tag>.sock`, Connect
   wie gehabt (Regression: Bootstrap, Reattach, Adopt).
2. Alt-Daemon läuft weiter → neuer Connect erreicht ihn NICHT (frischer
   Daemon), Alt-Daemon exit nach Ende seiner Sessions (Log: grace timer).
3. Dev-Loop (`cargo run` + `deploy_remote_server`): unverändert
   `server.sock`, keine Enforcement-Fehler.
4. Session schließen als letzte bei 0 Clients → Grace-Timer-Log sofort,
   nicht erst nach GC-Tick.
