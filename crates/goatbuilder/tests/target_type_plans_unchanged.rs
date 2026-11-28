//! A type file (`ops/goatbuilder/targets/<type>.yaml`, ADR 1257) says what a
//! machine of that type converges as. This test holds every type's plans to
//! the ones recorded before the files existed, when the same steps were Rust:
//! `join-node --dry-run -o json` for every type, at a founder site and at
//! joining sites of each shape, over the mesh, through the env door and over
//! USB; and `standup --dry-run -o json` for a site that declares an obs host
//! and both rescue nodes, and for one that declares none.
//!
//! The same runs with a private overlay beside them (a type and a family of its
//! own, and the overlay's play) give the same bytes: an overlay adds, and no
//! public output changes (gate J26).
//!
//! A change that is meant to alter a plan regenerates the record and says why
//! in the same change:
//! `GOATBUILDER_GOLDEN=overwrite cargo test -p goatbuilder --test target_type_plans_unchanged`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/target-type-plans.golden.json")
}

fn write(path: PathBuf, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

/// Goatnet `zagros`: a founder with an obs host and both rescue nodes; a joiner
/// that federates signal and has no obs host; a joiner that declares federation
/// off, with an obs host and the public front door; a founder that declares no
/// optional machine.
fn zagros() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let d = t.path();
    write(
        d.join("ops/goatbuilder/goatbuilder.yaml"),
        "schema: goatbuilder/v1\n\
         repo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\n\
         components:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\n\
         environments:\n  - name: zagros\n    converger: push-ansible\n    sites:\n\
         \x20     - {name: zagros, role: founder}\n\
         \x20     - {name: zagros-b, role: joiner, joins: zagros}\n\
         \x20     - {name: zagros-c, role: joiner, joins: zagros}\n\
         \x20     - {name: zagros-d, role: founder}\n\
         promotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\n\
         deploy: {ssh_user: operator}\n",
    );
    let gv = |site: &str, text: &str| write(d.join(format!("ops/ansible/group_vars/{site}_hosts/vars.yml")), text);
    let zone = "site_enrollment_dns: netbird.example.net\nsite_dns_zone: zagros.example.org\n";
    gv("zagros", &format!("goatnet_founder: true\nintra_site_pair: true\nhas_obs_host: true\nhas_breakglass_node: true\nhas_network_jump_node: true\n{zone}"));
    gv("zagros_b", &format!("goatnet_founder: false\njoin_founder_site: zagros\nobs_colocated_site_zones:\n  zagros-b: zagros.example.org\nhas_breakglass_node: true\n{zone}"));
    gv("zagros_c", &format!("goatnet_founder: false\njoin_founder_site: zagros\nhas_obs_host: true\nsignal_federation_absent_reason: one relay\ndashboard_cert_delivery_mode: acme\n{zone}"));
    gv("zagros_d", &format!("goatnet_founder: true\nbreakglass_absent_reason: no second bearer\n{zone}"));
    write(d.join("ops/ansible/group_vars/zagros_obs_hosts/vars.yml"), "site_id: zagros\n");
    // network-jump's input, which join-node refuses a machine without (GB-414).
    for site in ["zagros", "zagros_b", "zagros_c"] {
        write(d.join(format!("ops/ansible/group_vars/{site}_network_jump_hosts/vars.yml")), "herd_connector_image: \"ghcr.io/example-org/herd-connector:1.0@sha256:0000000000000000000000000000000000000000000000000000000000000001\"\n");
    }
    for site in ["zagros-b", "zagros-c"] {
        write(
            d.join(format!("ops/enrollment/manifests/{site}-certs.yaml")),
            &format!("site_id: {site}\nout_dir: /s/{site}-leaves\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health\n  - name: portal\n    cn: portal\n"),
        );
    }
    write(d.join("ops/dns/zagros/mesh-hosts"), "# zagros mesh DNS\n100.127.0.1 isitup.zagros.zagros.example.org\n");
    write(
        d.join("ops/ansible/inventory/mgmt-hosts.ini"),
        "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_obs_hosts]\nmgmt-zagros-01-obs\n[zagros_breakglass_hosts]\nzagros-bg-1\n[zagros_network_jump_hosts]\nzagros-nj-1\n",
    );
    write(d.join("ops/ansible/inventory/mgmt-hosts-mesh.ini"), "[zagros_hosts]\nmgmt-zagros-01-a ansible_host=100.127.0.1\n");
    // One rescue class keeps an inventory of its own, the other does not.
    write(d.join("ops/ansible/inventory/zagros-breakglass-nodes.ini"), "[zagros_breakglass_hosts]\nzagros-bg-1\n");
    // goatcore ships no utility play; an assembled tree does.
    write(d.join("ops/ansible/utility-host-bootstrap.yml"), "---\n");
    std::fs::create_dir_all(d.join("ops/scripts")).unwrap();
    std::fs::copy(repo().join("ops/scripts/inventory-hosts.py"), d.join("ops/scripts/inventory-hosts.py")).unwrap();
    t
}

