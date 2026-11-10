//! A stand-in repository for driving `goatbuilder federate` through the real
//! binary: a preflight, leaf minting and acceptance that pass, and an Ansible
//! wrapper that only records which play it was asked to run. Nothing here
//! touches a host. Shared by the federate integration tests.
//!
//! The federation verbs (phases 3, 4, 6, 7 and 9) run on either path the
//! driver has ([`Verbs`]): `goat federate` in a shell (`FED_VERBS=goat`),
//! stood in for by a `goat` that writes fixed files, or goatbuilder's own
//! in-process verbs (the default), which run for real. For those the world
//! supplies what a running system would: a context directory with a
//! declaration, an mTLS certificate and its CA, the test vectors' signing key
//! (ops/federation/vectors/testkeys), and `wg`, `docker`, `podman` and
//! `systemctl` stand-ins on PATH that answer init's read-only probes. The
//! counterparty's package is made the same way, by `goatbuilder federate init`
//! with the partner's own context, so every package and agreement is signed,
//! current and composes.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_goatbuilder")
}

fn real_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

pub fn exe(p: &Path, text: &str) {
    write(p, text);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Who runs the federation verbs in a world's runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbs {
    /// `FED_VERBS=goat`: `goat federate <verb>` in a shell, a stub here.
    Goat,
    /// The default (FED_VERBS unset): goatbuilder's verbs, in process.
    InProcess,
}

/// The transport public keys the `wg` stand-in reports: 32 bytes, base64.
const OUR_WG_KEY: &str = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE=";
const THEIR_WG_KEY: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

pub struct World {
    pub verbs: Verbs,
    pub root: tempfile::TempDir,
    pub repo: PathBuf,
    pub work: PathBuf,
    pub profile: PathBuf,
    pub log: PathBuf,
    /// While this file exists, the phase-12 play fails (exit 2).
    pub fail_12: PathBuf,
    /// While this file exists, the phase-12 play sleeps (to be killed).
    pub hang_12: PathBuf,
    /// While this file exists, `goat federate compose` (phase 6) sleeps.
    pub hang_6: PathBuf,
    /// While this file exists, init's router probe (phase 3, in process) sleeps.
    pub hang_3: PathBuf,
}

/// The world on the `goat` path (`FED_VERBS=goat`).
pub fn world() -> World {
    world_on(Verbs::Goat)
}

