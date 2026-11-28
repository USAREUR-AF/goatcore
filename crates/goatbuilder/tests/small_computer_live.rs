//! `doctor --target <type> --live` on a machine of the small-computer family,
//! through the built binary. `with-aws`, which starts every ansible tool, is a
//! stand-in that runs the real collector (ops/scripts/small-computer-endstate.py)
//! against the stand-in machine of tests/fixtures/small-computer/machine.sh: a
//! real LUKS2 header made by the real cryptsetup, stand-in tools for the rest.
//! Where cryptsetup is not installed the test says so and does not run.

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Goatnet `zagros` with two break-glass machines: pi-1 a small computer, bg-2
/// declaring no family; and a network-jump machine on a host-class box.
fn checkout() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    write(r, "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_breakglass_hosts]\npi-1\npi-2\nbg-3\n[zagros_network_jump_hosts]\njump-1\n");
    write(r, "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(r, "ops/ansible/host_vars/pi-1.yml", "platform_family: small-computer\n");
    write(r, "ops/ansible/host_vars/pi-2.yml", "platform_family: small-computer\n");
    write(r, "ops/ansible/host_vars/jump-1.yml", "platform_family: host-class\n");
    write(r, "ops/pins/fleet-pins.yaml", "goatnets:\n  zagros:\n    members: [zagros_hosts, zagros_breakglass_hosts, zagros_network_jump_hosts]\ncomponents:\n  svc:\n    var: svc_image\n    fleet: \"a@sha256:1\"\n");
    for (f, d) in [("inventory-hosts.py", "ops/scripts"), ("small-computer-endstate.py", "ops/scripts"), ("project-setting.sh", "ops/scripts")] {
        test_support::copied_executable(&real.join(d).join(f), &r.join(d).join(f));
    }
    // ansible's answer to `<node> -b -m script -a "executable=/usr/bin/python3 <script>"`;
    // pi-2 does not answer.
    test_support::executable(
        &r.join("ops/scripts/with-aws"),
        r#"#!/bin/sh
shift; [ "$1" = "--" ] && shift
echo "$*" >> "$GB_CALLS"
[ "$1" = ansible ] || exit 0
shift
node=""; args=""
while [ $# -gt 0 ]; do case "$1" in -i) shift 2 ;; -m) shift 2 ;; -b) shift ;; -a) args="$2"; shift 2 ;; *) node="$1"; shift ;; esac; done
[ "$node" = pi-2 ] && { printf '%s | UNREACHABLE! => {\n    "msg": "no route to host"\n}\n' "$node"; exit 4; }
out=$(SMALL_COMPUTER_ROOT="$GB_MACHINE/root" SMALL_COMPUTER_PATH="$GB_MACHINE/bin:/usr/sbin:/usr/bin:/sbin:/bin" python3 ${args#executable=/usr/bin/python3 }) || exit 2
printf '%s | CHANGED => ' "$node"
printf '%s' "$out" | python3 -c 'import json,sys; print(json.dumps({"rc": 0, "stdout": sys.stdin.read()}, indent=4))'
"#,
    );
    t
}

fn run(root: &Path, args: &[&str], machine: &Path) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", args[0], "-C", root.to_str().unwrap()])
        .args(&args[1..])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", root.join("home"))
        .env("GB_CALLS", root.join("calls"))
        .env("GB_MACHINE", machine)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .unwrap();
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

type Row = (String, bool, String);

fn rows(out: &str) -> Vec<Row> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["ok"] == true, c["detail"].as_str().unwrap().to_string())).collect()
}

fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter().find(|r| r.0 == name).unwrap_or_else(|| panic!("no row {name}: {rows:#?}"))
}

fn machine() -> Option<tempfile::TempDir> {
    if !Path::new("/usr/sbin/cryptsetup").exists() {
        // The job that installs it (goatbuilder-pr.yml) sets GOAT_TEST_REQUIRE_CRYPTSETUP:
        // there a missing one is a failure, never a pass. Elsewhere the test says it did not run.
        assert!(std::env::var_os("GOAT_TEST_REQUIRE_CRYPTSETUP").is_none(), "GOAT_TEST_REQUIRE_CRYPTSETUP is set and there is no /usr/sbin/cryptsetup: the LUKS2 header these tests read cannot be made");
        eprintln!("SKIP (not run, not a pass): no /usr/sbin/cryptsetup here, so no LUKS2 header can be made");
        return None;
    }
    let m = tempfile::tempdir().unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/small-computer/machine.sh");
    let o = Command::new("bash").arg(&script).arg(m.path()).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    Some(m)
}

