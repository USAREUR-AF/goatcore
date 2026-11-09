//! A stand-in repository for driving `goatbuilder federate` through the real
//! binary: a `goat` that writes fixed files, a preflight, leaf minting and
//! acceptance that pass, and an Ansible wrapper that only records which play it
//! was asked to run. Nothing here touches a host. Shared by the federate
//! integration tests.

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

pub fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(b))
}

pub fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn vectors() -> PathBuf {
    real_repo().join("ops/federation/vectors")
}

pub fn seed_hex(party: &str) -> String {
    std::fs::read_to_string(vectors().join(format!("testkeys/{party}.seed.hex")))
        .unwrap()
        .trim()
        .to_string()
}

pub fn public_b64(party: &str) -> String {
    std::fs::read_to_string(vectors().join(format!("testkeys/{party}.public.b64")))
        .unwrap()
        .trim()
        .to_string()
}

/// A conformance-vector package, with its CA fingerprint replaced, signed
/// again with the committed test seed (worthless keys, public on purpose).
pub fn resigned(party: &str, fp: &str) -> Vec<u8> {
    use base64::Engine;
    use goat_federation::json::{canonical, parse_document, JVal};
    let bytes =
        std::fs::read(vectors().join(format!("compose-full/package-{party}.canonical.json")))
            .unwrap();
    let JVal::Obj(mut top) = parse_document(&bytes).unwrap().without_member("signature") else {
        panic!()
    };
    for (k, v) in top.iter_mut() {
        if let (true, JVal::Obj(id)) = (k == "identity", v) {
            for (ik, iv) in id.iter_mut() {
                if ik == "ca_fingerprint" {
                    *iv = JVal::Str(fp.into());
                }
            }
        }
    }
    let hex = seed_hex(party);
    let seed: [u8; 32] = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect::<Vec<u8>>()
        .try_into()
        .unwrap();
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let sig = ed25519_dalek::Signer::sign(&sk, canonical(&JVal::Obj(top.clone())).as_bytes());
    top.push((
        "signature".into(),
        JVal::Str(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())),
    ));
    canonical(&JVal::Obj(top)).into_bytes()
}

pub const FRAGMENT: &str = "// GENERATED\n##ACL-ORG-REGION storages\n##ACL-ORG-REGION write-rules\n      {\n        id: \"fed-w-xab\",\n        messages: [\"put\", \"delete\"],\n        flows: [\"ingress\"],\n        permission: \"allow\",\n        key_exprs: [\n          \"from/xab/**\",\n        ],\n      },\n##ACL-ORG-REGION subjects\n      {\n        id: \"fed-xab\",\n        cert_common_names: [\"fed.xab.xaa.01\"],\n      },\n##ACL-ORG-REGION policies\n";

pub struct World {
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
}

