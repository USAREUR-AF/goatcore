use std::process::Command;

#[test]
fn revoked_manifest_refuses_both_reissues_before_network_even_in_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("ops/operators/zagros");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fixture.yml"), "identity: {handle: fixture, status: revoked}\nnetbird: {enabled: true, handset_limit: 1}\n").unwrap();
    // The context carries the goatnet's facts, as `goat context refresh` writes them.
    let ctx = temp.path().join("contexts/default");
    std::fs::create_dir_all(&ctx).unwrap();
    std::fs::write(
        ctx.join("profile.toml"),
        "goatnet = \"zagros\"\nnetbird_mgmt_url = \"https://netbird.zagros.example\"\nnetbird_token_env = \"ZAGROS_NB_API_TOKEN\"\n",
    )
    .unwrap();
    for flag in ["--reissue-device-key", "--reissue-handset-key"] {
        for dry_run in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_goat"));
            cmd.env_clear().env("GOAT_OPERATOR_HANDLE", "fixture")
                .args(["--profile-dir", temp.path().to_str().unwrap(), "admin", "onboard", "fixture", "--native", "--env", "zagros", "--repo-root", temp.path().to_str().unwrap(), flag]);
            if dry_run { cmd.arg("--dry-run"); }
            let out = cmd.output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success());
            assert!(err.contains("revoked"), "{flag}: {err}");
            assert!(!err.contains("token env var"), "guard ran too late: {err}");
        }
    }
}