pub fn world_on(verbs: Verbs) -> World {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let repo = r.join("repo");
    let real = real_repo();
    // The declaration: the tutorial-3 fixture's config, which declares `hello-ally`.
    std::fs::create_dir_all(repo.join("ops/goatbuilder")).unwrap();
    std::fs::copy(
        real.join(
            "ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node/goatbuilder.yaml",
        ),
        repo.join("ops/goatbuilder/goatbuilder.yaml"),
    )
    .unwrap();
    let log = r.join("log");
    let fake = r.join("fake");
    write(
        &fake.join("ours.json"),
        r#"{"sovereign_id":"xaa","identity":{"ca_fingerprint":"sha256:00aa"}}"#,
    );
    write(
        &fake.join("agreement.json"),
        r#"{"spec":"federation-agreement/v1","transport":{"kind":"wireguard"}}"#,
    );
    let (l, f) = (log.display(), fake.display());
    let (fail_12, hang_12, hang_6, hang_3) = (
        r.join("fail-12"),
        r.join("hang-12"),
        r.join("hang-6"),
        r.join("hang-3"),
    );
    exe(
        &r.join("bin/goat"),
        &format!(
            "#!/bin/sh\necho \"goat $*\" >> '{l}'\nverb=$2; shift 2; out=\nwhile [ $# -gt 0 ]; do [ \"$1\" = --out ] && out=$2; shift; done\ncase $verb in\n init) cp '{f}/ours.json' \"$out\";;\n compose) [ -e '{h6}' ] && sleep 60; cp '{f}/agreement.json' \"$out\";;\n apply) mkdir -p \"$out/ops/federation/acl\"; echo rendered > \"$out/ops/federation/acl/xab-fragment.json5\";;\nesac\n",
            h6 = hang_6.display()
        ),
    );
    exe(
        &repo.join("ops/federation/standup/preflight.py"),
        &format!("#!/usr/bin/env python3\nopen('{l}','a').write('preflight\\n')\n"),
    );
    exe(
        &repo.join("ops/federation/standup/mint-peering-leaves.sh"),
        &format!("#!/bin/sh\necho mint >> '{l}'\nmkdir -p \"$2\"; echo cert > \"$2/core.pem\"; echo key > \"$2/core-key.pem\"\n"),
    );
    exe(
        &repo.join("ops/federation/standup/acceptance.sh"),
        &format!("#!/bin/sh\necho acceptance >> '{l}'\n"),
    );
    exe(
        &repo.join("ops/scripts/with-aws"),
        "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done; shift; exec \"$@\"\n",
    );
    exe(
        &repo.join("ops/scripts/ansible-preflight.sh"),
        &format!(
            "#!/bin/sh\nfor a; do last=$a; done\necho \"play $last\" >> '{l}'\nif [ \"$last\" = peering-zenoh-edge.yml ]; then\n  [ -e '{}' ] && exit 2\n  [ -e '{}' ] && sleep 60\nfi\nexit 0\n",
            fail_12.display(),
            hang_12.display()
        ),
    );
    std::fs::create_dir_all(repo.join("ops/ansible")).unwrap();
    let work = r.join("work").join("hello-ally");
    std::fs::create_dir_all(&work).unwrap();
    write(
        &work.join("theirs.canonical.json"),
        r#"{"sovereign_id":"xab","identity":{"ca_fingerprint":"sha256:00ab"}}"#,
    );
    write(&work.join("theirs-ca.pem"), "their ca\n");
    let profile = r.join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    if verbs == Verbs::InProcess {
        in_process_stand_in(r, &profile, &work, &log, &hang_3);
    }
    World {
        verbs,
        hang_3,
        repo,
        work,
        profile,
        log,
        fail_12,
        hang_12,
        hang_6,
        root,
    }
}

impl World {
    pub fn command(&self, extra: &[&str]) -> Command {
        self.command_with(&[], extra)
    }

    /// `global` goes before `federate` (e.g. `--mock <dir>`).
    pub fn command_with(&self, global: &[&str], extra: &[&str]) -> Command {
        let r = self.root.path();
        let mut args: Vec<String> = global.iter().map(|s| s.to_string()).collect();
        args.extend([
            "-C".into(),
            self.repo.display().to_string(),
            "--yes".into(),
        ]);
        if !global.contains(&"-o") {
            args.extend(["-o".into(), "json".into()]);
        }
        args.push("federate".into());
        args.extend(extra.iter().map(|s| s.to_string()));
        let mut c = Command::new(bin());
        c.args(&args)
            .env_clear()
            .env("HOME", r)
            .env("GOAT_BIN", r.join("bin/goat"));
        match self.verbs {
            // The world stands in for `goat` with a stub; since the verbs run in
            // goatbuilder by default, name the stub path so the run uses it.
            Verbs::Goat => {
                c.env("PATH", std::env::var("PATH").unwrap())
                    .env("FED_VERBS", "goat");
            }
            // The default path: FED_VERBS unset. The probe stand-ins come
            // first on PATH, and phase 3 signs with the test vectors' key.
            Verbs::InProcess => {
                c.env("PATH", tools_path(r))
                    .env("FED_SIGNING_KEY", r.join("keys/xaa.seed.hex"));
            }
        }
        c.env("GOAT_PROFILE_DIR", &self.profile)
            .env("GOAT_NO_FUN", "1")
            .env("FED_WORK", &self.work)
            .env("FED_NO_DIRECTORY", "1")
            .current_dir(&self.repo)
            .stdin(std::process::Stdio::null());
        c
    }

    pub fn run(&self, extra: &[&str]) -> (Output, serde_json::Value) {
        self.run_with(&[], extra)
    }

    pub fn run_with(&self, global: &[&str], extra: &[&str]) -> (Output, serde_json::Value) {
        let out = self.command_with(global, extra).output().unwrap();
        let v = report(&out.stdout);
        (out, v)
    }

