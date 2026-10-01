use super::*;
use serde_json::json;

fn init_json(shell: &str, is_subshell: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "hook": "InitShell",
        "value": {"session_id": 12345, "shell": shell, "is_subshell": is_subshell}
    }))
    .unwrap()
}

fn bootstrapped_json(shell: &str) -> Vec<u8> {
    // These required string fields match the bundled shell emitters. A default
    // Rust value serializes optional fields as null, which is not their wire format.
    serde_json::to_vec(&json!({
        "hook": "Bootstrapped",
        "value": {
            "shell": shell, "histfile": "/home/test/.history", "home_dir": "/home/test",
            "path": "/usr/bin", "aliases": "", "abbreviations": "", "function_names": "",
            "env_var_names": "", "builtins": "", "keywords": "", "shell_version": "1"
        }
    }))
    .unwrap()
}

fn dcs(json: &[u8], hex_encoded: bool, terminator: &[u8]) -> Vec<u8> {
    let (marker, payload) = if hex_encoded {
        (b'd', hex::encode(json).into_bytes())
    } else {
        (b'f', json.to_vec())
    };
    [b"\x1bP$".as_slice(), &[marker], payload.as_slice(), terminator].concat()
}

fn handshake(shell: &str, terminator: &[u8]) -> Vec<u8> {
    [
        dcs(&init_json(shell, false), true, terminator),
        b"sourcing user rc\r\n".to_vec(),
        dcs(&bootstrapped_json(shell), true, terminator),
    ]
    .concat()
}

#[test]
fn bootstrap_preamble_freezes_without_client_ack_across_every_chunk_boundary() {
    for terminator in [b"\x9c".as_slice(), b"\x1b\\".as_slice()] {
        let bytes = handshake("bash", terminator);
        for split in 0..=bytes.len() {
            let mut preamble = BootstrapPreamble::new(bytes.len());
            preamble.capture(&bytes[..split]);
            if split < bytes.len() {
                assert_eq!(preamble.frozen(), None);
            }
            preamble.capture(&bytes[split..]);
            assert_eq!(preamble.frozen(), Some(bytes.as_slice()));
        }
    }
}

#[test]
fn bootstrap_preamble_survives_lost_ack_and_ring_eviction_in_a_large_output_chunk() {
    let bytes = handshake("zsh", b"\x9c");
    let output = [bytes.clone(), vec![b'x'; BOOTSTRAP_PREAMBLE_CAP_BYTES + 1]].concat();
    let mut preamble = BootstrapPreamble::new(BOOTSTRAP_PREAMBLE_CAP_BYTES);
    let mut ring = OutputRing::new(128);
    preamble.capture(&output);
    ring.append(&output);
    let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, true);
    assert_eq!(sent, bytes);
    assert_eq!(replay, vec![b'x'; 128]);
    assert!(base_seq > sent.len() as u64);

    // An obsolete or duplicate client report cannot replace the exact boundary.
    preamble.freeze(1);
    preamble.freeze(output.len() as u64);
    preamble.capture(b"more output");
    assert_eq!(preamble.frozen(), Some(sent.as_slice()));

    // Clients without preamble support still receive their normal replay window.
    let (_, legacy_replay, legacy_preamble) = plan_attach(&ring, &preamble, 0, false);
    assert!(legacy_preamble.is_empty());
    assert_eq!(legacy_replay, replay);
}

#[test]
fn bootstrap_preamble_accepts_shell_emitters_and_unencoded_init_only() {
    for shell in ["bash", "zsh", "fish", "pwsh"] {
        let bytes = [
            dcs(&init_json(shell, false), false, b"\x9c"),
            dcs(&bootstrapped_json(shell), true, b"\x9c"),
        ]
        .concat();
        let mut preamble = BootstrapPreamble::new(bytes.len());
        for byte in &bytes {
            preamble.capture(&[*byte]);
        }
        assert_eq!(preamble.frozen(), Some(bytes.as_slice()));
    }
    let bytes = [
        dcs(&init_json("bash", false), false, b"\x9c"),
        dcs(&bootstrapped_json("bash"), false, b"\x9c"),
    ]
    .concat();
    let mut preamble = BootstrapPreamble::new(bytes.len());
    preamble.capture(&bytes);
    assert_eq!(preamble.frozen(), None);
}

