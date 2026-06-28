# Phase B3 Design — Native UDP Transport (mosh-grade)

> **Status:** Design + reserved groundwork only. **NOT runtime-ready.** Created 2026-06-28.
> **Parent:** [native-remote-session-layer.md](../plans/2026-06-24-native-remote-session-layer.md) §8.
> **Honesty note:** a full mosh-grade transport is a large, networking-heavy subsystem that is **not** verifiable without a real client/host pair on a lossy/roaming network. This document is the design + the additive, compile-safe groundwork (a reserved capability name); it deliberately does **not** ship a half-working UDP datapath.

---

## 1. Why B3 (and why it's last)

B2 (Stages 2–4) already delivers the must-have: persistence, re-attach + replay, multi-session. B3 changes only the **transport beneath the session protocol** — from "SSH ControlMaster + proxy-stdio" to native UDP — to add **roaming** (survive IP changes) and **low latency** (predictive echo). The session protobuf protocol is unchanged. Order is strictly B2 → B3 (§8).

## 2. Model (mosh-derived)

1. **Bootstrap over SSH (reuse B2):** the existing headless ControlMaster connect (`headless_connect`) starts the daemon and, in the B3 case, also has the daemon mint a per-session **AEAD key** (e.g. AES-GCM/OCB) + a UDP port, returned over the secure SSH channel. No UDP port is open/usable without the key (mosh's security model).
2. **Switch to UDP:** thereafter the client speaks the *same* `ClientMessage`/`ServerMessage` protocol, but each datagram is AEAD-sealed under the session key and sent over UDP instead of the length-prefixed stdio stream.
3. **SSP-style state sync:** the protocol's existing monotonic `seq` (already used for ring replay) is the sync cursor — the client acks the last `seq` it has; the server sends only the delta. This is exactly the Stage 3 replay primitive, now driving steady-state UDP sync rather than just reconnect.
4. **Roaming for free:** the session is keyed by the AEAD key, not the source IP, so an IP change (Wi-Fi→cellular) just continues — the server accepts datagrams that authenticate, from any address.
5. **Predictive local echo:** client-side prediction of keystroke echo/cursor (mosh heuristic), reconciled against server confirmations.

## 3. Where it slots in (seams)

- **Transport trait:** `remote_server::transport::RemoteTransport` is already the abstraction (`SshTransport` is the B2 impl). B3 is a second impl, e.g. `UdpTransport`, selected at connect time. `daemon_tty` / `RemoteServerManager::connect_session<T: RemoteTransport>` are already generic over it — no change to the session layer.
- **Capability negotiation:** `InitializeResponse.features` already carries the capability list (Stage 0). B3 adds `FEATURE_UDP_TRANSPORT` (reserved now in `zaplex_remote_session::types`). The daemon advertises it only once implemented; the client upgrades SSH→UDP only when both sides advertise it, else stays on B2 (graceful, never a hard dependency).
- **Feature gate:** a `warp_features` flag (e.g. `NativeUdpTransport`) gates the client-side upgrade attempt, so it can ship dark and be enabled per-channel.

## 4. What's reserved now (this commit)

- `zaplex_remote_session::types::FEATURE_UDP_TRANSPORT = "udp-transport"` — the negotiation name, documented as not-yet-advertised. `supported_features()` is unchanged (still honest: daemon does not claim UDP).

That is the *entire* safe, additive footprint. Everything below is **remaining work**, not done here.

## 5. Remaining work (the real subsystem — needs a real network to build/verify)

- UDP socket lifecycle on both ends; datagram framing + MTU/fragmentation handling.
- AEAD seal/open (key from the SSH bootstrap), replay-window/nonce management.
- SSP-style delta sync loop on top of `seq` (sender keeps per-client acked-seq; resend/RTT/heartbeat; congestion/timeout policy).
- Roaming: accept authenticated datagrams from new source addresses; rebind.
- Predictive echo engine + reconciliation (the latency feel; substantial UI-side logic).
- Daemon: open UDP listener per session, hand key+port back over SSH at `OpenSession`/`Initialize`.
- Security review (key handling, nonce reuse, DoS on the open UDP port) before enabling.
- **Verification:** integration over a real lossy/roaming link — cannot be unit/headless-tested meaningfully; needs a client/host pair, packet loss injection, and IP-change testing.

## 6. Recommendation

Land B3 as a dedicated, separately-reviewed effort once B2 (Stages 2–4) has been exercised on real hosts (the GUI/real-host E2E). The reserved capability name keeps the door open without committing unverified code. Until then, B2's SSH-ControlMaster transport is the shipping path; it already gives persistence + re-attach (the user-visible payoff). Roaming/latency are the upside B3 adds.
