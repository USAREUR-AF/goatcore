// The package v2 conformance vectors, run as tests (ADR 1242).
//
// ops/federation/vectors-v2/ holds v2's vectors; ops/federation/vectors/ stays
// v1's, frozen, and is still run by `vectors.rs` through the v1 entry point.
// v2 here is v1 with one member added, `identity.ca_fingerprint_next`, and the
// agreement copying it into its party (`docs/design/federation-package-v2-spec.md`).
//
// Production composes through `compose_any`, which routes by the two packages'
// `spec`: a v1 pair takes the frozen v1 path unchanged, a v2 pair the v2 path,
// and any other pair is refused by name.

use super::compose::{compose, compose_any, Composed, KeyRing};
use super::package::{decode_verifying_key, parse_package, warnings};
use std::path::{Path, PathBuf};

fn root(tree: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/federation")
        .join(tree)
}

fn pinned_now() -> chrono::NaiveDate {
    let s = std::fs::read_to_string(root("vectors-v2").join("now.txt")).expect("now.txt");
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").expect("now.txt is a date")
}

fn testkeys() -> KeyRing {
    let mut ring = KeyRing::default();
    for sid in ["xaa", "xab"] {
        let b64 =
            std::fs::read_to_string(root("vectors").join(format!("testkeys/{sid}.public.b64")))
                .unwrap();
        ring.insert(sid, decode_verifying_key(b64.trim()).unwrap());
    }
    ring
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn pair(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    (
        bytes(&dir.join("package-xaa.canonical.json")),
        bytes(&dir.join("package-xab.canonical.json")),
    )
}

const COMPOSING: [&str; 3] = [
    "compose-no-next",
    "compose-next-one-side",
    "compose-next-both-sides",
];

const REFUSALS: [(&str, &str); 2] = [
    // One v1 package and one v2: never composed under either version's rules.
    ("refuse-mixed-versions", "C_SPEC_MIXED"),
    // A pair agreeing on a version this build does not implement.
    ("refuse-both-v3", "C_SPEC_PIN"),
];

const MALFORMED: [(&str, &str); 2] = [
    ("next-equals-current.json", "M_CA_NEXT_SAME"),
    ("next-not-a-fingerprint.json", "M_CA_FINGERPRINT"),
];

#[test]
fn every_v2_composing_pair_reproduces_its_agreement_and_digest() {
    let (now, keys) = (pinned_now(), testkeys());
    for name in COMPOSING {
        let dir = root("vectors-v2").join(name);
        let (a, b) = pair(&dir);
        let (pa, pb) = (parse_package(&a).unwrap(), parse_package(&b).unwrap());
        for (raw, p) in [(&a, &pa), (&b, &pb)] {
            assert_eq!(
                p.canonical().as_bytes(),
                raw.as_slice(),
                "{name}: canonical form"
            );
            let si = bytes(&dir.join(format!(
                "package-{}.signing-input.canonical.json",
                p.sovereign_id
            )));
            assert_eq!(
                p.signing_input().as_bytes(),
                si.as_slice(),
                "{name}: signing input"
            );
        }
        // Order of the two packages does not matter.
        for (p, q) in [(&pa, &pb), (&pb, &pa)] {
            match compose_any(p, q, now, &keys) {
                Composed::Agreement {
                    canonical, digest, ..
                } => {
                    assert_eq!(
                        canonical.as_bytes(),
                        bytes(&dir.join("agreement.canonical.json")).as_slice(),
                        "{name}"
                    );
                    assert_eq!(
                        digest,
                        String::from_utf8(bytes(&dir.join("agreement.digest.txt")))
                            .unwrap()
                            .trim(),
                        "{name}"
                    );
                }
                Composed::Refused(f) => panic!("{name}: refused — {f}"),
            }
        }
        // The frozen v1 entry point refuses every v2 pair, as before.
        match compose(&pa, &pb, now, &keys) {
            Composed::Refused(f) => assert_eq!(f.rule, "C_SPEC_PIN", "{name}: {f}"),
            Composed::Agreement { .. } => panic!("{name}: the v1 entry point composed a v2 pair"),
        }
    }
}

#[test]
fn every_v2_refusal_pair_is_refused_by_its_rule() {
    let (now, keys) = (pinned_now(), testkeys());
    for (name, rule) in REFUSALS {
        let (a, b) = pair(&root("vectors-v2").join(name));
        let (pa, pb) = (parse_package(&a).unwrap(), parse_package(&b).unwrap());
        for (p, q) in [(&pa, &pb), (&pb, &pa)] {
            match compose_any(p, q, now, &keys) {
                Composed::Refused(f) => assert_eq!(f.rule, rule, "{name}: {f}"),
                Composed::Agreement { .. } => panic!("{name}: composed"),
            }
        }
    }
}

#[test]
fn every_v2_malformed_package_is_refused_by_its_rule() {
    for (name, rule) in MALFORMED {
        let raw = bytes(&root("vectors-v2").join("malformed").join(name));
        match parse_package(&raw) {
            Err(f) => assert_eq!(f.rule, rule, "{name}: {f}"),
            Ok(_) => panic!("{name}: parsed"),
        }
    }
}

/// Every v1 vector composes or is refused through `compose_any` exactly as
/// through the frozen v1 entry point: routing a v1 pair changes nothing.
#[test]
fn a_v1_pair_routes_to_the_frozen_v1_path_unchanged() {
    let (now, keys) = (pinned_now(), testkeys());
    let mut seen = 0;
    for e in std::fs::read_dir(root("vectors")).unwrap() {
        let dir = e.unwrap().path();
        let n = dir.file_name().unwrap().to_string_lossy().into_owned();
        if !(n.starts_with("compose-") || n.starts_with("refuse-"))
            || n == "refuse-both-v2"
            || n == "refuse-version-mismatch"
        {
            continue;
        }
        let mut pk: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let f = p.file_name().unwrap().to_string_lossy();
                f.starts_with("package-")
                    && f.ends_with(".canonical.json")
                    && !f.contains("signing-input")
            })
            .collect();
        pk.sort();
        let (a, b) = (bytes(&pk[0]), bytes(&pk[1]));
        let (Ok(pa), Ok(pb)) = (parse_package(&a), parse_package(&b)) else {
            continue;
        };
        let one = format!("{:?}", compose(&pa, &pb, now, &keys));
        let any = format!("{:?}", compose_any(&pa, &pb, now, &keys));
        assert_eq!(one, any, "{n}");
        seen += 1;
    }
    assert!(seen >= 20, "{seen} v1 pairs");
}