#[test]
fn bootstrap_preamble_accepts_complete_osc_json_and_kv_hooks() {
    for terminator in [b"\x07".as_slice(), b"\x1b\\".as_slice()] {
        let bytes = [
            b"\x1b]9278;d;".as_slice(),
            hex::encode(init_json("pwsh", false)).as_bytes(),
            terminator,
            b"\x1b]9278;d;",
            hex::encode(bootstrapped_json("pwsh")).as_bytes(),
            terminator,
        ]
        .concat();
        let mut preamble = BootstrapPreamble::new(bytes.len());
        for byte in &bytes {
            preamble.capture(&[*byte]);
        }
        assert_eq!(preamble.frozen(), Some(bytes.as_slice()));
    }
    let bytes = b"\x1b]9278;k;A;InitShell\x07\x1b]9278;k;B;session_id;12345\x07\x1b]9278;k;B;shell;$'bash'\x07\x1b]9278;k;C\x07\x1b]9278;k;A;Bootstrapped\x07\x1b]9278;k;B;shell;$'bash'\x07\x1b]9278;k;B;path;$'/bin;/usr/bin'\x07\x1b]9278;k;C\x07";
    let mut preamble = BootstrapPreamble::new(bytes.len());
    for byte in bytes {
        preamble.capture(&[*byte]);
    }
    assert_eq!(preamble.frozen(), Some(bytes.as_slice()));
}

#[test]
fn bootstrap_preamble_rejects_incomplete_malformed_aborted_or_nested_handshakes() {
    let init = dcs(&init_json("bash", false), true, b"\x9c");
    let boot = dcs(&bootstrapped_json("bash"), true, b"\x9c");
    let invalid_streams = [
        boot.clone(),
        [b"\x1b]9278;k;A;InitShell\x07\x1b]9278;k;B;shell;bash\x07\x1b]9278;k;C\x07".to_vec(), boot.clone()].concat(),
        [init.clone(), b"\x1b]9278;k;A;Bootstrapped\x07".to_vec(), boot.clone()].concat(),
        [boot.clone(), init.clone()].concat(),
        [init.clone(), boot[..boot.len() - 1].to_vec()].concat(),
        [init.clone(), dcs(b"not json", true, b"\x9c")].concat(),
        [init.clone(), dcs(br#"{"hook":"Bootstrapped","value":{"shell":"bash"}}"#, true, b"\x9c")].concat(),
        [init.clone(), dcs(&bootstrapped_json("bash"), true, b"\x18")].concat(),
        [init.clone(), dcs(&bootstrapped_json("bash"), true, b"\x1b[0m")].concat(),
        [init.clone(), dcs(&bootstrapped_json("zsh"), true, b"\x9c")].concat(),
        [init.clone(), dcs(&init_json("bash", true), true, b"\x9c"), boot.clone()].concat(),
        [init.clone(), init.clone(), boot.clone()].concat(),
        [dcs(&init_json("bash", true), true, b"\x9c"), boot.clone()].concat(),
        [init_json("bash", false), bootstrapped_json("bash")].concat(),
        [init.clone(), b"\x1b]9278;k;A;Bootstrapped\x07\x1b]9278;k;B;shell;bash\x07".to_vec()].concat(),
        [init, b"\x1b]9278;k;A;Bootstrapped\x07\x1b]9278;k;A;Bootstrapped\x07\x1b]9278;k;B;shell;bash\x07\x1b]9278;k;C\x07".to_vec()].concat(),
    ];
    for bytes in invalid_streams {
        let mut preamble = BootstrapPreamble::new(bytes.len());
        preamble.capture(&bytes);
        assert_eq!(preamble.frozen(), None, "invalid stream: {bytes:?}");
    }
}

#[test]
fn bootstrap_preamble_cap_never_serves_partial_handshake() {
    let bytes = handshake("bash", b"\x1b\\");
    let mut preamble = BootstrapPreamble::new(bytes.len() - 1);
    preamble.capture(&bytes);
    preamble.freeze(bytes.len() as u64);
    assert_eq!(preamble.frozen(), None);
    assert!(preamble.bytes.is_empty());
    assert!(preamble.handshake.dcs_data.is_empty());
    preamble.capture(&bytes);
    assert_eq!(preamble.frozen(), None);
}

#[test]
fn bootstrap_preamble_is_served_when_attach_limit_skips_retained_handshake() {
    let bytes = handshake("bash", b"\x9c");
    let mut preamble = BootstrapPreamble::new(bytes.len());
    preamble.capture(&bytes);
    let mut ring = OutputRing::new(ATTACH_REPLAY_MAX_BYTES + bytes.len());
    ring.append(&bytes);
    ring.append(&vec![b'x'; ATTACH_REPLAY_MAX_BYTES]);
    assert_eq!(ring.base_seq(), 0, "the ring still retains the handshake");

    let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, true);
    assert_eq!(sent, bytes);
    assert_eq!(base_seq, bytes.len() as u64);
    assert_eq!(replay.len(), ATTACH_REPLAY_MAX_BYTES);
    assert!(replay.iter().all(|byte| *byte == b'x'));

    let (_, _, sent) = plan_attach(&ring, &preamble, base_seq, true);
    assert!(sent.is_empty(), "ordinary reconnects do not re-bootstrap");
}
