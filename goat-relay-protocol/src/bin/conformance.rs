//! goat-relay-conformance — wire-format conformance tool for Block 33A.
//!
//! Two subcommands:
//!
//! - `gen <dir>` — emit a hand-crafted set of fixture files (one per
//!   message type plus edge cases) to `<dir>/`. Used to seed the
//!   smoke-tier fixture set.
//!
//! - `check <dir>` — for each `.bin` file under `<dir>/`, parse via
//!   [`Frame::parse`], re-serialize via [`Frame::marshal`], assert
//!   byte-for-byte equality. Emits one JSONL line per fixture to
//!   stdout. Exits with code 0 iff every fixture round-trips.
//!
//! This tool is the smoke-tier conformance soak driver. Validation
//! tier (run on generated random inputs) is exercised by the crate's
//! proptest tests via `cargo test`. Canonical tier (real captured
//! production traffic) replaces the synthetic fixtures with packets
//! captured from the bench env once a Rust relay binary exists at 33B.
//!
//! See `docs/design/netbird-relay-wire-protocol.md` §6 and
//! `ops/wave-33/relay-conformance/README.md`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use bytes::Bytes;
use goat_relay_protocol::framing::Frame;
use goat_relay_protocol::messages::{
    auth::{Auth, AuthResponse, Hello, HelloResponse, MAX_AUTH_PAYLOAD},
    payload::{Transport, MAX_TRANSPORT_PAYLOAD},
    peer_state::{marshal_peer_list, PeerListKind},
    PeerId, PEER_ID_SIZE,
};

fn main() {
    let mut args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        usage();
        process::exit(2);
    }
    let cmd = args.remove(1);
    let dir = PathBuf::from(args.remove(1));

    let exit = match cmd.as_str() {
        "gen" => cmd_gen(&dir),
        "check" => cmd_check(&dir),
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
            2
        }
    };
    process::exit(exit);
}

fn usage() {
    eprintln!(
        "usage:\n  \
         goat-relay-conformance gen <fixtures-dir>\n  \
         goat-relay-conformance check <fixtures-dir>"
    );
}

// ----- gen --------------------------------------------------------------

fn cmd_gen(dir: &Path) -> i32 {
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("failed to create {dir:?}: {e}");
        return 1;
    }

    let fixtures = build_fixtures();
    let mut written = 0usize;
    for (name, bytes) in &fixtures {
        let path = dir.join(name);
        if let Err(e) = fs::write(&path, bytes) {
            eprintln!("failed to write {path:?}: {e}");
            return 1;
        }
        written += 1;
    }
    println!("wrote {written} fixtures to {}", dir.display());
    0
}