#[test]
fn a_small_computer_is_read_as_root_and_held_to_its_model() {
    let Some(m) = machine() else { return };
    let t = checkout();
    let (_, out) = run(t.path(), &["doctor", "--target", "breakglass", "--site", "zagros", "--live"], m.path());
    let rows = rows(&out);
    for name in ["os", "root", "primitives", "unlock", "fips", "ssh"] {
        let r = row(&rows, &format!("small-computer pi-1 {name}"));
        assert!(r.1, "{r:?}");
    }
    assert_eq!(row(&rows, "small-computer pi-1 primitives").2, "aes-xts-plain64 with a 512-bit key; 2 keyslots, every one pbkdf2-sha256");
    assert!(row(&rows, "small-computer pi-1 unlock").2.starts_with("yubikey-hmac; 2 keyslots"));
    // Two facts the stand-in cannot fake: this machine's own architecture, and
    // that the collector ran here without root.
    let arch = std::env::consts::ARCH;
    if arch != "aarch64" {
        assert!(!row(&rows, "small-computer pi-1 board").1 && row(&rows, "small-computer pi-1 board").2.contains("a rpi5 is aarch64"), "{rows:#?}");
    }
    let fb = row(&rows, "small-computer pi-1 first boot");
    assert!(!fb.1 && fb.2.contains("did not run as root"), "{fb:?}");
    // A machine that did not answer is unknown, never a pass.
    let down = row(&rows, "small-computer pi-2 machine");
    assert!(!down.1 && down.2.contains("unknown: pi-2 was not read") && down.2.contains("UNREACHABLE"), "{down:?}");
    // bg-3 declares no family, so nothing reads it.
    assert!(!rows.iter().any(|r| r.0.starts_with("small-computer bg-3")), "{rows:#?}");
    let calls = std::fs::read_to_string(t.path().join("calls")).unwrap();
    assert!(calls.lines().any(|l| l.contains("ansible") && l.contains("pi-1 -b -m script") && l.contains("small-computer-endstate.py")), "{calls}");
    assert!(!calls.contains("bg-3"), "{calls}");
}

/// A type that requires an encrypted root, from the overlay: the checkout's
/// UNKNOWN encryption row is made until every machine's root was read.
const EDGE: &str = "schema: goatbuilder-target/v1\ntype: edge-box\nfamilies: [small-computer]\nencryption: root\nphases:\n  - label: nothing\n    skip: a test type\n";

#[test]
fn a_root_read_on_every_machine_makes_no_unknown_encryption_row_and_a_broken_rule_fails_its_row() {
    let Some(m) = machine() else { return };
    let t = checkout();
    write(t.path(), "ops/goatbuilder/overlay/targets/edge-box.yaml", EDGE);
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_edge_box_hosts]\npi-1\npi-2\n");
    let doctor = ["doctor", "--target", "edge-box", "--site", "zagros", "--live"];
    let (_, out) = run(t.path(), &doctor, m.path());
    assert!(row(&rows(&out), "encryption").2.starts_with("UNKNOWN"), "pi-2 was not read: {out}");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_edge_box_hosts]\npi-1\n");
    let (_, out) = run(t.path(), &doctor, m.path());
    assert!(!rows(&out).iter().any(|r| r.0 == "encryption"), "{out}");
    let (_, out) = run(t.path(), &["doctor", "--target", "edge-box", "--site", "zagros"], m.path());
    assert!(row(&rows(&out), "encryption").2.starts_with("UNKNOWN"), "without --live: {out}");
    // A Tang binding added beside the YubiKey, in the real header: a way in the model does not deliver.
    write(m.path(), "token.json", "{\"type\": \"clevis\", \"keyslots\": [\"1\"], \"jwe\": {\"ciphertext\": \"x\"}}");
    let o = Command::new("/usr/sbin/cryptsetup").args(["token", "import", "--json-file"]).arg(m.path().join("token.json")).arg(m.path().join("luks.img")).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let (_, out) = run(t.path(), &doctor, m.path());
    let unlock = row(&rows(&out), "small-computer pi-1 unlock").clone();
    assert!(!unlock.1 && unlock.2.contains("unlocks by token clevis, which the rpi5 does not deliver"), "{unlock:?}");
    // A plain root, as a Pi that skipped the encryption step has.
    write(m.path(), "bin/lsblk", "#!/bin/sh\necho '{\"blockdevices\": [{\"name\": \"/dev/nvme0n1p2\", \"type\": \"part\", \"fstype\": \"ext4\", \"children\": [{\"name\": \"/dev/nvme0n1\", \"type\": \"disk\", \"fstype\": null}]}]}'\n");
    let (_, out) = run(t.path(), &doctor, m.path());
    let rows = rows(&out);
    let root = row(&rows, "small-computer pi-1 root");
    assert!(!root.1 && root.2.contains("is not on an encrypted device"), "{root:?}");
}

#[test]
fn without_live_nothing_is_read_and_a_host_class_machine_gets_no_small_computer_rows() {
    let t = checkout();
    let dir = tempfile::tempdir().unwrap();
    let (_, out) = run(t.path(), &["doctor", "--target", "breakglass", "--site", "zagros"], dir.path());
    assert!(!rows(&out).iter().any(|r| r.0.starts_with("small-computer")), "{out}");
    assert!(!t.path().join("calls").exists() || !std::fs::read_to_string(t.path().join("calls")).unwrap().contains("small-computer-endstate"));
    let (_, out) = run(t.path(), &["doctor", "--target", "network-jump", "--site", "zagros", "--live"], dir.path());
    let rows = rows(&out);
    assert!(!rows.iter().any(|r| r.0.starts_with("small-computer")), "{rows:#?}");
    assert!(row(&rows, "live").2.contains("no machine of it declares the small-computer family"), "{rows:#?}");
}
