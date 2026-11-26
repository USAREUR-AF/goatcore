// `goatbuilder join-node <env> --site <site> --node <node> --class phone --reseed`:
// a node a slot of which was invalidated is rebuilt from its founder in eight
// steps (docs/operations/spock-site-rebuild.md), and `--slots` is the
// detector. On the phone site `delta` of the fixture
// `ops/goatbuilder/fixtures/join-node-third-site`, which joins `alpha`.
//
// Nothing here reaches a host or a database. The real inventory lookup
// (`inventory-hosts.py`) runs; `spock-reseed.py` is a stand-in that records
// its arguments and writes the journal the real one writes, refusing at a
// step when told to. The real helper runs against two postgres-spock
// containers, which is recorded in the design document, not here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NODE: &str = "delta-phone-1";
const NODE_MESH: &str = "100.127.0.4";
const STEPS: [&str; 8] = [
    "lost",
    "caught-up",
    "export",
    "size",
    "reseed",
    "synced",
    "replay",
    "converged",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

struct Site {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    checkout: PathBuf,
    log: PathBuf,
}

impl Site {
    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
    fn steps_run(&self) -> Vec<String> {
        self.log()
            .iter()
            .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
            .collect()
    }
    fn journal(&self) -> PathBuf {
        self.root
            .join(format!(".goat/state/join-node-delta-{NODE}-reseed.json"))
    }
}

/// `enrolled`: the node's mesh address is in the mesh inventory.
fn site(enrolled: bool) -> Site {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let checkout = root.join("checkout");
    let r = repo_root();
    copy_tree(
        &r.join("ops/goatbuilder/fixtures/join-node-third-site"),
        &checkout,
    );
    std::fs::create_dir_all(checkout.join("ops/scripts")).unwrap();
    std::fs::copy(
        r.join("ops/scripts/inventory-hosts.py"),
        checkout.join("ops/scripts/inventory-hosts.py"),
    )
    .unwrap();
    let defaults = "ops/ansible/roles/spock-mesh-peering/defaults/main.yml";
    std::fs::create_dir_all(checkout.join(defaults).parent().unwrap()).unwrap();
    std::fs::copy(r.join(defaults), checkout.join(defaults)).unwrap();
    if enrolled {
        let inv = checkout.join("ops/ansible/inventory/mgmt-hosts-mesh.ini");
        let text = std::fs::read_to_string(&inv)
            .unwrap()
            .replace("TBD_DELTA_MESH_IP_A", NODE_MESH);
        std::fs::write(&inv, text).unwrap();
    }
    let log = root.join("reseed.log");
    // The stand-in: one line per call, "<step> <the rest of its arguments>",
    // and the journal entry the real helper writes. RESEED_REFUSE=<step>:<code>
    // makes that step refuse as the real one does.
    let stub = format!(
        r#"import json, os, sys
step = sys.argv[1]
args = sys.argv[2:]
with open({log:?}, "a") as f:
    f.write(step + " " + " ".join(args) + "\n")
if step == "slots":
    # The detector's JSON: SLOTS_LOST=1 makes the founder's slot for the
    # node read lost.
    lost = os.environ.get("SLOTS_LOST") == "1"
    print(json.dumps({{"schema": "goatbuilder.spock-slots/v1", "lost": int(lost), "slots": [
        {{"db": "netbird", "direction": "in", "subscription": "sub_alpha_to_delta", "provider": "alpha",
          "subscriber": "delta", "slot": "spk_netbird_alpha_sub_alpha_to_delta", "slot_at": "alpha",
          "status": "replicating", "wal_status": "lost" if lost else "reserved",
          "verdict": "cannot recover: alpha no longer holds this node's changes" if lost else "can recover"}},
        {{"db": "netbird", "direction": "out", "subscription": "sub_delta_to_alpha", "provider": "delta",
          "subscriber": "alpha", "slot": "spk_netbird_delta_sub_delta_to_alpha", "slot_at": "delta",
          "status": "replicating", "wal_status": "reserved", "verdict": "can recover"}}]}}))
    sys.exit(1 if lost else 0)
journal = args[args.index("--journal") + 1]
doc = {{"schema": "goatbuilder.spock-reseed/v2", "steps": {{}}}}
if os.path.exists(journal):
    doc = json.load(open(journal))
refuse = os.environ.get("RESEED_REFUSE", "")
if refuse.split(":")[0] == step:
    code = refuse.split(":")[1]
    doc["steps"][step] = {{"at": "2026-11-15T00:00:00Z", "ok": False, "code": code, "line": "the stand-in refuses " + step}}
    rc = 1
    print(code + ": the stand-in refuses " + step, file=sys.stderr)
else:
    doc["steps"][step] = {{"at": "2026-11-15T00:00:00Z", "ok": True}}
    rc = 0
os.makedirs(os.path.dirname(journal), exist_ok=True)
json.dump(doc, open(journal, "w"))
sys.exit(rc)
"#,
        log = log.display().to_string()
    );
    std::fs::write(checkout.join("ops/scripts/spock-reseed.py"), stub).unwrap();
    // ssh is never started by the verb itself: the helper is.
    Site {
        _tmp: tmp,
        root,
        checkout,
        log,
    }
}

fn reseed(s: &Site, extra: &[&str], refuse: Option<&str>) -> (Output, serde_json::Value) {
    let mut args: Vec<String> = vec![
        "-C".into(),
        s.checkout.display().to_string(),
        "--yes".into(),
        "-o".into(),
        "json".into(),
        "join-node".into(),
        "zagros".into(),
        "--site".into(),
        "delta".into(),
        "--node".into(),
        NODE.into(),
        "--class".into(),
        "phone".into(),
        "--via".into(),
        "usb".into(),
        "--reseed".into(),
    ];
    args.extend(extra.iter().map(|a| a.to_string()));
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    c.args(&args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &s.root)
        .env("GOAT_NO_FUN", "1");
    if let Some(r) = refuse {
        c.env("RESEED_REFUSE", r);
    }
    let out = c.output().unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
}

fn says(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A dry run prints the eight steps, each with the helper's command, and runs
/// nothing.
#[test]
fn a_dry_run_prints_the_eight_steps_and_runs_nothing() {
    let s = site(true);
    let (out, _) = reseed(&s, &["--dry-run"], None);
    let all = says(&out);
    assert!(out.status.success(), "{all}");
    let mut at = 0;
    for step in STEPS {
        let i = all[at..]
            .find(&format!("spock-reseed.py {step} "))
            .unwrap_or_else(|| panic!("step {step} is not in the plan, in order: {all}"));
        at += i + 1;
    }
    assert!(
        all.contains(&s.journal().display().to_string()),
        "the plan names no journal: {all}"
    );
    assert!(s.log().is_empty(), "a dry run ran a step: {:?}", s.log());
}

/// The eight steps run in order, each against the node at its mesh address and
/// the founder site's first host, through the Postgres container the
/// replication role names.
#[test]
fn a_run_takes_the_eight_steps_in_order_against_the_node_and_its_founder() {
    let s = site(true);
    let (out, v) = reseed(&s, &[], None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(v["exit"], 0, "{v}");
    assert_eq!(s.steps_run(), STEPS.to_vec(), "{:?}", s.log());
    let first = &s.log()[0];
    assert!(
        first.contains(&format!(
            "--node-psql ssh operator@{NODE_MESH} sudo docker exec -i postgres-a psql -U postgres"
        )),
        "{first}"
    );
    assert!(
        first.contains(
            "--site-psql ssh operator@100.127.0.1 sudo docker exec -i postgres-a psql -U postgres"
        ),
        "{first}"
    );
    assert!(
        first.contains(&format!("--journal {}", s.journal().display())),
        "{first}"
    );
}

/// A refusal stops the run at its step with the helper's code, and no later
/// step runs. Run again, the verb starts at the step that refused: the steps
/// the journal records as passed are not taken again.
#[test]
fn a_refused_step_stops_the_run_with_its_code_and_a_rerun_resumes_there() {
    let s = site(true);
    let (out, v) = reseed(&s, &[], Some("caught-up:GB-346"));
    let all = says(&out);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert!(
        all.contains("GB-346") && all.contains("the stand-in refuses caught-up"),
        "{all}"
    );
    assert_eq!(s.steps_run(), vec!["lost", "caught-up"], "{:?}", s.log());
    assert_eq!(v["ok"], false, "{v}");

    std::fs::remove_file(&s.log).unwrap();
    let (out, _) = reseed(&s, &[], None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(
        s.steps_run(),
        STEPS[1..].to_vec(),
        "the rerun did not resume at the refused step: {:?}",
        s.log()
    );
}

/// A re-seed that finished is kept beside the journal, and the next one, for
/// a later absence, starts at the first step.
#[test]
fn a_finished_reseed_is_kept_and_the_next_one_starts_at_the_first_step() {
    let s = site(true);
    let (out, _) = reseed(&s, &[], None);
    assert!(out.status.success(), "{}", says(&out));
    std::fs::remove_file(&s.log).unwrap();
    let (out, _) = reseed(&s, &[], None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(s.steps_run(), STEPS.to_vec(), "{:?}", s.log());
    let kept: Vec<_> = std::fs::read_dir(s.journal().parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            n.starts_with(&format!("join-node-delta-{NODE}-reseed.")) && n.ends_with(".done.json")
        })
        .collect();
    assert_eq!(kept.len(), 1, "the finished journal was not kept: {kept:?}");
}

/// A node with no mesh address yet has nothing to seed again: the first step
/// says so and nothing else runs.
#[test]
fn a_node_with_no_mesh_address_is_refused_before_any_step() {
    let s = site(false);
    let (out, _) = reseed(&s, &[], None);
    let all = says(&out);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert!(all.contains("no mesh address"), "{all}");
    assert!(s.log().is_empty(), "{:?}", s.log());
}

/// Only a replicating node is seeded again: an obs node is not a Spock member.
#[test]
fn reseed_is_for_a_node_that_replicates() {
    let s = site(true);
    let out = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args([
            "-C",
            s.checkout.to_str().unwrap(),
            "--yes",
            "-o",
            "json",
            "join-node",
            "zagros",
            "--site",
            "alpha",
            "--node",
            "alpha-obs-1",
            "--class",
            "obs",
            "--reseed",
            "--dry-run",
        ])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &s.root)
        .env("GOAT_NO_FUN", "1")
        .output()
        .unwrap();
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("GB-247") && all.contains("--reseed"), "{all}");
}

fn slots(s: &Site, extra: &[&str], lost: bool) -> (Output, serde_json::Value) {
    let mut args: Vec<String> = [
        "-C",
        s.checkout.to_str().unwrap(),
        "-o",
        "json",
        "join-node",
        "zagros",
        "--site",
        "delta",
        "--node",
        NODE,
        "--class",
        "phone",
        "--via",
        "usb",
        "--slots",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    args.extend(extra.iter().map(|a| a.to_string()));
    let out = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(&args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &s.root)
        .env("GOAT_NO_FUN", "1")
        .env("SLOTS_LOST", if lost { "1" } else { "0" })
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
}

/// The detector reads only, so it runs without `--yes` and without a
/// terminal. It reaches the node, its founder and each `--peer` at their
/// mesh addresses, and every slot is one line with its wal_status.
#[test]
fn the_detector_runs_without_yes_and_lists_every_slot() {
    let s = site(true);
    let (out, v) = slots(&s, &["--peer", "beta"], false);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(v["exit"], 0, "{v}");
    let line = &s.log()[0];
    assert!(line.starts_with("slots "), "{line}");
    assert!(
        line.contains(&format!(
            "--node-psql ssh operator@{NODE_MESH} sudo docker exec -i postgres-a psql -U postgres"
        )),
        "{line}"
    );
    assert!(
        line.contains("--site-psql ssh operator@100.127.0.1 "),
        "{line}"
    );
    assert!(
        line.contains("--peer-psql beta=ssh operator@100.127.0.2 sudo docker exec -i postgres-a psql -U postgres"),
        "{line}"
    );
    let slot_lines: Vec<_> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "slot")
        .collect();
    assert_eq!(slot_lines.len(), 2, "{v}");
    assert!(slot_lines.iter().all(|c| c["ok"] == true), "{v}");
}

/// A lost slot is named, and the detector refuses with GB-387 and says how
/// the node is rebuilt.
#[test]
fn the_detector_names_a_lost_slot_and_refuses_with_its_code() {
    let s = site(true);
    let (out, v) = slots(&s, &[], true);
    let all = says(&out);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert!(all.contains("GB-387"), "{all}");
    assert!(
        all.contains("spk_netbird_alpha_sub_alpha_to_delta (at alpha)"),
        "{all}"
    );
    assert!(all.contains("--reseed --dry-run"), "{all}");
    assert_eq!(v["ok"], false, "{v}");
}

/// A dry run of the detector prints its command and reads nothing.
#[test]
fn a_dry_run_of_the_detector_reads_nothing() {
    let s = site(true);
    let (out, _) = slots(&s, &["--dry-run"], true);
    let all = says(&out);
    assert!(out.status.success(), "{all}");
    assert!(all.contains("spock-reseed.py slots "), "{all}");
    assert!(s.log().is_empty(), "{:?}", s.log());
}

/// `--peer` names another site: the node's own site or its founder is refused.
#[test]
fn a_peer_that_is_the_founder_is_refused() {
    let s = site(true);
    let (out, _) = slots(&s, &["--peer", "alpha"], false);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(
        all.contains("GB-247") && all.contains("--peer alpha"),
        "{all}"
    );
    assert!(s.log().is_empty(), "{:?}", s.log());
}

/// The export step refuses until its digest is accepted. Run again with
/// `--accept`, the verb resumes at the export step and gives the digest to
/// every step it runs, with each `--peer` as well.
#[test]
fn an_unaccepted_export_stops_the_run_and_accept_resumes_it_with_the_peers() {
    let s = site(true);
    let (out, _) = reseed(&s, &["--peer", "beta"], Some("export:GB-388"));
    let all = says(&out);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert!(all.contains("GB-388"), "{all}");
    assert_eq!(
        s.steps_run(),
        vec!["lost", "caught-up", "export"],
        "{:?}",
        s.log()
    );
    assert!(
        s.log()[0].contains("--peer-psql beta=ssh operator@100.127.0.2 "),
        "{:?}",
        s.log()
    );

    std::fs::remove_file(&s.log).unwrap();
    let digest = "8f65fca2f3eed1728daf04f6dbe92806c2e1a05909d44c9b46801b7eb863a2ff";
    let (out, _) = reseed(&s, &["--peer", "beta", "--accept", digest], None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(s.steps_run(), STEPS[2..].to_vec(), "{:?}", s.log());
    assert!(
        s.log()
            .iter()
            .all(|l| l.contains(&format!("--accept {digest}")) && l.contains("--peer-psql beta=")),
        "{:?}",
        s.log()
    );
}

/// `--peer` belongs to `--slots` or `--reseed`, and `--accept` to `--reseed`.
#[test]
fn peer_and_accept_go_with_the_verbs_that_read_them() {
    let s = site(true);
    let run = |extra: &[&str]| {
        let mut args = vec![
            "-C",
            s.checkout.to_str().unwrap(),
            "--yes",
            "join-node",
            "zagros",
            "--site",
            "delta",
            "--node",
            NODE,
            "--class",
            "phone",
            "--via",
            "usb",
            "--dry-run",
        ];
        args.extend_from_slice(extra);
        Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
            .args(&args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &s.root)
            .env("GOAT_NO_FUN", "1")
            .output()
            .unwrap()
    };
    let out = run(&["--peer", "beta"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("GB-247") && all.contains("--peer"), "{all}");
    let out = run(&["--slots", "--accept", "abc"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("GB-247") && all.contains("--accept"), "{all}");
    assert!(s.log().is_empty(), "{:?}", s.log());
}