/// A v1 package is read by v1's grammar: a `ca_fingerprint_next` it carries is
/// not v1's member and is not read as a next CA.
#[test]
fn a_v1_package_carrying_the_member_has_no_next_ca() {
    let raw = bytes(
        &root("vectors-v2")
            .join("compose-next-one-side")
            .join("package-xaa.pretty.json"),
    );
    let mut v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    v["spec"] = "federation-package/v1".into();
    let p = parse_package(serde_json::to_string(&v).unwrap().as_bytes()).unwrap();
    assert!(p.identity.ca_fingerprint_next.is_none());
    let p2 = parse_package(&bytes(
        &root("vectors-v2")
            .join("compose-next-one-side")
            .join("package-xaa.canonical.json"),
    ))
    .unwrap();
    assert!(p2.identity.ca_fingerprint_next.is_some());
}

/// lint names a version this build composes as no warning, and any other as one.
#[test]
fn lint_warns_only_for_a_version_this_build_does_not_compose() {
    let ok = bytes(
        &root("vectors-v2")
            .join("compose-next-one-side")
            .join("package-xaa.canonical.json"),
    );
    let w = warnings(&parse_package(&ok).unwrap(), &ok);
    assert!(!w.iter().any(|w| w.rule.starts_with("W_SPEC")), "{w:?}");
    let v3 = bytes(
        &root("vectors-v2")
            .join("refuse-both-v3")
            .join("package-xaa.canonical.json"),
    );
    let w = warnings(&parse_package(&v3).unwrap(), &v3);
    assert!(w.iter().any(|w| w.rule.starts_with("W_SPEC")), "{w:?}");
}

/// The version is inside the signed bytes in both versions: `spec` is a member
/// of the package, and the signing input is the canonical form without
/// `signature` alone. So verifying signatures before routing by `spec` is
/// sound: a package whose `spec` was changed after signing no longer verifies.
#[test]
fn the_spec_member_is_inside_the_signed_bytes_in_both_versions() {
    for (tree, dir, spec) in [
        ("vectors", "compose-full", "federation-package/v1"),
        (
            "vectors-v2",
            "compose-next-one-side",
            "federation-package/v2",
        ),
    ] {
        let p = parse_package(&bytes(
            &root(tree).join(dir).join("package-xaa.canonical.json"),
        ))
        .unwrap();
        assert!(
            p.signing_input().contains(&format!("\"spec\":\"{spec}\"")),
            "{tree}/{dir}"
        );
    }
}

/// A package relabelled to the other version after it was signed fails step
/// 0's signature check and is never composed, under either version: a v2
/// package relabelled v1 with its next CA kept or dropped, and a v1 package
/// relabelled v2. Its partner in each pair is a valid package of the version
/// claimed, so nothing but the relabelling can be refused.
#[test]
fn a_package_relabelled_to_the_other_version_fails_verification() {
    let (now, keys) = (pinned_now(), testkeys());
    for name in [
        "refuse-downgrade-next-kept",
        "refuse-downgrade-next-dropped",
        "refuse-relabel-v1-as-v2",
    ] {
        let (a, b) = pair(&root("vectors-v2").join(name));
        let (pa, pb) = (parse_package(&a).unwrap(), parse_package(&b).unwrap());
        for (p, q) in [(&pa, &pb), (&pb, &pa)] {
            for composed in [compose_any(p, q, now, &keys), compose(p, q, now, &keys)] {
                match composed {
                    Composed::Refused(f) => {
                        assert_eq!(f.rule, "C_SIGNATURE", "{name}: {f}");
                        assert!(
                            f.path.contains("xaa"),
                            "{name}: the relabelled package is xaa's: {f}"
                        );
                    }
                    Composed::Agreement { .. } => panic!("{name}: a relabelled package composed"),
                }
            }
        }
    }
}