pub fn world() -> World {
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
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(
        fake.join("ours.json"),
        resigned("xaa", &sha256_hex(b"our-ca-der")),
    )
    .unwrap();
    let (our_fp, their_fp) = (sha256_hex(b"our-ca-der"), sha256_hex(b"fake-ca-der"));
    write(
        &fake.join("agreement.json"),
        &format!(
            r#"{{"spec":"federation-agreement/v1","transport":{{"kind":"wireguard"}},"parties":[{{"sovereign_id":"xaa","ca_fingerprint":"{our_fp}"}},{{"sovereign_id":"xab","ca_fingerprint":"{their_fp}"}}]}}"#
        ),
    );
    write(&fake.join("fragment.json5"), FRAGMENT);
    // Our core's org fragment, with another vendor and another partner already in it.
    write(&r.join("core/org.json5"), "##ACL-ORG-REGION storages\n##ACL-ORG-REGION write-rules\n      { id: \"vendor-a\", messages: [\"put\"], flows: [\"ingress\"], permission: \"allow\", key_exprs: [\"vendor/a/**\"] },\n      // BEGIN peering xac (goatbuilder federate)\n      { id: \"fed-w-xac\", messages: [\"put\"], flows: [\"ingress\"], permission: \"allow\", key_exprs: [\"from/xac/**\"] },\n      // END peering xac\n##ACL-ORG-REGION subjects\n##ACL-ORG-REGION policies\n");
    // Our core's access control seed, for phase 10 by the command.
    std::fs::create_dir_all(repo.join("ops/ansible/roles/goat-zenoh/files")).unwrap();
    std::fs::copy(
        real.join("ops/ansible/roles/goat-zenoh/files/acl-assemble.py"),
        repo.join("ops/ansible/roles/goat-zenoh/files/acl-assemble.py"),
    )
    .unwrap();
    write(&r.join("core/seed.json5"), "{\n  // BEGIN: per-vendor storages\n  // END: per-vendor storages\n  // BEGIN: per-vendor write rules\n  // END: per-vendor write rules\n  // BEGIN: per-vendor subjects\n  // END: per-vendor subjects\n  // BEGIN: per-vendor policies\n  // END: per-vendor policies\n}\n");
    let (l, f) = (log.display(), fake.display());
    let (fail_12, hang_12, hang_6) = (r.join("fail-12"), r.join("hang-12"), r.join("hang-6"));
    exe(
        &r.join("bin/goat"),
        &format!(
            "#!/bin/sh\necho \"goat $*\" >> '{l}'\nverb=$2; shift 2; out=\nwhile [ $# -gt 0 ]; do [ \"$1\" = --out ] && out=$2; shift; done\ncase $verb in\n init) cp '{f}/ours.json' \"$out\";;\n compose) [ -e '{h6}' ] && sleep 60; cp '{f}/agreement.json' \"$out\";;\n apply) mkdir -p \"$out/ops/federation/acl\"; cp '{f}/fragment.json5' \"$out/ops/federation/acl/xab-fragment.json5\";;\nesac\n",
            h6 = hang_6.display()
        ),
    );
    exe(
        &repo.join("ops/federation/standup/preflight.py"),
        &format!(
            "#!/usr/bin/env python3\nimport json, os, sys\nopen('{l}','a').write('preflight\\n')\na = sys.argv\nif '--result' in a:\n    only = a[a.index('--only') + 1].split(',') if '--only' in a else ['inputs', 'edge-ports', 'core-ports', 'slot-anchors', 'core-router', 'ingress-stores', 'ca-key']\n    open(a[a.index('--result') + 1], 'w').write(json.dumps({{'ready': True, 'results': [{{'check': c, 'outcome': 'PASS', 'detail': 'stub'}} for c in only]}}))\n"
        ),
    );
    exe(
        &repo.join("ops/federation/standup/mint-peering-leaves.sh"),
        &format!("#!/bin/sh\necho mint >> '{l}'\nmkdir -p \"$2\"; printf -- '-----BEGIN CERTIFICATE-----\\nQUFB\\n-----END CERTIFICATE-----\\n' > \"$2/core.pem\"; printf -- '-----BEGIN PRIVATE KEY-----\\nQkJC\\n-----END PRIVATE KEY-----\\n' > \"$2/core-key.pem\"\n"),
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
    // This peering's values on its hosts, as the planner and the plays read them.
    write(
        &repo.join("ops/ansible/group_vars/hello-ally_edge/peering.yml"),
        &format!("peering_wg_interface: wg-fed1\npeering_wg_listen_port: 51821\ngoat_zenoh_edge_listen_port: 7448\ngoat_zenoh_edge_partner_ca_path: {}\n", r.join("edge/partner-ca.pem").display()),
    );
    write(
        &repo.join("ops/ansible/group_vars/hello-ally_core/peering.yml"),
        "goat_fed_republisher_metrics_port: 9464\n",
    );
    // The peering inventory the planner reads (documentation addresses).
    write(
        &repo.join("ops/ansible/inventory/peering-edges.ini"),
        "[hello-ally_edge]\nedge-1 ansible_host=192.0.2.10\n\n[hello-ally_core]\ncore-1 ansible_host=192.0.2.11\n",
    );
    let work = r.join("work").join("hello-ally");
    std::fs::create_dir_all(&work).unwrap();
    // Their CA certificate (any DER-shaped bytes) and their half naming its
    // fingerprint, as phase 5 checks them.
    let der = b"fake-ca-der";
    let their_fp = sha256_hex(der);
    std::fs::write(
        work.join("theirs.canonical.json"),
        resigned("xab", &their_fp),
    )
    .unwrap();
    write(
        &work.join("theirs-ca.pem"),
        &format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            b64(der)
        ),
    );
    write(&r.join("signing.key"), &seed_hex("xaa"));
    let profile = r.join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    World {
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
        args.extend(["-C".into(), self.repo.display().to_string(), "--yes".into()]);
        if !global.contains(&"-o") {
            args.extend(["-o".into(), "json".into()]);
        }
        args.push("federate".into());
        args.extend(extra.iter().map(|s| s.to_string()));
        let mut c = Command::new(bin());
        c.args(&args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", r)
            .env("GOAT_BIN", r.join("bin/goat"))
            // The world stands in for `goat` with a stub; since the verbs run in
            // goatbuilder by default, name the stub path so the run uses it.
            .env("FED_VERBS", "goat")
            .env("GOAT_PROFILE_DIR", &self.profile)
            .env("GOAT_NO_FUN", "1")
            .env("FED_WORK", &self.work)
            .env("FED_NO_DIRECTORY", "1")
            .env("FED_COUNTERPARTY_ID", "xab")
            .env("FED_SIGNING_KEY", r.join("signing.key"))
            .env("FED_CORE_ACL_ORG", r.join("core/org.json5"))
            .env("FED_CORE_ACL_SEED", r.join("core/seed.json5"))
            .env("FED_CORE_ACL_OUT", r.join("core/config.json5"))
            .env(
                "FED_CORE_DRY_RUN",
                format!("echo dry-run >> '{}'", self.log.display()),
            )
            .env(
                "FED_CORE_RESTART",
                format!("echo restart >> '{}'", self.log.display()),
            )
            .env("FED_SLOT_CHECK", "skip")
            .current_dir(&self.repo)
            .stdin(std::process::Stdio::null());
        c
    }

    pub fn run(&self, extra: &[&str]) -> (Output, serde_json::Value) {
        self.run_with(&[], extra)
    }

    pub fn run_with(&self, global: &[&str], extra: &[&str]) -> (Output, serde_json::Value) {
        let out = self.command_with(global, extra).output().unwrap();
        let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
        (out, v)
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