/// The third-site fixture with the end-state model beside it, so a joiner's
/// core plan holds the health publishers the site declares.
fn third_site() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    copy_tree(&repo().join("ops/goatbuilder/fixtures/join-node-third-site"), t.path());
    copy_tree(&repo().join("ops/goatbuilder/endstate"), &t.path().join("ops/goatbuilder/endstate"));
    for p in ["fabric", "mesh", "presence", "time", "consistency"] {
        write(t.path().join(format!("ops/ansible/{p}-health-deploy.yml")), "---\n");
    }
    write(t.path().join("ops/ansible/fabric-crawler-deploy.yml"), "---\n");
    t
}

/// One dry run's rows, with the throwaway paths taken out. A refused plan is
/// recorded as its error text.
fn run(dir: &Path, args: &[&str]) -> serde_json::Value {
    let dir = dir.canonicalize().unwrap();
    let home = dir.join("home");
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", args[0], "-C", dir.to_str().unwrap()])
        .args(&args[1..])
        .arg("--dry-run")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .expect("run goatbuilder");
    let clean = |b: &[u8]| String::from_utf8_lossy(b).replace(home.to_str().unwrap(), "<home>").replace(dir.to_str().unwrap(), "<repo>");
    let out = clean(&o.stdout);
    if !o.status.success() {
        let text = format!("{out}{}", clean(&o.stderr));
        let doc: serde_json::Value = serde_json::from_str(&out).or_else(|_| serde_json::from_str(&clean(&o.stderr))).unwrap_or_else(|_| panic!("{args:?}: {text}"));
        return serde_json::json!({"exit": o.status.code(), "code": doc["code"], "refused": doc["error"]});
    }
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    v["checks"].clone()
}

/// A private overlay as an assembled tree carries it: a type and a family of
/// its own in ops/goatbuilder/overlay/, and the overlay's play in ops/ansible.
fn with_overlay(d: &Path) {
    write(d.join("ops/goatbuilder/overlay/families/example-board.yaml"), "schema: goatbuilder-family/v1\nfamily: example-board\ntitle: A synthetic board\nmodels: [example-board-1]\n");
    write(
        d.join("ops/goatbuilder/overlay/targets/example-role.yaml"),
        "schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [example-board, host-class]\nphases:\n  - label: the example role at {site}\n    plays:\n      - {play: example-role.yml, args: \"-e target={group}\", from: overlay}\n",
    );
    write(d.join("ops/ansible/example-role.yml"), "- name: example role\n  hosts: \"{{ target }}\"\n  tasks: []\n");
}

fn plans() -> serde_json::Map<String, serde_json::Value> {
    plans_with(&|_| {})
}

