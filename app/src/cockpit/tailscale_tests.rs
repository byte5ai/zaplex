//! Tests for the Tailscale `status --json` → host-candidate parser.

use super::*;

const SAMPLE: &str = r#"{
  "Self": {
    "HostName": "my-laptop",
    "DNSName": "my-laptop.tail1234.ts.net.",
    "OS": "macOS",
    "Online": true,
    "TailscaleIPs": ["100.64.0.9", "fd7a:1::9"]
  },
  "Peer": {
    "nodekey:aaa": {
      "HostName": "devhost",
      "DNSName": "devhost.tail1234.ts.net.",
      "OS": "linux",
      "Online": true,
      "TailscaleIPs": ["100.64.0.1", "fd7a:1::1"]
    },
    "nodekey:bbb": {
      "HostName": "macmini",
      "DNSName": "macmini.tail1234.ts.net.",
      "OS": "macOS",
      "Online": false,
      "TailscaleIPs": ["100.64.0.2", "fd7a:1::2"]
    }
  }
}"#;

#[test]
fn excludes_self_and_extracts_peers() {
    let hosts = parse_tailscale_status(SAMPLE);
    assert_eq!(hosts.len(), 2, "Self is excluded; two peers remain");
    assert!(
        !hosts.iter().any(|h| h.hostname == "my-laptop"),
        "Self must not be a candidate"
    );
}

#[test]
fn strips_trailing_dot_and_picks_ipv4() {
    let hosts = parse_tailscale_status(SAMPLE);
    let devhost = hosts.iter().find(|h| h.hostname == "devhost").unwrap();
    assert_eq!(devhost.dns_name, "devhost.tail1234.ts.net"); // trailing dot gone
    assert_eq!(devhost.ipv4, "100.64.0.1"); // v4, not the fd7a:: v6
    assert_eq!(devhost.os, "linux");
    assert!(devhost.online);
}

#[test]
fn sorts_online_first_then_by_hostname() {
    let hosts = parse_tailscale_status(SAMPLE);
    // devhost (online) before macmini (offline), regardless of alpha order.
    assert_eq!(hosts[0].hostname, "devhost");
    assert_eq!(hosts[1].hostname, "macmini");
    assert!(hosts[0].online && !hosts[1].online);
}

#[test]
fn connect_host_prefers_dns_then_ipv4() {
    let with_dns = TailscaleHost {
        hostname: "h".into(),
        dns_name: "h.ts.net".into(),
        os: "linux".into(),
        online: true,
        ipv4: "100.64.0.7".into(),
    };
    assert_eq!(with_dns.connect_host(), "h.ts.net");
    let ip_only = TailscaleHost {
        dns_name: String::new(),
        ..with_dns.clone()
    };
    assert_eq!(ip_only.connect_host(), "100.64.0.7");
}

#[test]
fn drops_peers_without_any_address() {
    let json = r#"{"Peer":{"nodekey:x":{"HostName":"ghost","OS":"linux","Online":true,"TailscaleIPs":[]}}}"#;
    assert!(
        parse_tailscale_status(json).is_empty(),
        "a peer with no DNS name and no IP is not a usable candidate"
    );
}

#[test]
fn ipv6_only_peer_still_candidate_via_dns() {
    // No IPv4 in the list, but a MagicDNS name → still connectable.
    let json = r#"{"Peer":{"nodekey:x":{"HostName":"v6box","DNSName":"v6box.ts.net.","OS":"linux","Online":true,"TailscaleIPs":["fd7a:1::5"]}}}"#;
    let hosts = parse_tailscale_status(json);
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].ipv4, "", "no v4 address present");
    assert_eq!(hosts[0].connect_host(), "v6box.ts.net");
}

#[test]
fn empty_and_garbage_input_yield_no_candidates() {
    assert!(parse_tailscale_status("").is_empty());
    assert!(parse_tailscale_status("not json").is_empty());
    assert!(parse_tailscale_status("{}").is_empty(), "no Peer key → empty");
}
