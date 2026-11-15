//! Confirmation 1 (phase 5 keys, phase 6 agreement digest) by flag, through
//! the real binary against the stand-in repository (federate_world). Without
//! a terminal every value comes by flag; without them the run stops as it
//! always has.

mod federate_world;

use federate_world::{phases_run, public_b64, resigned_with_transport, rows, says, sha256_hex, world};

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

/// ADR 1229: the partner rotated its WireGuard key and published a new
/// package under the key already bound. The directory step leaves it at
/// `theirs.incoming.json`; phase 5 stops and names its digest, and places it
/// only with `--accept-partner-package <that digest>`, keeping the old one
/// aside. Compose is then planned again with the new package.
#[test]
fn a_partners_new_package_is_placed_at_phase_5_only_with_its_digest() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    let theirs = w.work.join("theirs.canonical.json");
    let old = std::fs::read(&theirs).unwrap();
    let new = resigned_with_transport("xab", &fp, Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="));
    std::fs::write(w.work.join("theirs.incoming.json"), &new).unwrap();
    let digest = sha256_hex(&new);
    let keys = ["hello-ally", "--from", "5", "--to", "6", "--partner-key", &key, "--partner-ca-fingerprint", &fp];

    let (out, v) = w.run(&keys);
    assert!(out.status.success(), "a stop, not a failure: {}", says(&out));
    assert_eq!(phases_run(&v), [5], "{}", says(&out));
    assert!(says(&out).contains(&digest) && says(&out).contains("--accept-partner-package"), "{}", says(&out));
    assert_eq!(std::fs::read(&theirs).unwrap(), old, "placed without a confirmation");
    assert!(w.state()["phases"]["5"]["detail"].as_str().unwrap().starts_with("GB-192"));

    let mut with = keys.to_vec();
    with.extend(["--accept-partner-package", "sha256:00"]);
    let (out, _) = w.run(&with);
    assert!(says(&out).contains("GB-191"), "{}", says(&out));
    assert_eq!(std::fs::read(&theirs).unwrap(), old);

    w.clear_log();
    let mut with = keys.to_vec();
    with.extend(["--accept-partner-package", &digest]);
    let (out, v) = w.run(&with);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [5, 6], "{}", says(&out));
    assert_eq!(std::fs::read(&theirs).unwrap(), new);
    assert!(!w.work.join("theirs.incoming.json").exists());
    let kept: Vec<_> = std::fs::read_dir(&w.work)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("theirs.canonical.json.previous-"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read(w.work.join(&kept[0])).unwrap(), old);
    let st = w.state();
    let c = st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "partner-package")
        .expect("journaled");
    assert_eq!((c["how"].as_str(), c["bound_to"].as_str()), (Some("flag"), Some(digest.as_str())));
    assert!(w.log().contains("federate compose"), "compose ran again on the new package: {}", w.log());
}

#[test]
fn accept_partner_package_belongs_to_phase_5() {
    let w = world();
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--accept-partner-package", "sha256:00"]);
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
    let (out, _) = w.run(&["deploy", "hello-ally", "--accept-partner-package", "sha256:00"]);
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
}

/// ADR 1229, item 5: a package copied over the placed one by hand is refused
/// before compose even when the run resumes past phase 5, naming phase 5.
#[test]
fn a_resume_past_phase_5_refuses_a_package_that_is_not_the_one_confirmed() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    let other = resigned_with_transport("xab", &fp, Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="));
    std::fs::write(w.work.join("theirs.canonical.json"), &other).unwrap();
    w.clear_log();
    let (out, _) = w.run(&["hello-ally", "--from", "6"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(says(&out).contains("GB-191") && says(&out).contains("phase 5"), "{}", says(&out));
    assert!(!w.log().contains("federate compose"), "compose did not run: {}", w.log());
}

/// ADR 1229, item 3: a peering whose keys were confirmed by a build that did
/// not record the package has it recorded, as a migration, on its first run.
/// A held package that cannot be read stops the run with GB-310.
#[test]
fn a_peering_confirmed_before_1229_is_recorded_or_stopped_with_gb_310() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    // As a build before ADR 1229 journaled it: no package recorded.
    let path = w.work.join("state.json");
    let mut st = w.state();
    for c in st["confirmations"].as_array_mut().unwrap() {
        if let Some(v) = c["values"].as_object_mut() {
            v.remove("partner_package");
            v.remove("partner_package_issued");
        }
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&st).unwrap()).unwrap();
    let theirs = w.work.join("theirs.canonical.json");
    let held = std::fs::read(&theirs).unwrap();
    std::fs::rename(&theirs, w.work.join("held.json")).unwrap();
    let (out, _) = w.run(&["hello-ally", "--from", "6"]);
    assert_eq!(out.status.code(), Some(1), "{}", says(&out));
    assert!(says(&out).contains("GB-310"), "{}", says(&out));
    std::fs::rename(w.work.join("held.json"), &theirs).unwrap();
    let (out, _) = w.run(&["hello-ally", "--from", "6"]);
    assert!(!says(&out).contains("GB-310"), "{}", says(&out));
    let st = w.state();
    let m = st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["how"] == "migration")
        .expect("recorded as a migration");
    assert_eq!(m["bound_to"], sha256_hex(&held).as_str());
}