fn plans_with(add: &dyn Fn(&Path)) -> serde_json::Map<String, serde_json::Value> {
    let mut now = serde_json::Map::new();
    let z = zagros();
    add(z.path());
    let mut join = |dir: &Path, env: &str, site: &str, class: &str, via: &str| {
        let key = format!("join-node {env}/{site} {class} {via}");
        let mut args = vec!["join-node", env, "--site", site, "--node", "node-x", "--class", class, "--via", via];
        if via != "usb" {
            args.extend(["--address", "192.0.2.9"]);
        }
        now.insert(key, run(dir, &args));
    };
    for site in ["zagros", "zagros-b", "zagros-c"] {
        for class in ["core", "obs", "breakglass", "network-jump"] {
            join(z.path(), "zagros", site, class, "mesh");
        }
    }
    for class in ["core", "obs", "breakglass", "network-jump"] {
        join(z.path(), "zagros", "zagros", class, "door");
    }
    join(z.path(), "zagros", "zagros-b", "phone", "usb");
    join(z.path(), "zagros", "zagros", "phone", "usb");
    // A type with no plan from before its file: its plans are recorded from the file.
    for site in ["zagros", "zagros-b"] {
        join(z.path(), "zagros", site, "simple-boundary", "mesh");
    }
    let t = third_site();
    add(t.path());
    for (site, class, via) in [("gamma", "core", "mesh"), ("delta", "phone", "usb")] {
        join(t.path(), "zagros", site, class, via);
    }
    // goatcore's own tree has no utility play: the founder's obs node is refused.
    std::fs::remove_file(z.path().join("ops/ansible/utility-host-bootstrap.yml")).unwrap();
    now.insert("join-node zagros/zagros obs mesh, no utility play".into(), run(z.path(), &["join-node", "zagros", "--site", "zagros", "--node", "node-x", "--class", "obs"]));
    write(z.path().join("ops/ansible/utility-host-bootstrap.yml"), "---\n");
    for (site, profile, extra) in [("zagros", "lan", None), ("zagros", "internet", None), ("zagros", "lan", Some("--obs-only")), ("zagros", "lan", Some("--skip-obs")), ("zagros-b", "lan", None), ("zagros-d", "lan", None)] {
        let mut args = vec!["standup", "zagros", "--site", site, "--profile", profile];
        args.extend(extra);
        now.insert(format!("standup {site} {profile} {}", extra.unwrap_or("")).trim().to_string(), run(z.path(), &args));
    }
    now
}

#[test]
fn every_type_keeps_the_plans_it_had_before_its_file() {
    let now = plans();
    let refused: Vec<&String> = now.iter().filter(|(_, v)| v.get("refused").is_some()).map(|(k, _)| k).collect();
    // A phone node joins a goatnet (GB-301), and a founder's obs node needs
    // the utility play (GB-248); every other run is a plan.
    assert_eq!(refused, ["join-node zagros/zagros phone usb", "join-node zagros/zagros obs mesh, no utility play"], "{:?}", refused.iter().map(|k| &now[*k]).collect::<Vec<_>>());
    if std::env::var("GOATBUILDER_GOLDEN").as_deref() == Ok("overwrite") {
        std::fs::write(golden_path(), serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let was: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(golden_path()).expect("the recorded plans")).unwrap();
    assert_eq!(was.keys().collect::<Vec<_>>(), now.keys().collect::<Vec<_>>(), "the same runs are recorded");
    for (key, plan) in &now {
        let before = &was[key];
        if before != plan {
            let empty = Vec::new();
            let (b, a) = (before.as_array().unwrap_or(&empty), plan.as_array().unwrap_or(&empty));
            let first = b.iter().zip(a).position(|(x, y)| x != y).unwrap_or(b.len().min(a.len()));
            panic!("{key}: the plan changed at row {first}\nrecorded: {}\nnow:      {}", b.get(first).unwrap_or(before), a.get(first).unwrap_or(plan));
        }
    }
}

/// The recorded plans, byte for byte, with a private overlay beside them: an
/// overlay adds a type and a family, and changes no public type's plan.
#[test]
fn an_overlay_changes_no_public_plan_by_a_byte() {
    let recorded = std::fs::read_to_string(golden_path()).expect("the recorded plans");
    let now = serde_json::to_string_pretty(&plans_with(&with_overlay)).unwrap() + "\n";
    if now != recorded {
        let at = now.bytes().zip(recorded.bytes()).position(|(a, b)| a != b).unwrap_or(now.len().min(recorded.len()));
        let line = recorded[..at.min(recorded.len())].lines().count();
        panic!("with an overlay, the plans differ from the record at line {line}:\nrecorded: {}\nnow:      {}", recorded.lines().nth(line.saturating_sub(1)).unwrap_or(""), now.lines().nth(line.saturating_sub(1)).unwrap_or(""));
    }
    // And the overlay's own type is there to be built.
    let z = zagros();
    with_overlay(z.path());
    let rows = run(z.path(), &["join-node", "zagros", "--site", "zagros", "--node", "node-x", "--class", "example-role", "--address", "192.0.2.9"]);
    assert!(rows.to_string().contains("example-role.yml -e target=zagros_example_role_hosts -l node-x"), "{rows}");
}