/// Build the hand-crafted smoke-tier fixture set.
///
/// Goal: at least one fixture exercising each active message type +
/// boundary cases (empty payload, max payload, IPv6 in URLs, deprecated
/// Hello path).
fn build_fixtures() -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let p1 = peer(0x11);
    let p2 = peer(0x22);
    let p3 = peer(0x33);

    // ---- §3.4 Close (header-only) ----
    out.insert(
        "010-close.bin".into(),
        Frame::Close.marshal().unwrap(),
    );

    // ---- §3.5 HealthCheck (header-only) ----
    out.insert(
        "020-healthcheck.bin".into(),
        Frame::HealthCheck.marshal().unwrap(),
    );

    // ---- §3.3 Transport with empty payload ----
    out.insert(
        "030-transport-empty.bin".into(),
        Frame::Transport(Transport::new(p1, Bytes::new()).unwrap())
            .marshal()
            .unwrap(),
    );

    // ---- §3.3 Transport with small WireGuard-shaped payload ----
    out.insert(
        "031-transport-small.bin".into(),
        Frame::Transport(
            Transport::new(p1, Bytes::from_static(&[0xAA, 0xBB, 0xCC, 0xDD])).unwrap(),
        )
        .marshal()
        .unwrap(),
    );

    // ---- §3.3 Transport with typical WG MTU-sized payload (1380 bytes) ----
    out.insert(
        "032-transport-mtu1380.bin".into(),
        Frame::Transport(Transport::new(p2, Bytes::from(vec![0x55u8; 1380])).unwrap())
            .marshal()
            .unwrap(),
    );

    // ---- §3.3 Transport with MAX payload (boundary case for 8820-byte total) ----
    out.insert(
        "033-transport-max.bin".into(),
        Frame::Transport(
            Transport::new(p3, Bytes::from(vec![0xFFu8; MAX_TRANSPORT_PAYLOAD])).unwrap(),
        )
        .marshal()
        .unwrap(),
    );

    // ---- §3.1.1 Auth with token ----
    out.insert(
        "040-auth-typical.bin".into(),
        Frame::Auth(Auth::new(p1, Bytes::from_static(b"hmac-sha256-token")).unwrap())
            .marshal()
            .unwrap(),
    );

    // ---- §3.1.1 Auth with MAX payload (212-byte total boundary) ----
    out.insert(
        "041-auth-max.bin".into(),
        Frame::Auth(Auth::new(p2, Bytes::from(vec![0x77u8; MAX_AUTH_PAYLOAD])).unwrap())
            .marshal()
            .unwrap(),
    );

    // ---- §3.1.2 AuthResponse with typical instance URL ----
    out.insert(
        "050-authresponse-typical.bin".into(),
        Frame::AuthResponse(
            AuthResponse::new("rels://relay-a.goat.example:33073".to_owned()).unwrap(),
        )
        .marshal()
        .unwrap(),
    );

    // ---- §3.1.2 AuthResponse with IPv6 host (`[2001:db8::1]:443` URL form) ----
    out.insert(
        "051-authresponse-ipv6.bin".into(),
        Frame::AuthResponse(AuthResponse::new("rels://[2001:db8::1]:33073".to_owned()).unwrap())
            .marshal()
            .unwrap(),
    );

    // ---- §3.1.3 Hello (deprecated; parsed for parity) ----
    out.insert(
        "060-hello-deprecated.bin".into(),
        Frame::Hello(Hello {
            peer_id: p1,
            // Treat additions as opaque bytes — gob decoding is out of scope.
            additions: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
        })
        .marshal()
        .unwrap(),
    );

    // ---- §3.1.3 HelloResponse (deprecated) ----
    out.insert(
        "061-helloresponse-deprecated.bin".into(),
        Frame::HelloResponse(HelloResponse {
            additional_data: Bytes::from_static(b"opaque-server-bytes"),
        })
        .marshal()
        .unwrap(),
    );

    // ---- §3.2 SubscribePeerState with one peer ----
    out.insert(
        "070-subscribe-one.bin".into(),
        marshal_peer_list(PeerListKind::Subscribe, &[p1]).unwrap(),
    );

    // ---- §3.2 UnsubscribePeerState with three peers ----
    out.insert(
        "071-unsubscribe-three.bin".into(),
        marshal_peer_list(PeerListKind::Unsubscribe, &[p1, p2, p3]).unwrap(),
    );

    // ---- §3.2 PeersOnline (server-side) ----
    out.insert(
        "072-peersonline-three.bin".into(),
        marshal_peer_list(PeerListKind::Online, &[p1, p2, p3]).unwrap(),
    );

    // ---- §3.2 PeersWentOffline (server-side) ----
    out.insert(
        "073-peerswentoffline-three.bin".into(),
        marshal_peer_list(PeerListKind::Offline, &[p3]).unwrap(),
    );

    // ---- §3.2 SubscribePeerState at chunk boundary (244 peers — max for one msg) ----
    let peers_full: Vec<PeerId> = (0..244).map(|i| peer((i % 256) as u8)).collect();
    out.insert(
        "074-subscribe-244-peers.bin".into(),
        marshal_peer_list(PeerListKind::Subscribe, &peers_full).unwrap(),
    );

    out
}

fn peer(byte: u8) -> PeerId {
    let mut b = [0u8; PEER_ID_SIZE];
    b[..4].copy_from_slice(b"sha-");
    b[4..].fill(byte);
    PeerId::from_bytes(b)
}

// ----- check ------------------------------------------------------------

fn cmd_check(dir: &Path) -> i32 {
    let entries = match collect_bin_files(dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to scan {dir:?}: {e}");
            return 1;
        }
    };

    if entries.is_empty() {
        eprintln!("no .bin fixtures found in {}", dir.display());
        return 1;
    }

    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    let mut failures = 0usize;
    let mut passes = 0usize;

    for path in &entries {
        let result = check_one(path);
        let line = result.to_jsonl();
        let _ = writeln!(stdout, "{line}");
        if result.passed() {
            passes += 1;
        } else {
            failures += 1;
        }
    }
    let _ = stdout.flush();

    eprintln!(
        "{} fixtures: {passes} pass, {failures} fail",
        entries.len()
    );
    if failures == 0 {
        0
    } else {
        1
    }
}

