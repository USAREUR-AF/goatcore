//! Gate row B6: one environment holds several peerings, each with its own
//! directory package, and that is the default. Phase 5's directory step runs
//! `ops/federation/standup/directory.py`; here a stand-in for it records the
//! arguments and the switch variable it was given, so the test reads what the
//! real binary asks of the directory for each peering. What directory.py does
//! with `--per-counterparty` (one slot per counterparty, neither peering's
//! package replacing the other's) is tested against a stand-in directory in
//! tests/test_fed_directory.py.

mod federate_world;

use federate_world::{exe, phases_run, says, world, write, World};

const URL: &str = "https://192.0.2.40:8443";

/// Replace the directory step with a stand-in that appends one line per run:
/// `directory <args> | FED_DIRECTORY_PER_COUNTERPARTY=<value or unset>`.
fn stand_in_directory(w: &World) {
    exe(
        &w.repo.join("ops/federation/standup/directory.py"),
        &format!(
            "#!/usr/bin/env python3\nimport os, sys\nv = os.environ.get('FED_DIRECTORY_PER_COUNTERPARTY')\nopen('{}', 'a').write('directory ' + ' '.join(sys.argv[1:]) + ' | FED_DIRECTORY_PER_COUNTERPARTY=' + ('unset' if v is None else v) + '\\n')\n",
            w.log.display()
        ),
    );
}

/// The directory lines of the log, one per directory step run.
fn directory_runs(w: &World) -> Vec<String> {
    w.log()
        .lines()
        .filter(|l| l.starts_with("directory "))
        .map(String::from)
        .collect()
}

/// A run of `partner` through the directory, up to phase 5's stop.
fn run_to_phase_5(
    w: &World,
    partner: &str,
    counterparty: &str,
    work: &std::path::Path,
    switch: Option<&str>,
) -> (std::process::Output, serde_json::Value) {
    let mut c = w.command(&[partner]);
    c.env_remove("FED_NO_DIRECTORY")
        .env("FED_DIRECTORY_URL", URL)
        .env("FED_COUNTERPARTY_ID", counterparty)
        .env("FED_WORK", work);
    if let Some(v) = switch {
        c.env("FED_DIRECTORY_PER_COUNTERPARTY", v);
    }
    let out = c.output().unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
}

/// The argument after `flag` in one recorded directory run.
fn arg<'a>(line: &'a str, flag: &str) -> Option<&'a str> {
    let mut it = line.split(' ');
    it.find(|a| *a == flag)?;
    it.next()
}

#[test]
fn the_directory_step_publishes_one_package_per_counterparty_by_default() {
    let w = world();
    stand_in_directory(&w);
    let (out, v) = run_to_phase_5(&w, "hello-ally", "xab", &w.work.clone(), None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5], "{}", says(&out));
    let runs = directory_runs(&w);
    assert_eq!(runs.len(), 1, "{}\n{}", w.log(), says(&out));
    assert!(
        runs[0].split(' ').any(|a| a == "--per-counterparty"),
        "phase 5 must ask for this peering's own package slot by default: {}",
        runs[0]
    );
    assert_eq!(arg(&runs[0], "--their-id"), Some("xab"), "{}", runs[0]);
    // The plan shows the command it runs.
    let plan = String::from_utf8_lossy(&out.stdout);
    assert!(
        plan.contains("--their-id 'xab' --per-counterparty"),
        "the plan names the switch: {}",
        says(&out)
    );
}

