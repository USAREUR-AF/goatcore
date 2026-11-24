//! `doctor --target network-tap --live` and `status` on a network tap, through
//! the built binary. `with-aws`, which starts every ansible tool, is a stand-in
//! that runs the real collector (ops/scripts/tap-endstate.py) as the machine
//! would: inside a network namespace of its own, with real interfaces, bridges
//! and egress drops, where the machine allows one. Elsewhere that part is
//! skipped, and says so.

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Goatnet `zagros` with two taps: tap-1 inline, tap-2 listening.
fn checkout() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    write(r, "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_network_tap_hosts]\ntap-1\ntap-2\n");
    write(r, "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(r, "ops/ansible/host_vars/tap-1.yml", "tap:\n  mode: inline\n  ports: [tap0, tap1]\n  management: mgmt0\n");
    write(r, "ops/ansible/host_vars/tap-2.yml", "tap:\n  mode: listen\n  source: copper-tap\n  ports: [tap0, tap1]\n  management: mgmt0\n");
    write(r, "ops/pins/fleet-pins.yaml", "goatnets:\n  zagros:\n    members: [zagros_hosts, zagros_network_tap_hosts]\ncomponents:\n  svc:\n    var: svc_image\n    fleet: \"a@sha256:1\"\n");
    for (f, d) in [("inventory-hosts.py", "ops/scripts"), ("tap-endstate.py", "ops/scripts"), ("project-setting.sh", "ops/scripts")] {
        test_support::copied_executable(&real.join(d).join(f), &r.join(d).join(f));
    }
    // ansible's answer to `-m script -a "executable=... <script> <args>"`.
    test_support::executable(
        &r.join("ops/scripts/with-aws"),
        r#"#!/bin/sh
shift; [ "$1" = "--" ] && shift
echo "$*" >> "$GB_CALLS"
[ "$1" = ansible ] || exit 0
shift
node=""; args=""
while [ $# -gt 0 ]; do case "$1" in -i) shift 2 ;; -m) shift 2 ;; -a) args="$2"; shift 2 ;; *) node="$1"; shift ;; esac; done
[ "$node" = "$GB_DOWN" ] && { printf '%s | UNREACHABLE! => {\n    "msg": "no route to host"\n}\n' "$node"; exit 4; }
out=$(sh "$GB_NS" "$node" "python3 ${args#executable=/usr/bin/python3 }") || exit 2
printf '%s | CHANGED => ' "$node"
printf '%s' "$out" | python3 -c 'import json,sys; print(json.dumps({"rc": 0, "stdout": sys.stdin.read()}, indent=4))'
"#,
    );
    t
}

fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    c.args(["-o", "json", args[0], "-C", root.to_str().unwrap()])
        .args(&args[1..])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", root.join("home"))
        .env("GB_CALLS", root.join("calls"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn rows(out: &str) -> Vec<(String, bool, String)> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["ok"] == true, c["detail"].as_str().unwrap().to_string())).collect()
}

fn row<'a>(rows: &'a [(String, bool, String)], name: &str) -> &'a (String, bool, String) {
    rows.iter().find(|r| r.0 == name).unwrap_or_else(|| panic!("no row {name}: {rows:#?}"))
}

const DOCTOR: [&str; 6] = ["doctor", "--target", "network-tap", "--site", "zagros", "--live"];

/// The namespace a tap is stood up in: user and mount namespaces, or sudo on a
/// hosted CI runner, which nobody else uses. `None` where neither is allowed.
fn namespace() -> Option<&'static str> {
    let ok = |c: &str| Command::new("sh").args(["-c", c]).env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").output().is_ok_and(|o| o.status.success());
    let probe = "ip link add aa type veth peer name bb && ip link add br9 type bridge";
    if ok(&format!("unshare -Urnm sh -c 'mount -t sysfs sysfs /sys && {probe}'")) {
        return Some("unshare -Urnm");
    }
    let hosted = std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted");
    if hosted && ok(&format!("sudo -n unshare -nm sh -c 'mount -t sysfs sysfs /sys && {probe}'")) {
        return Some("sudo -n unshare -nm");
    }
    None
}

/// A script that stands one node's links up in a namespace and runs a command
/// there. `good` is a tap as designed; otherwise a machine nobody prepared.
fn links(root: &Path, ns: &str, good: bool) -> String {
    let p = root.join(if good { "good.sh" } else { "raw.sh" });
    let prepare = if good {
        r#"
for i in tap0 tap1; do sysctl -qw net.ipv6.conf.$i.disable_ipv6=1; ethtool -K $i gro off >/dev/null 2>&1 || true; done
if [ "$1" = tap-1 ]; then
  ip link add br0 type bridge stp_state 0 forward_delay 0 group_fwd_mask 16384
  sysctl -qw net.ipv6.conf.br0.disable_ipv6=1
  for i in tap0 tap1; do ip link set $i master br0; done; ip link set br0 up
else
  tc qdisc add dev tap0 clsact; tc filter add dev tap0 egress matchall action drop
  nft add table netdev t; nft 'add chain netdev t e { type filter hook egress device "tap1" priority 0; policy drop; }'
fi
python3 - "$(dirname "$0")/rec" tap0 tap1 <<'PY' &
import socket, sys, time
s = [socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(3)) for _ in sys.argv[2:]]
for x, n in zip(s, sys.argv[2:]): x.bind((n, 3))
open(sys.argv[1], "w").close(); time.sleep(20)
PY
rec=$!; n=0; while [ ! -e "$(dirname "$0")/rec" ] && [ $n -lt 100 ]; do sleep 0.1; n=$((n+1)); done
"#
    } else {
        // Its network manager holds a packet socket on tap0, which records nothing.
        r#"python3 -c 'import socket, time
open("/proc/self/comm", "w").write("systemd-network")
s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(3)); s.bind(("tap0", 3)); time.sleep(20)' &
rec=$!; sleep 1; ip addr add 198.51.100.9/24 dev tap0; ip link add br0 type bridge; ip link set tap1 master br0; ip link set br0 up
"#
    };
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nif [ -z \"$GB_IN\" ]; then rm -f \"$(dirname \"$0\")/rec\"; exec {ns} env GB_IN=1 PATH=/usr/sbin:/usr/bin:/sbin:/bin sh \"$0\" \"$@\"; fi\nset -e\nmount -t sysfs sysfs /sys\nip link add tap0 type veth peer name far0; ip link add tap1 type veth peer name far1; ip link add mgmt0 type dummy\nip addr add 192.0.2.5/24 dev mgmt0; for i in lo mgmt0 tap0 tap1 far0 far1; do ip link set $i up; done; ip route add default via 192.0.2.1\n{prepare}\n$2\nif [ -n \"$rec\" ]; then kill $rec; fi\n"
        ),
    )
    .unwrap();
    p.display().to_string()
}