fn collect_bin_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("bin") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Debug)]
enum CheckResult {
    Ok {
        fixture: String,
        size: usize,
        msg_type: String,
    },
    ParseFail {
        fixture: String,
        size: usize,
        error: String,
    },
    RemarshalFail {
        fixture: String,
        size: usize,
        msg_type: String,
        error: String,
    },
    ByteDrift {
        fixture: String,
        size: usize,
        msg_type: String,
        first_diff_offset: usize,
    },
    IoFail {
        fixture: String,
        error: String,
    },
}

impl CheckResult {
    fn passed(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    fn to_jsonl(&self) -> String {
        // Tiny ad-hoc JSON emitter — avoids dragging serde into the lib
        // crate just for this one binary. Field set is fixed and
        // controlled, no escape concerns beyond filename.
        match self {
            Self::Ok {
                fixture,
                size,
                msg_type,
            } => format!(
                r#"{{"fixture":"{}","outcome":"ok","size":{size},"msg_type":"{msg_type}"}}"#,
                escape(fixture)
            ),
            Self::ParseFail {
                fixture,
                size,
                error,
            } => format!(
                r#"{{"fixture":"{}","outcome":"parse_fail","size":{size},"error":"{}"}}"#,
                escape(fixture),
                escape(error)
            ),
            Self::RemarshalFail {
                fixture,
                size,
                msg_type,
                error,
            } => format!(
                r#"{{"fixture":"{}","outcome":"remarshal_fail","size":{size},"msg_type":"{msg_type}","error":"{}"}}"#,
                escape(fixture),
                escape(error)
            ),
            Self::ByteDrift {
                fixture,
                size,
                msg_type,
                first_diff_offset,
            } => format!(
                r#"{{"fixture":"{}","outcome":"byte_drift","size":{size},"msg_type":"{msg_type}","first_diff_offset":{first_diff_offset}}}"#,
                escape(fixture)
            ),
            Self::IoFail { fixture, error } => format!(
                r#"{{"fixture":"{}","outcome":"io_fail","error":"{}"}}"#,
                escape(fixture),
                escape(error)
            ),
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn check_one(path: &Path) -> CheckResult {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_owned();

    let original = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return CheckResult::IoFail {
                fixture: name,
                error: e.to_string(),
            }
        }
    };
    let size = original.len();

    let frame = match Frame::parse(&original) {
        Ok(f) => f,
        Err(e) => {
            return CheckResult::ParseFail {
                fixture: name,
                size,
                error: format!("{e}"),
            }
        }
    };

    let msg_type = frame_label(&frame);

    let remarshaled = match frame.marshal() {
        Ok(b) => b,
        Err(e) => {
            return CheckResult::RemarshalFail {
                fixture: name,
                size,
                msg_type,
                error: format!("{e}"),
            }
        }
    };

    if remarshaled != original {
        let first_diff = original
            .iter()
            .zip(remarshaled.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(original.len().min(remarshaled.len()));
        return CheckResult::ByteDrift {
            fixture: name,
            size,
            msg_type,
            first_diff_offset: first_diff,
        };
    }

    CheckResult::Ok {
        fixture: name,
        size,
        msg_type,
    }
}

fn frame_label(frame: &Frame) -> String {
    match frame {
        Frame::Auth(_) => "Auth",
        Frame::AuthResponse(_) => "AuthResponse",
        Frame::Hello(_) => "Hello",
        Frame::HelloResponse(_) => "HelloResponse",
        Frame::Transport(_) => "Transport",
        Frame::Close => "Close",
        Frame::HealthCheck => "HealthCheck",
        Frame::PeerList { kind, .. } => match kind {
            PeerListKind::Subscribe => "PeerList.Subscribe",
            PeerListKind::Unsubscribe => "PeerList.Unsubscribe",
            PeerListKind::Online => "PeerList.Online",
            PeerListKind::Offline => "PeerList.Offline",
        },
        // Frame is #[non_exhaustive]; future variants land here.
        _ => "Unknown",
    }
    .to_owned()
}
