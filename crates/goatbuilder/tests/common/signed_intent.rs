//! A signed intent for a stand-in checkout: deploy verifies the goatnet's intent
//! before any host (GB-383), so a world whose deploy is to run needs one. The
//! binary renders pins.yml and the intent; a CA key made here, and dropped with
//! the world, signs it; its public key is committed as the checkout's.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub const CA: &str = "fixture-ca";

fn write(p: &Path, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

/// What rendering an intent reads besides the pins table: the intent schema, an
/// empty capabilities file, and each group's CA id. Every goatnet of the table
/// names the variable that holds it.
pub fn prepare(repo: &Path, members: &[&str]) {
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema = "ops/consistency/schema/intent-v1.schema.json";
    write(&repo.join(schema), &std::fs::read_to_string(real.join(schema)).unwrap());
    write(&repo.join("ops/pins/parity-capabilities.yaml"), "capabilities: []\nnode_shape: []\nretired: []\ndeviations: {}\n");
    for m in members {
        let d = repo.join("ops/ansible/group_vars").join(m);
        write(&d.join("ca.yml"), &format!("fixture_ca_id: \"{CA}\"\n"));
    }
    let table = repo.join("ops/pins/fleet-pins.yaml");
    let t = std::fs::read_to_string(&table).unwrap();
    if !t.contains("ca_id_var") {
        let t: String = t.lines().map(|l| if l.trim_start().starts_with("members:") { format!("{l}\n    ca_id_var: fixture_ca_id\n") } else { format!("{l}\n") }).collect();
        std::fs::write(&table, t).unwrap();
    }
}

/// Render every pins.yml and intent of the checkout, and sign each intent.
pub fn sign(repo: &Path) {
    let key_dir = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["pins", "-C"])
        .arg(repo)
        .env("HOME", key_dir.path())
        .env("GOAT_NO_FUN", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "pins: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let key = key_dir.path().join("ca.key");
    let openssl = |args: &[&std::ffi::OsStr]| {
        let o = Command::new("openssl").args(args).output().unwrap();
        assert!(o.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };
    let s = |x: &str| std::ffi::OsString::from(x);
    let pem = repo.join("ops/enrollment/public-keys").join(format!("{CA}.pem"));
    std::fs::create_dir_all(pem.parent().unwrap()).unwrap();
    openssl(&[&s("ecparam"), &s("-name"), &s("prime256v1"), &s("-genkey"), &s("-noout"), &s("-out"), key.as_os_str()]);
    openssl(&[&s("ec"), &s("-in"), key.as_os_str(), &s("-pubout"), &s("-out"), pem.as_os_str()]);
    for e in std::fs::read_dir(repo.join("ops/consistency/intent")).unwrap().flatten() {
        let (intent, der) = (e.path().join("intent.json"), key_dir.path().join("sig.der"));
        openssl(&[&s("dgst"), &s("-sha256"), &s("-sign"), key.as_os_str(), &s("-out"), der.as_os_str(), intent.as_os_str()]);
        use base64::Engine;
        let sig = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&der).unwrap());
        std::fs::write(e.path().join("intent.json.sig"), sig + "\n").unwrap();
    }
}
