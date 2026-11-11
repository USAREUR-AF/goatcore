//! What `goatbuilder federate <partner>` reads from the deployment's own
//! declarations instead of from a person, through the real binary against a
//! stand-in repository (federate_world).

mod federate_world;

use federate_world::{exe, says, world, write};

/// Phase 8 gets the core certificate's names from the deployment's
/// declarations when FED_CORE_SAN is unset: here, the endpoints the edge dials
/// in its rendered peering-deployment.yml, then the core's inventory host. A
/// set FED_CORE_SAN still wins.
#[test]
fn phase_8_names_the_core_from_the_declarations() {
    let w = world();
    let log = w.root.path().join("log");
    exe(
        &w.repo.join("ops/federation/standup/mint-peering-leaves.sh"),
        &format!(
            "#!/bin/sh\necho \"mint san=$FED_CORE_SAN\" >> '{}'\n",
            log.display()
        ),
    );
    write(
        &w.repo.join("ops/ansible/group_vars/hello-ally_edge/peering-deployment.yml"),
        "goat_zenoh_edge_core_endpoints:\n  - \"tls/core.zagros.example.org:7447#root_ca_certificate_file=/var/lib/goat-federation/hello-ally/pki/our-ca.pem\"\n",
    );
    write(
        &w.repo.join("ops/ansible/inventory/peering-edges.ini"),
        "[hello-ally_core]\ncore-1 ansible_host=192.0.2.20\n",
    );
    let mint = |w: &federate_world::World| {
        w.log()
            .lines()
            .find(|l| l.starts_with("mint "))
            .map(str::to_string)
            .unwrap_or_default()
    };

    let (out, _) = w.run(&["hello-ally", "--from", "8", "--to", "8"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(mint(&w), "mint san=core.zagros.example.org,192.0.2.20");

    w.clear_log();
    let out = w
        .command(&["hello-ally", "--from", "8", "--to", "8"])
        .env("FED_CORE_SAN", "198.51.100.7")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(mint(&w), "mint san=198.51.100.7");
}