    /// The keyring phase 6 reads: on the goat path any two lines (the stub
    /// ignores them); in process, the test vectors' public keys, which sign
    /// the two packages.
    pub fn write_keys(&self) {
        let text = match self.verbs {
            Verbs::Goat => "xaa=QUJD\nxab=QUJE\n".to_string(),
            Verbs::InProcess => ["xaa", "xab"]
                .iter()
                .map(|p| format!("{p}={}\n", testkey(&format!("{p}.public.b64")).trim()))
                .collect(),
        };
        std::fs::write(self.work.join("keys.txt"), text).unwrap();
    }

    /// Whether the federation verb ran since the log was cleared: on the goat
    /// path the stub logs its command line; in process, the verb's own report
    /// precedes the run's JSON on stdout, and init's probes are logged.
    pub fn verb_ran(&self, out: &Output, verb: &str) -> bool {
        match self.verbs {
            Verbs::Goat => self.log().contains(&format!("goat federate {verb}")),
            Verbs::InProcess => {
                let text = String::from_utf8_lossy(&out.stdout);
                let before_report = text.rfind("\n{").map_or(&text[..], |i| &text[..i]);
                // Each verb's report opens with its own line
                // (crates/goat-federation/src/verbs/); init's is its probes.
                let opens = |prefix: &str| before_report.lines().any(|l| l.starts_with(prefix));
                match verb {
                    "init" => self.log().contains("probe wg show wg-fed0 public-key"),
                    "lint" => opens("package "),
                    "compose" => before_report.lines().any(|l| l.contains("] compose — ")),
                    "verify" => opens("verify "),
                    "apply" => opens("apply "),
                    _ => panic!("verb_ran: no report line known for {verb}"),
                }
            }
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn clear_log(&self) {
        let _ = std::fs::remove_file(&self.log);
    }

    pub fn state(&self) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(self.work.join("state.json")).unwrap())
            .unwrap()
    }
}

/// The run's JSON report: the whole of stdout on the goat path; in process,
/// the verbs' own reports come first on stdout, so the report is the last
/// part of it that parses as one JSON value.
pub fn report(stdout: &[u8]) -> serde_json::Value {
    if let Ok(v) = serde_json::from_slice(stdout) {
        return v;
    }
    let text = String::from_utf8_lossy(stdout);
    let starts = text
        .match_indices("\n{")
        .map(|(i, _)| i + 1)
        .collect::<Vec<_>>();
    for i in starts {
        if let Ok(v) = serde_json::from_str(&text[i..]) {
            return v;
        }
    }
    serde_json::Value::Null
}

fn real_testkeys() -> PathBuf {
    real_repo().join("ops/federation/vectors/testkeys")
}

fn testkey(name: &str) -> String {
    std::fs::read_to_string(real_testkeys().join(name)).unwrap()
}

fn tools_path(root: &Path) -> String {
    format!(
        "{}:{}",
        root.join("tools").display(),
        std::env::var("PATH").unwrap()
    )
}

/// A CA and an mTLS certificate whose URI SAN names `trust_domain`, as
/// `goat profile init` leaves them: `mtls.cert.pem` and `ca-roots.pem` in `dir`.
fn credentials(dir: &Path, trust_domain: &str) {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(vec![]).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.distinguished_name
        .push(DnType::CommonName, format!("{trust_domain} root"));
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let mut leaf = CertificateParams::new(vec![]).unwrap();
    leaf.distinguished_name.push(DnType::CommonName, "operator");
    leaf.subject_alt_names = vec![SanType::URI(
        format!("spiffe://{trust_domain}/operator")
            .try_into()
            .unwrap(),
    )];
    leaf.use_authority_key_identifier_extension = true;
    let leaf_cert = leaf
        .signed_by(&KeyPair::generate().unwrap(), &ca_cert, &ca_key)
        .unwrap();
    write(&dir.join("mtls.cert.pem"), &leaf_cert.pem());
    write(&dir.join("ca-roots.pem"), &ca_cert.pem());
}