#[test]
fn live_reads_real_links_and_holds_them_to_the_declared_mode() {
    let Some(ns) = namespace() else {
        eprintln!("SKIP: this machine allows no network namespace, so the collector was not run on real links");
        return;
    };
    let t = checkout();
    let good = links(t.path(), ns, true);
    let (_, out) = run(t.path(), &DOCTOR, &[("GB_NS", &good)]);
    let r = rows(&out);
    for (node, mode, says) in [("tap-1", "inline", "one bridge of exactly tap0, tap1, STP off, forward delay zero, LLDP forwarded"), ("tap-2", "listen", "tap0, tap1 in no bridge, and every frame they would send is dropped")] {
        for (name, words) in [(mode, says), ("addresses", "no address on tap0, tap1"), ("management", "the default route is on mgmt0, and no route uses tap0, tap1"), ("recording", "a recorder is on the port, not on a bridge")] {
            let x = row(&r, &format!("tap {node} {name}"));
            assert!(x.1 && x.2.contains(words), "{x:?}");
        }
        // What this machine is not, whatever the links: each row says so or passes.
        row(&r, &format!("tap {node} watchdog"));
        row(&r, &format!("tap {node} root"));
    }
    let raw = links(t.path(), ns, false);
    let (rc, out) = run(t.path(), &DOCTOR, &[("GB_NS", &raw)]);
    let r = rows(&out);
    assert_eq!(rc, 1, "{out}");
    for (name, words) in [
        ("tap tap-1 addresses", "tap0 has 198.51.100.9/24"),
        ("tap tap-1 addresses", "IPv6 is not disabled on tap1"),
        ("tap tap-1 management", "a route uses tap0"),
        ("tap tap-1 inline", "br0 holds tap1, not exactly tap0, tap1"),
        ("tap tap-1 inline", "does not forward LLDP"),
        ("tap tap-1 recording", "nothing records tap0 or tap1"),
        ("tap tap-2 listen", "tap1 is in br0"),
        ("tap tap-2 listen", "tap0's egress is not dropped"),
    ] {
        let x = row(&r, name);
        assert!(!x.1 && x.2.contains(words), "{words}: {x:?}");
    }
}

#[test]
fn a_tap_that_is_not_read_is_unknown_and_without_live_no_machine_is_reached() {
    let t = checkout();
    let (_, out) = run(t.path(), &["doctor", "--target", "network-tap", "--site", "zagros"], &[]);
    assert!(!t.path().join("calls").exists(), "no --live, no machine: {out}");
    assert!(out.contains("requires an encrypted root"), "{out}");
    let (rc, out) = run(t.path(), &[&DOCTOR[..], &["--node", "tap-2"]].concat(), &[("GB_DOWN", "tap-2"), ("GB_NS", "/nonexistent")]);
    let r = rows(&out);
    let x = row(&r, "tap tap-2 machine");
    assert!(rc == 1 && !x.1 && x.2.starts_with("unknown: tap-2 was not read"), "{x:?}");
    let calls = std::fs::read_to_string(t.path().join("calls")).unwrap();
    assert!(calls.contains("tap-endstate.py --ports tap0,tap1 --management mgmt0") && !calls.contains("tap-1"), "{calls}");
    // A type that declares no tap has nothing read on a machine yet.
    let (_, out) = run(t.path(), &["doctor", "--target", "core", "--site", "zagros", "--live"], &[]);
    assert!(row(&rows(&out), "live").2.contains("core declares nothing this reads"), "{out}");
}

#[test]
fn status_prints_each_taps_mode() {
    let t = checkout();
    let (_, out) = run(t.path(), &["status", "--env", "zagros"], &[("GB_NS", "/nonexistent")]);
    let r = rows(&out);
    let a = row(&r, "zagros/tap-1/tap");
    assert!(a.1 && a.2.starts_with("inline, fail-closed: ") && a.2.ends_with("(ops/ansible/host_vars/tap-1.yml)"), "{a:?}");
    assert!(row(&r, "zagros/tap-2/tap").2.starts_with("listen from a copper-tap, fail-open: "), "{out}");
    write(t.path(), "ops/ansible/host_vars/tap-2.yml", "tap:\n  ports: [tap0]\n  management: mgmt0\n");
    let (rc, out) = run(t.path(), &["status", "--env", "zagros"], &[("GB_NS", "/nonexistent")]);
    let b = rows(&out);
    let b = row(&b, "zagros/tap-2/tap");
    assert!(rc == 3 && !b.1 && b.2.contains("it has no mode"), "{rc} {b:?}");
}
