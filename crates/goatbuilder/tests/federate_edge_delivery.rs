//! Phase 12 from a driver that is not the edge host: the edge mounts the files
//! phase 8 wrote in `<work>/pki` on the driver's machine, and nothing copies
//! them to the edge unless the role installs them (goat_zenoh_edge_install_certs).
//! Through the real binary on the federate tests' stand-in world, whose
//! inventory names the edge by an address of its own (192.0.2.10).

mod federate_world;

use federate_world::{rows, says, world, write};

/// Phases 1 to 11 done, then the edge group's vars point the edge at the
/// working directory's files, as a deployment rendered with the install off does.
fn at_phase_12(install: bool) -> federate_world::World {
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--to", "11"]);
    assert!(out.status.success(), "{}", says(&out));
    let pki = w.work.join("pki");
    let mut vars = format!(
        "goat_zenoh_edge_our_ca_path: {p}/our-ca.pem\ngoat_zenoh_edge_outward_cert: {p}/peering-outward.pem\n",
        p = pki.display()
    );
    if install {
        vars.push_str("goat_zenoh_edge_install_certs: true\n");
    }
    write(
        &w.repo
            .join("ops/ansible/group_vars/hello-ally_edge/mounts.yml"),
        &vars,
    );
    w.clear_log();
    w
}

#[test]
fn a_driver_that_is_not_the_edge_host_is_refused_before_phase_12_when_nothing_installs_the_files() {
    let w = at_phase_12(false);
    let (out, v) = w.run(&["hello-ally", "--from", "12", "--to", "12"]);
    assert!(!out.status.success(), "{}", says(&out));
    let failed = rows(&v, "failed");
    assert_eq!(failed.len(), 1, "{}", says(&out));
    assert!(failed[0].starts_with("GB-275: "), "{}", failed[0]);
    assert!(failed[0].contains("edge-1 (192.0.2.10)"), "{}", failed[0]);
    assert!(
        failed[0].contains("goat_zenoh_edge_install_certs: true"),
        "{}",
        failed[0]
    );
    // Its banner prints, as for every guard; its play does not run.
    assert!(
        !w.log().contains("peering-zenoh-edge.yml"),
        "the edge play ran: {}",
        w.log()
    );
}

#[test]
fn with_the_roles_install_on_phase_12_runs() {
    let w = at_phase_12(true);
    let (out, _) = w.run(&["hello-ally", "--from", "12", "--to", "12"]);
    assert!(!says(&out).contains("GB-275"), "{}", says(&out));
    assert!(
        w.log().contains("peering-zenoh-edge.yml"),
        "the edge play did not run: {}",
        w.log()
    );
}
