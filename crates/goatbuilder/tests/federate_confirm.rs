//! Confirmation 1 (phase 5 keys, phase 6 agreement digest) by flag, through
//! the real binary against the stand-in repository (federate_world). Without
//! a terminal every value comes by flag; without them the run stops as it
//! always has.

mod federate_world;

use federate_world::{phases_run, public_b64, rows, says, sha256_hex, world};

fn their_key() -> String {
    public_b64("xab")
}

#[test]
fn the_keys_and_the_digest_by_flag_carry_the_run_to_phase_10() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    // Keys given, digest not yet compared: phases 1 to 6 run, then it stops.
    let (out, v) = w.run(&[
        "hello-ally",
        "--partner-key",
        &key,
        "--partner-ca-fingerprint",
        &fp,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5, 6], "{}", says(&out));
    assert!(
        rows(&v, "read-aloud")
            .iter()
            .any(|d| d.contains("our verifying key")),
        "{}",
        says(&out)
    );
    let ring = std::fs::read_to_string(w.work.join("keys.txt")).unwrap();
    assert!(
        ring.contains(&format!("xab={key}")) && ring.contains("xaa="),
        "{ring}"
    );
    assert!(
        w.log().contains(&format!("--key xab={key}")),
        "compose was planned again with the keys: {}",
        w.log()
    );
    let st = w.state();
    assert!(st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["kind"] == "keys" && c["how"] == "flag"));
    assert_eq!(st["phases"]["6"]["outcome"], "stopped");
    assert!(
        st["phases"]["6"]["detail"]
            .as_str()
            .unwrap()
            .starts_with("GB-192"),
        "{}",
        st["phases"]["6"]
    );

    // The digest, compared: compose again, confirmed, on to phase 10.
    let digest = sha256_hex(&std::fs::read(w.work.join("agreement.json")).unwrap());
    let (out, v) = w.run(&["hello-ally", "--resume", "--agreement-digest", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 4, 6, 7, 8, 9, 10], "{}", says(&out));
    assert!(w.state()["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["kind"] == "agreement" && c["bound_to"] == digest.as_str()));
}

#[test]
fn a_value_that_differs_stops_the_run_and_writes_nothing() {
    let w = world();
    let (out, _) = w.run(&[
        "hello-ally",
        "--partner-key",
        &their_key(),
        "--partner-ca-fingerprint",
        "sha256:00",
    ]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("GB-191"), "{text}");
    assert!(!w.work.join("keys.txt").exists());
    assert_eq!(w.state()["phases"]["5"]["outcome"], "failed");
}

#[test]
fn a_key_that_does_not_verify_their_half_stops_the_run_and_writes_nothing() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let (out, _) = w.run(&[
        "hello-ally",
        "--partner-key",
        &public_b64("xaa"),
        "--partner-ca-fingerprint",
        &fp,
    ]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("not signed by the key read aloud"), "{text}");
    assert!(!w.work.join("keys.txt").exists());
}

#[test]
fn a_corrected_key_on_resume_runs_phase_5_again() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    // A first run binds the keys and stops for the digest.
    w.run(&[
        "hello-ally",
        "--partner-key",
        &their_key(),
        "--partner-ca-fingerprint",
        &fp,
    ]);
    w.clear_log();
    let (out, v) = w.run(&[
        "hello-ally",
        "--resume",
        "--to",
        "5",
        "--partner-key",
        &their_key(),
        "--partner-ca-fingerprint",
        &fp,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        phases_run(&v).contains(&5),
        "given key values make phase 5 run: {}",
        says(&out)
    );
}

#[test]
fn values_for_a_phase_outside_the_run_are_refused() {
    let w = world();
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--partner-key", &their_key()]);
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
    let (out, _) = w.run(&["hello-ally", "--to", "5", "--agreement-digest", "sha256:00"]);
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
}

#[test]
fn one_value_without_the_other_stops_at_phase_5_and_names_the_missing_flag() {
    let w = world();
    let (out, v) = w.run(&["hello-ally", "--partner-key", &their_key()]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5]);
    assert!(
        rows(&v, "stopped")
            .iter()
            .any(|d| d.contains("--partner-ca-fingerprint")),
        "{}",
        says(&out)
    );
    assert!(!w.work.join("keys.txt").exists());
}

#[test]
fn confirmation_flags_do_not_belong_to_deploy_or_test() {
    let w = world();
    let (out, _) = w.run(&["test", "hello-ally", "--confirm"]);
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
}