#[test]
fn the_switch_variable_set_by_the_operator_is_left_to_the_directory_step() {
    // Any value the operator sets is the directory step's to read (1, true, yes
    // on; 0, false, no or empty off; any other word refused by it): the
    // command adds no flag that would override it.
    for value in ["0", "no", "1"] {
        let w = world();
        stand_in_directory(&w);
        let (out, _) = run_to_phase_5(&w, "hello-ally", "xab", &w.work.clone(), Some(value));
        assert!(out.status.success(), "{}", says(&out));
        let runs = directory_runs(&w);
        assert_eq!(runs.len(), 1, "{}", w.log());
        assert!(
            !runs[0].split(' ').any(|a| a == "--per-counterparty"),
            "FED_DIRECTORY_PER_COUNTERPARTY={value}: {}",
            runs[0]
        );
        assert!(
            runs[0].ends_with(&format!("| FED_DIRECTORY_PER_COUNTERPARTY={value}")),
            "{}",
            runs[0]
        );
    }
}

/// One environment, two peerings: `hello-ally` (counterparty xab) and a second
/// partner `second-ally` (counterparty xac) on the same edge class at the same
/// site, each with its own working folder.
fn add_second_peering(w: &World) -> std::path::PathBuf {
    let decl = w.repo.join("ops/goatbuilder/goatbuilder.yaml");
    let text = std::fs::read_to_string(&decl).unwrap();
    let entry =
        "  - with: hello-ally\n    environment: zagros\n    site: alpha\n    edge_class: edge\n";
    assert!(text.contains(entry), "the fixture's peering entry moved");
    let both = text.replacen(
        entry,
        &format!("{entry}  - with: second-ally\n    environment: zagros\n    site: alpha\n    edge_class: edge\n"),
        1,
    );
    std::fs::write(&decl, both).unwrap();
    write(
        &w.repo.join("ops/ansible/group_vars/second-ally_edge/peering.yml"),
        "peering_wg_interface: wg-fed2\npeering_wg_listen_port: 51822\ngoat_zenoh_edge_listen_port: 7449\n",
    );
    write(
        &w.repo
            .join("ops/ansible/group_vars/second-ally_core/peering.yml"),
        "goat_fed_republisher_metrics_port: 9465\n",
    );
    let inv = w.repo.join("ops/ansible/inventory/peering-edges.ini");
    let mut ini = std::fs::read_to_string(&inv).unwrap();
    ini.push_str("\n[second-ally_edge]\nedge-1-second ansible_host=192.0.2.10\n\n[second-ally_core]\ncore-1-second ansible_host=192.0.2.11\n");
    std::fs::write(&inv, ini).unwrap();
    let work = w.work.parent().unwrap().join("second-ally");
    std::fs::create_dir_all(&work).unwrap();
    work
}

#[test]
fn one_environment_with_two_peerings_publishes_each_peerings_own_package() {
    let w = world();
    stand_in_directory(&w);
    let second = add_second_peering(&w);
    let (out, v) = run_to_phase_5(&w, "hello-ally", "xab", &w.work.clone(), None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5], "{}", says(&out));
    let (out, v) = run_to_phase_5(&w, "second-ally", "xac", &second, None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5], "{}", says(&out));

    let runs = directory_runs(&w);
    assert_eq!(runs.len(), 2, "{}", w.log());
    for r in &runs {
        assert!(r.split(' ').any(|a| a == "--per-counterparty"), "{r}");
    }
    // Each peering publishes its own package, for its own counterparty, from
    // its own working folder: the directory keeps one slot per counterparty.
    let first_work = w.work.display().to_string();
    let second_work = second.display().to_string();
    assert_eq!(arg(&runs[0], "--their-id"), Some("xab"), "{}", runs[0]);
    assert_eq!(arg(&runs[1], "--their-id"), Some("xac"), "{}", runs[1]);
    assert_eq!(
        arg(&runs[0], "--ours"),
        Some(format!("{first_work}/ours.canonical.json").as_str())
    );
    assert_eq!(
        arg(&runs[1], "--ours"),
        Some(format!("{second_work}/ours.canonical.json").as_str())
    );
    assert_eq!(arg(&runs[0], "--work"), Some(first_work.as_str()));
    assert_eq!(arg(&runs[1], "--work"), Some(second_work.as_str()));
}