/// Lab finding (window 6): once confirmation 1 was given through the command,
/// a partner's new package confirmed by its digest carries the run on through
/// compose, without the keys being asked again; the keys are unchanged.
#[test]
fn a_new_package_by_its_digest_alone_carries_the_run_past_phase_5_when_the_keys_are_unchanged() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    let new = resigned_with_transport("xab", &fp, Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="));
    std::fs::write(w.work.join("theirs.incoming.json"), &new).unwrap();
    let digest = sha256_hex(&new);
    w.clear_log();
    let (out, v) = w.run(&["hello-ally", "--from", "5", "--to", "6", "--accept-partner-package", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(!says(&out).contains("waiting for the keys read aloud"), "{}", says(&out));
    assert_eq!(phases_run(&v), [5, 6], "{}", says(&out));
    assert_eq!(std::fs::read(w.work.join("theirs.canonical.json")).unwrap(), new);
    assert!(w.log().contains("federate compose"), "compose ran on the new package: {}", w.log());
}

/// Window 6's next stop: after a transport rotation the recomposed agreement is
/// the one both sides confirmed before (the agreement carries no WireGuard key),
/// so phase 6 does not ask its digest again, and the run goes on.
#[test]
fn an_unchanged_agreement_digest_confirmed_before_is_not_asked_again() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    let digest = sha256_hex(&std::fs::read(w.work.join("agreement.json")).unwrap());
    let (out, _) = w.run(&["hello-ally", "--resume", "--agreement-digest", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    let new = resigned_with_transport("xab", &fp, Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="));
    std::fs::write(w.work.join("theirs.incoming.json"), &new).unwrap();
    let pkg = sha256_hex(&new);
    let (out, v) = w.run(&["hello-ally", "--from", "5", "--to", "7", "--accept-partner-package", &pkg]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(!says(&out).contains("waiting for the agreement digest"), "{}", says(&out));
    assert_eq!(sha256_hex(&std::fs::read(w.work.join("agreement.json")).unwrap()), digest, "the agreement is unchanged");
    assert_eq!(phases_run(&v), [5, 6, 7], "{}", says(&out));
}

/// Owner's read, point 2: the package is checked before EACH phase after 5,
/// not once per run. Here phase 6's compose also copies another package over
/// the confirmed one; phase 7 must not run on it.
#[test]
fn the_package_is_checked_before_each_phase_after_5_not_once_per_run() {
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = their_key();
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    let digest = sha256_hex(&std::fs::read(w.work.join("agreement.json")).unwrap());
    let (out, _) = w.run(&["hello-ally", "--resume", "--agreement-digest", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    let goat = w.root.path().join("bin/goat");
    std::fs::rename(&goat, w.root.path().join("bin/goat.orig")).unwrap();
    let other = w.root.path().join("other-package.json");
    std::fs::write(&other, resigned_with_transport("xab", &fp, Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="))).unwrap();
    federate_world::exe(
        &goat,
        &format!(
            "#!/bin/sh\n[ \"$2\" = compose ] && cp '{}' '{}'\nexec '{}' \"$@\"\n",
            other.display(),
            w.work.join("theirs.canonical.json").display(),
            w.root.path().join("bin/goat.orig").display()
        ),
    );
    let (out, v) = w.run(&["hello-ally", "--from", "6", "--to", "8"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(says(&out).contains("GB-191") && says(&out).contains("phase 5"), "{}", says(&out));
    assert_eq!(phases_run(&v), [6], "{}", says(&out));
}
