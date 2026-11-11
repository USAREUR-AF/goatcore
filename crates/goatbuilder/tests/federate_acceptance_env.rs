//! Phase 14 finds our core and edge declarations by group: the driver hands
//! it the edge group the run uses and the core group, through the
//! environment, so the phase's command line (the parity gate) is unchanged.

mod federate_world;

use federate_world::{exe, says, world};

#[test]
fn acceptance_is_given_the_runs_edge_and_core_groups() {
    let w = world();
    let log = w.log.display().to_string();
    exe(
        &w.repo.join("ops/federation/standup/acceptance.sh"),
        &format!(
            "#!/bin/sh\necho \"acceptance edge=$FED_EDGE_GROUP core=$FED_CORE_GROUP\" >> '{log}'\n"
        ),
    );
    let (out, _) = w.run(&[
        "hello-ally",
        "--from",
        "14",
        "--to",
        "14",
        "--edge-group",
        "site_edge",
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        w.log()
            .contains("acceptance edge=site_edge core=hello-ally_core"),
        "{}\n{}",
        w.log(),
        says(&out)
    );

    // An edge group left in the environment by an earlier shell does not
    // override the run's own.
    w.clear_log();
    let out = w
        .command(&["hello-ally", "--from", "14", "--to", "14"])
        .env("FED_EDGE_GROUP", "stale_edge")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        w.log().contains("acceptance edge=hello-ally_edge "),
        "{}",
        w.log()
    );
}
