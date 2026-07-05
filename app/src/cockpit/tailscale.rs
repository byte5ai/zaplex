//! Tailscale peer discovery (remote breadth, audit (d)#3).
//!
//! Parses `tailscale status --json` into a list of **host candidates** — the
//! machines on the user's tailnet that could be added as SSH servers. This is
//! the pure parsing spine (no process spawning, no UI): given the JSON, it
//! yields clean, deduped, online-first candidates. Spawning `tailscale` and
//! turning a chosen candidate into an `SshServerInfo` builds on top.
//!
//! Only the fields we need are deserialized; Tailscale adds many more and we
//! stay forward-compatible by ignoring the rest.

use serde::Deserialize;

/// A machine on the tailnet that could become an SSH host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscaleHost {
    /// Short hostname, e.g. `devhost`.
    pub hostname: String,
    /// Fully-qualified MagicDNS name (trailing dot stripped), e.g.
    /// `devhost.tail1234.ts.net` — empty when the tailnet has no MagicDNS.
    pub dns_name: String,
    /// Reported OS, e.g. `linux` / `macOS` / `windows`.
    pub os: String,
    /// Whether the peer is currently reachable.
    pub online: bool,
    /// The peer's first (IPv4) Tailscale address, e.g. `100.64.0.1`. The most
    /// reliable thing to SSH to; empty only for a malformed peer.
    pub ipv4: String,
}

impl TailscaleHost {
    /// The best address to connect to: the MagicDNS name when present (stable,
    /// human-readable), else the Tailscale IPv4. Empty only if the peer had
    /// neither — such peers are dropped by [`parse_tailscale_status`].
    pub fn connect_host(&self) -> &str {
        if !self.dns_name.is_empty() {
            &self.dns_name
        } else {
            &self.ipv4
        }
    }
}

/// Raw `tailscale status --json` shape — only the fields we consume.
#[derive(Debug, Deserialize)]
struct StatusDto {
    #[serde(rename = "Peer", default)]
    peer: std::collections::HashMap<String, PeerDto>,
}

#[derive(Debug, Deserialize)]
struct PeerDto {
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "Online", default)]
    online: bool,
    #[serde(rename = "TailscaleIPs", default)]
    tailscale_ips: Vec<String>,
}

/// Parse `tailscale status --json` into host candidates.
///
/// - `Self` is intentionally excluded — the local machine is not a *remote*
///   host to add.
/// - Peers with neither a MagicDNS name nor any Tailscale IP are dropped (no
///   usable address → no candidate, never a broken host entry).
/// - Results are sorted **online-first, then by hostname** so the reachable
///   machines surface at the top of the picker.
///
/// Returns an empty vec on any parse failure (caller surfaces "no peers found"
/// rather than an error dialog for a missing or malformed tailnet).
pub fn parse_tailscale_status(json: &str) -> Vec<TailscaleHost> {
    let Ok(status) = serde_json::from_str::<StatusDto>(json) else {
        return Vec::new();
    };
    let mut hosts: Vec<TailscaleHost> = status
        .peer
        .into_values()
        .filter_map(|p| {
            // First IP is the IPv4 (Tailscale lists v4 before v6).
            let ipv4 = p
                .tailscale_ips
                .into_iter()
                .find(|ip| ip.contains('.'))
                .unwrap_or_default();
            let dns_name = p.dns_name.trim_end_matches('.').to_string();
            // No address at all → not a usable candidate.
            if dns_name.is_empty() && ipv4.is_empty() {
                return None;
            }
            Some(TailscaleHost {
                hostname: p.host_name,
                dns_name,
                os: p.os,
                online: p.online,
                ipv4,
            })
        })
        .collect();
    hosts.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| a.hostname.cmp(&b.hostname))
    });
    hosts
}

#[cfg(test)]
#[path = "tailscale_tests.rs"]
mod tests;
