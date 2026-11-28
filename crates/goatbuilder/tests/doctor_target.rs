//! `goatbuilder doctor --target <type> --site <site>` (ADR 1257; gate row J1):
//! the base check of a site's machines of one target type, through the built
//! binary. It reads the checkout only: no run here reaches a host.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Goatnet `zagros`, one founder site with two break-glass machines, in a
/// checkout with no type files of its own.
fn checkout() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_breakglass_hosts]\nbg-1\nbg-2\n");
    let scripts = t.path().join("ops/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/inventory-hosts.py"), scripts.join("inventory-hosts.py")).unwrap();
    t
}

/// The exit code and the -o json report (or the error) of one run.
fn doctor(root: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "doctor", "-C", root.to_str().unwrap()])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("run goatbuilder");
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn rows(out: &str) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().clone()
}

fn row<'a>(rows: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    rows.iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("no row {name}: {rows:?}"))
}

#[test]
fn each_machine_of_the_type_is_held_to_its_families() {
    let t = checkout();
    let (rc, out) = doctor(t.path(), &["--target", "breakglass", "--site", "zagros"]);
    let r = rows(&out);
    // The break-glass play is goatcore's own; this checkout does not have it.
    assert_eq!(rc, 1, "a failed row exits 1: {out}");
    assert!(row(&r, "target")["detail"].as_str().unwrap().contains("[zagros_breakglass_hosts], families small-computer"), "{out}");
    for m in ["bg-1", "bg-2"] {
        let f = row(&r, &format!("family {m}"));
        assert!(f["ok"] == true && f["skipped"] == true && f["detail"].as_str().unwrap().contains("declares no platform_family"), "{f}");
    }
    let plays = row(&r, "plays");
    assert!(plays["ok"] == false && plays["detail"].as_str().unwrap().contains("ops/ansible/breakglass-nodes.yml"), "{plays}");
    assert_eq!(row(&r, "end state")["skipped"], true);

    write(t.path(), "ops/ansible/breakglass-nodes.yml", "---\n");
    write(t.path(), "ops/ansible/host_vars/bg-1.yml", "platform_family: small-computer\n");
    write(t.path(), "ops/ansible/host_vars/bg-2.yml", "platform_family: cloud\n");
    let (rc, out) = doctor(t.path(), &["--target", "breakglass", "--site", "zagros"]);
    let r = rows(&out);
    assert_eq!(rc, 1, "a failed row exits 1: {out}");
    let ok = row(&r, "family bg-1");
    assert!(ok["ok"] == true && ok["detail"].as_str().unwrap().contains("bg-1 is small-computer (ops/ansible/host_vars/bg-1.yml)"), "{ok}");
    let bad = row(&r, "family bg-2");
    assert!(bad["ok"] == false && bad["detail"].as_str().unwrap().contains("accepts only: small-computer, host-class"), "{bad}");
    // A host-class board is a break-glass machine as a small computer is.
    write(t.path(), "ops/ansible/host_vars/bg-2.yml", "platform_family: host-class\n");
    let (_, out) = doctor(t.path(), &["--target", "breakglass", "--site", "zagros"]);
    let hr = rows(&out);
    let hc = row(&hr, "family bg-2");
    assert!(hc["ok"] == true && hc["detail"].as_str().unwrap().contains("bg-2 is host-class (ops/ansible/host_vars/bg-2.yml), which breakglass accepts"), "{hc}");
    write(t.path(), "ops/ansible/host_vars/bg-2.yml", "platform_family: cloud\n");
    assert_eq!(row(&r, "plays")["ok"], true);

    // One machine alone.
    let (rc, out) = doctor(t.path(), &["--target", "breakglass", "--site", "zagros", "--node", "bg-1"]);
    let r = rows(&out);
    // The type requires an encrypted root, which the checkout cannot show: UNKNOWN, exit 1.
    assert_eq!(rc, 1, "{out}");
    assert!(row(&r, "encryption")["detail"].as_str().unwrap().starts_with("UNKNOWN"), "{out}");
    assert!(r.iter().all(|x| x["name"] != "family bg-2"), "{out}");
}

#[test]
fn an_unknown_type_is_refused_and_target_needs_a_site() {
    let t = checkout();
    let (rc, out) = doctor(t.path(), &["--target", "toaster", "--site", "zagros"]);
    assert!(rc != 0 && out.contains("GB-247") && out.contains("known: breakglass, core, federation-boundary, first-bite, goat-processors, network-jump, network-tap, obs, octo-pod, phone, simple-boundary"), "{out}");
    let (rc, out) = doctor(t.path(), &["--target", "core"]);
    assert!(rc == 2 && out.contains("--site"), "{rc} {out}");
}
