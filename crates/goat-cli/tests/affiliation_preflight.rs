use std::process::Command;

#[test]
fn malformed_catalog_is_refused_before_native_provisioning() {
    let temp = tempfile::tempdir().unwrap();
    let manifests = temp.path().join("ops/operators/zagros");
    let catalogs = temp.path().join("ops/identity/orgs");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(manifests.join("fixture.yml"), "identity: {handle: fixture, email: fixture@example.invalid}\norg: {short_tag: example-org}\nory_site: {enabled: true}\n").unwrap();
    std::fs::write(catalogs.join("zagros.json"), "malformed").unwrap();
    // The context carries the goatnet's facts, as `goat context refresh` writes them.
    let ctx = temp.path().join("contexts/default");
    std::fs::create_dir_all(&ctx).unwrap();
    std::fs::write(
        ctx.join("profile.toml"),
        "goatnet = \"zagros\"\nory_auth_url = \"https://auth.zagros.example\"\nory_ssh_host = \"192.0.2.10\"\nory_ssh_user = \"goat\"\n",
    )
    .unwrap();
    for dry_run in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_goat"));
        cmd.env_clear().env("GOAT_OPERATOR_HANDLE", "onboard-runner-fixture").env("GOAT_ORY_AFF_TRAIT_ENABLED", "true")
            .args(["--profile-dir", temp.path().to_str().unwrap(), "admin", "onboard", "fixture", "--native", "--env", "zagros", "--repo-root", temp.path().to_str().unwrap()]);
        if dry_run { cmd.arg("--dry-run"); }
        let out = cmd.output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success());
        assert!(err.contains("parse affiliation catalog"), "{err}");
    }
}