/// One party's declaration (`federation.toml`): a WireGuard offer on
/// `wg-fed0` and a data offer. Exactly one of the two parties listens.
fn declaration(dir: &Path, sovereign: &str, endpoint: &str, listener: bool, release: &str) {
    write(
        &dir.join("federation.toml"),
        &format!(
            "sovereign_id = \"{sovereign}\"\nterm_days = 30\ncut_contact = \"fed-ops@example.org\"\n\n\
             [transport]\ninterface = \"wg-fed0\"\nendpoint = \"{endpoint}\"\nlistener = {listener}\n\
             proposed_link_subnet = \"169.254.77.0/31\"\nmeasured_mtu = 1380\n\n\
             [data]\nrelease = [\"{release}\"]\naccept_hz = 100\naccept_bytes_per_message = 1048576\n\
             accept_distinct_keys = 5000\n"
        ),
    );
}

/// What a running system answers to init's read-only probes, for the
/// in-process path: our context (xaa), the partner's package (xab, made by
/// `goatbuilder federate init` with its own context), the probe stand-ins
/// and the signing key.
fn in_process_stand_in(r: &Path, profile: &Path, work: &Path, log: &Path, hang_3: &Path) {
    let (l, h3) = (log.display(), hang_3.display());
    let tools = r.join("tools");
    // `wg`: one interface up, and its public key ($STUB_WG_KEY for the partner).
    exe(
        &tools.join("wg"),
        &format!(
            "#!/bin/sh\necho \"probe wg $*\" >> '{l}'\ncase \"$*\" in\n 'show interfaces') echo wg-fed0;;\n \
             'show wg-fed0 public-key') echo \"${{STUB_WG_KEY:-{OUR_WG_KEY}}}\";;\n *) exit 1;;\nesac\n"
        ),
    );
    // `docker`: one running core router, its image and its zenohd version.
    // While hang-3 exists, the listing sleeps (phase 3, to be killed).
    exe(
        &tools.join("docker"),
        &format!(
            "#!/bin/sh\necho \"probe docker $1\" >> '{l}'\ncase $1 in\n \
             ps) [ -e '{h3}' ] && sleep 60; printf 'c0ffee\\tgoat-zenoh-router\\tzenoh-core\\n';;\n \
             inspect) printf 'c0ffee\\ttrue\\tzenoh:1.9.0\\n';;\n \
             exec) echo 'zenohd v1.9.0';;\n *) exit 1;;\nesac\n"
        ),
    );
    // No podman, so a podman on the machine running the tests is not asked.
    exe(&tools.join("podman"), "#!/bin/sh\nexit 127\n");
    // `systemctl is-active rosenpass`: not active.
    exe(
        &tools.join("systemctl"),
        "#!/bin/sh\n[ \"$1\" = is-active ] && { echo inactive; exit 3; }\nexit 1\n",
    );
    for p in ["xaa", "xab"] {
        write(
            &r.join(format!("keys/{p}.seed.hex")),
            &testkey(&format!("{p}.seed.hex")),
        );
    }
    let ours = profile.join("contexts/default");
    credentials(&ours, "xaa.example");
    declaration(&ours, "xaa", "192.0.2.10:51820", true, "release/weather/**");
    let theirs = r.join("partner-context");
    credentials(&theirs, "xab.example");
    declaration(
        &theirs,
        "xab",
        "198.51.100.20:51820",
        false,
        "release/terrain/**",
    );
    let out = Command::new(bin())
        .args(["federate", "init", "--context-dir"])
        .arg(&theirs)
        .args(["--with-transport", "--with-data", "--signing-key"])
        .arg(r.join("keys/xab.seed.hex"))
        .arg("--out")
        .arg(work.join("theirs.canonical.json"))
        .env_clear()
        .env("PATH", tools_path(r))
        .env("HOME", r)
        .env("GOAT_NO_FUN", "1")
        .env("STUB_WG_KEY", THEIR_WG_KEY)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the partner's package: {}",
        says(&out)
    );
    let _ = std::fs::remove_file(log);
}

pub fn rows<'a>(v: &'a serde_json::Value, name: &str) -> Vec<&'a str> {
    v["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|c| c["name"] == name)
                .filter_map(|c| c["detail"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// The phases that ran (each prints its banner row).
pub fn phases_run(v: &serde_json::Value) -> Vec<u32> {
    rows(v, "phase")
        .iter()
        .filter_map(|d| {
            d.strip_prefix("=== Phase ")
                .and_then(|r| r.split(' ').next())
                .and_then(|n| n.parse().ok())
        })
        .collect()
}

pub fn says(out: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
