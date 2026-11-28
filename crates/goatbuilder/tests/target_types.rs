//! Target types read from files (ADR 1257), through the built binary: a
//! private type needs no public change, a play that is not in the tree stops a
//! run and not a plan, and a machine of the wrong platform family is refused.
//! Nothing here reaches a host: every run is a dry run or refused before its
//! first phase.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Goatnet `zagros`, one founder site, in a checkout with no type files.
fn checkout() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "ops/goatbuilder/goatbuilder.yaml",
        "schema: goatbuilder/v1\n\
         repo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\n\
         components:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\n\
         environments:\n  - name: zagros\n    converger: push-ansible\n\
         promotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\n\
         deploy: {ssh_user: operator}\n",
    );
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_acme_hosts]\nacme-1\n");
    let scripts = t.path().join("ops/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/inventory-hosts.py"), scripts.join("inventory-hosts.py")).unwrap();
    t
}

/// A private overlay's type, whose play is in no public repository.
fn with_acme(root: &Path) {
    write(root, "ops/goatbuilder/overlay/families/rack.yaml", "schema: goatbuilder-family/v1\nfamily: rack\ntitle: A rack server\nmodels: [rack-1]\n");
    write(
        root,
        "ops/goatbuilder/overlay/targets/acme.yaml",
        "schema: goatbuilder-target/v1\ntype: acme\nfamilies: [rack]\nsite: {declared_by: has_acme_node, absent_reason: acme_absent_reason, standup: phase-14}\nphases:\n  - label: the acme appliance at {site}\n    plays:\n      - {play: acme-deploy.yml, args: \"-e target={group}\", from: overlay}\n",
    );
}

/// The exit code, stdout and stderr of one run.
fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let home = root.join("home");
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "--yes", args[0], "-C", root.to_str().unwrap()])
        .args(&args[1..])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .expect("run goatbuilder");
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn join<'a>(class: &'a str, node: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut v = vec!["join-node", "zagros", "--site", "zagros", "--node", node, "--class", class, "--address", "192.0.2.9"];
    v.extend(rest);
    v
}

#[test]
fn a_private_type_needs_no_public_change() {
    let t = checkout();
    let (rc, out) = run(t.path(), &join("acme", "acme-1", &["--dry-run"]));
    assert!(rc != 0 && out.contains("GB-247") && out.contains("known: breakglass, core, federation-boundary, first-bite, goat-processors, network-jump, network-tap, obs, octo-pod, phone, simple-boundary"), "no such type yet: {out}");
    with_acme(t.path());
    let (rc, out) = run(t.path(), &join("acme", "acme-1", &["--dry-run"]));
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("=== Phase 13 [A] [acme] the acme appliance at zagros ===") && out.contains("acme-deploy.yml -e target=zagros_acme_hosts -l acme-1"), "{out}");
    assert!(out.contains("mgmt-host-baseline.yml -e target=zagros_acme_hosts"), "the node's own group throughout: {out}");
    // standup builds the site's declared machine of the type, in phase 14.
    let standup = ["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run"];
    let (rc, out) = run(t.path(), &standup);
    assert!(rc == 0 && out.contains("has_acme_node is not true"), "{out}");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_acme_node: true\n");
    let (rc, out) = run(t.path(), &standup);
    assert!(rc == 0 && out.contains("=== Phase 14 [A] the acme appliance at zagros ===") && out.contains("acme-deploy.yml -e target=zagros_acme_hosts"), "{out}");
}

/// A phase-14 type with no entry for this kind of site says so, rather than
/// building nothing and printing nothing.
#[test]
fn a_phase_14_type_with_nothing_for_this_site_says_so() {
    let t = checkout();
    with_acme(t.path());
    let text = std::fs::read_to_string(t.path().join("ops/goatbuilder/overlay/targets/acme.yaml")).unwrap();
    write(t.path(), "ops/goatbuilder/overlay/targets/acme.yaml", &text.replace("  - label: the acme appliance at {site}\n", "  - label: the acme appliance at {site}\n    when: joiner\n"));
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_acme_node: true\n");
    let (rc, out) = run(t.path(), &["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run"]);
    assert!(rc == 0 && out.contains("no phase entry of the type acme is for a founder site") && !out.contains("acme-deploy.yml"), "{out}");
}

#[test]
fn a_run_without_the_play_is_refused_before_its_first_phase() {
    let t = checkout();
    with_acme(t.path());
    let (rc, out) = run(t.path(), &join("acme", "acme-1", &[]));
    assert!(rc != 0 && out.contains("GB-377") && out.contains("ops/ansible/acme-deploy.yml") && out.contains("private overlay") && !out.contains("=== Phase"), "{rc} {out}");
    // goatcore does not have the network-jump play either: goatstuff does.
    write(t.path(), "ops/ansible/group_vars/zagros_network_jump_hosts/vars.yml", &format!("herd_connector_image: \"{IMAGE}\"\n"));
    let (rc, out) = run(t.path(), &join("network-jump", "nj-1", &[]));
    assert!(rc != 0 && out.contains("GB-377") && out.contains("USAREUR-AF/goatstuff"), "{rc} {out}");
    // A run that stops before phase 13 does not need it: the refusal is another one.
    let (_, out) = run(t.path(), &join("acme", "acme-1", &["--from", "3", "--to", "3"]));
    assert!(!out.contains("GB-377"), "{out}");
    // standup, for the machines the site declares.
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_acme_node: true\n");
    let (rc, out) = run(t.path(), &["standup", "zagros", "--site", "zagros", "--profile", "lan"]);
    assert!(rc != 0 && out.contains("GB-377") && out.contains("--to 13"), "{rc} {out}");
}

/// An image an operator supplies, pinned by digest (synthetic).
const IMAGE: &str = "ghcr.io/example-org/herd-connector:1.0@sha256:0000000000000000000000000000000000000000000000000000000000000001";

#[test]
fn a_machine_without_its_types_input_is_refused_before_any_phase() {
    let t = checkout();
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_network_jump_hosts]\nnj-1\n");
    let plan = join("network-jump", "nj-1", &["--dry-run"]);
    let (rc, out) = run(t.path(), &plan);
    assert!(rc == 2 && out.contains("GB-414") && out.contains("nj-1, a network-jump machine, needs herd_connector_image: it is not set") && !out.contains("=== Phase"), "{rc} {out}");
    // The role's placeholder is not a value.
    write(t.path(), "ops/ansible/group_vars/zagros_network_jump_hosts/vars.yml", "herd_connector_image: PENDING\n");
    let (rc, out) = run(t.path(), &plan);
    assert!(rc == 2 && out.contains("sets it to 'PENDING', which is a placeholder") && out.contains("group_vars/zagros_network_jump_hosts/vars.yml"), "{rc} {out}");
    // Set where Ansible reads it for the machine: its group's vars, or its own host_vars, which win.
    write(t.path(), "ops/ansible/group_vars/zagros_network_jump_hosts/vars.yml", &format!("herd_connector_image: \"{IMAGE}\"\n"));
    let (rc, out) = run(t.path(), &plan);
    assert!(rc == 0 && out.contains(&format!("nj-1: herd_connector_image: {IMAGE} (ops/ansible/group_vars/zagros_network_jump_hosts/vars.yml)")), "{rc} {out}");
    write(t.path(), "ops/ansible/host_vars/nj-1.yml", "herd_connector_image: PENDING\n");
    let (rc, out) = run(t.path(), &plan);
    assert!(rc == 2 && out.contains("host_vars/nj-1.yml sets it to 'PENDING'"), "{rc} {out}");
    std::fs::remove_file(t.path().join("ops/ansible/host_vars/nj-1.yml")).unwrap();
    // doctor --target shows it as a row of its own.
    let (_, out) = run(t.path(), &["doctor", "--target", "network-jump", "--site", "zagros", "--all"]);
    assert!(out.contains("input nj-1 herd_connector_image") && out.contains(IMAGE), "{out}");
    // standup refuses a run that reaches phase 14 before any phase; a dry run plans.
    write(t.path(), "ops/ansible/group_vars/zagros_network_jump_hosts/vars.yml", "herd_connector_image: PENDING\n");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_network_jump_node: true\n");
    let (rc, out) = run(t.path(), &["standup", "zagros", "--site", "zagros", "--profile", "lan"]);
    assert!(rc == 2 && out.contains("GB-414") && !out.contains("=== Phase"), "{rc} {out}");
    let (rc, out) = run(t.path(), &["standup", "zagros", "--site", "zagros", "--profile", "lan", "--from", "1", "--to", "13"]);
    assert!(!out.contains("GB-414"), "{rc} {out}");
}

#[test]
fn a_machine_of_the_wrong_family_is_refused() {
    let t = checkout();
    with_acme(t.path());
    write(t.path(), "ops/ansible/host_vars/acme-1.yml", "platform_family: small-computer\n");
    let (rc, out) = run(t.path(), &join("acme", "acme-1", &["--dry-run"]));
    assert!(rc != 0 && out.contains("GB-376") && out.contains("host_vars/acme-1.yml"), "{rc} {out}");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_acme_node: true\n");
    let (rc, out) = run(t.path(), &["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run"]);
    assert!(rc != 0 && out.contains("GB-376"), "{rc} {out}");
    write(t.path(), "ops/ansible/host_vars/acme-1.yml", "platform_family: rack\n");
    let (rc, out) = run(t.path(), &join("acme", "acme-1", &["--dry-run"]));
    assert_eq!(rc, 0, "{out}");
}

#[test]
fn a_type_file_with_an_unknown_field_is_refused() {
    let t = checkout();
    with_acme(t.path());
    let at = "ops/goatbuilder/overlay/targets/acme.yaml";
    let text = std::fs::read_to_string(t.path().join(at)).unwrap();
    write(t.path(), at, &format!("{text}colour: red\n"));
    let (rc, out) = run(t.path(), &join("core", "node-x", &["--dry-run"]));
    assert!(rc != 0 && out.contains("GB-374") && out.contains(at) && out.contains("colour"), "{rc} {out}");
    write(t.path(), at, &text.replace("[rack]", "[mainframe]"));
    let (rc, out) = run(t.path(), &join("core", "node-x", &["--dry-run"]));
    assert!(rc != 0 && out.contains("GB-375") && out.contains("mainframe"), "{rc} {out}");
}

/// An overlay copied over goatcore changes no built-in type, through the binary.
#[test]
fn an_assembled_tree_whose_overlay_replaces_a_type_is_refused() {
    let t = checkout();
    let rel = "ops/goatbuilder/targets/breakglass.yaml";
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)).unwrap();
    write(t.path(), rel, &text);
    let git = |args: &[&str]| {
        let o = Command::new("git").arg("-C").arg(t.path()).args(["-c", "user.name=t", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "goatcore"]);
    write(t.path(), ".goat-assembled", &format!("goatcore_ref={}\noverlay_commit={}\n", git(&["rev-parse", "HEAD"]), "b".repeat(40)));
    let args = join("breakglass", "bg-1", &["--dry-run"]);
    let standup = ["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run"];
    let (rc, out) = run(t.path(), &args);
    assert_eq!(rc, 0, "{out}");
    write(t.path(), rel, &text.replace("breakglass-nodes.yml", "other.yml"));
    for a in [&args[..], &standup[..]] {
        let (rc, out) = run(t.path(), a);
        assert!(rc != 0 && out.contains("GB-378") && out.contains(rel), "{rc} {out}");
    }
}

/// A type that declares its own inventory, whose machines only it names: every
/// play from phase 5 on is given that inventory after the goatnet's base and
/// before the transport override, so the node is found by the type's group
/// and reached by the transport, as phase 13 reaches it.
#[test]
fn a_types_own_inventory_is_given_to_every_play_before_the_transport() {
    let t = checkout();
    with_acme(t.path());
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n");
    write(t.path(), "ops/ansible/inventory/zagros-acme.ini", "[zagros_acme_hosts]\nacme-1\n");
    let rel = "ops/goatbuilder/overlay/targets/acme.yaml";
    let text = std::fs::read_to_string(t.path().join(rel)).unwrap();
    assert!(text.contains("families: [rack]\n"), "{text}");
    write(t.path(), rel, &text.replace("families: [rack]\n", "families: [rack]\ninventory: \"inventory/{site}-acme.ini\"\n"));
    let (rc, out) = run(t.path(), &join("acme", "acme-1", &["--dry-run"]));
    assert_eq!(rc, 0, "{out}");
    let want = "-i inventory/mgmt-hosts.ini -i inventory/zagros-acme.ini -i inventory/mgmt-hosts-mesh.ini ";
    let mut seen = Vec::new();
    for block in out.split("=== Phase ").skip(1) {
        let n: u32 = block.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("").parse().unwrap_or(0);
        if block.contains("ansible-preflight.sh") {
            assert!(block.contains(want), "phase {n} is not given the type's inventory before the transport's: {block}");
            seen.push(n);
        }
    }
    // Phase 10's enrolment is refused for a class that declares no NetBird
    // groups of its own (GB-430), so it runs no play for this one.
    for n in [5, 6, 11, 13] {
        assert!(seen.contains(&n), "phase {n} runs no play here ({seen:?}): {out}");
    }
    assert!(!seen.contains(&10) && out.contains("GB-430"), "{out}");
}
