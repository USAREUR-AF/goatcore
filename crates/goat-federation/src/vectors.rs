// The conformance vectors, run as tests.
//
// ops/federation/vectors/ is NORMATIVE. Every value there was derived by hand
// from the specification and no file was produced by running an
// implementation, which is the property that makes them a test of this code
// rather than a description of it. If this module and a vector disagree, this
// code is wrong.
//
// Three kinds, and the third is the one that goes wrong quietly:
//
//   composing pairs   canonical form, signing input, agreement bytes, digest
//   refusal pairs     refused, AND BY WHICH RULE
//   malformed/        single packages, refused BEFORE composition begins
//
// The last two assert the rule that fired, not merely that something failed.
// A fixture refused for the wrong reason is a passing test hiding a broken
// implementation — and reporting a malformed package as a composition refusal
// is exactly that: the same verdict from the wrong place.

use super::compose::{compose, digest_of, Composed, KeyRing};
use super::package::{decode_verifying_key, parse_package};
use std::path::{Path, PathBuf};

fn vectors() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors")
}

/// The evaluation date the vectors are pinned to. Run them at the wall clock
/// and `refuse-expired`, `compose-expiry-boundary` and every composing pair
/// change verdict as time passes.
fn pinned_now() -> chrono::NaiveDate {
    let s = std::fs::read_to_string(vectors().join("now.txt")).expect("now.txt");
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").expect("now.txt is a date")
}

/// The test keys, which are public, in a public repository, and worthless.
/// They exist so anyone can reproduce every signature in that directory.
fn testkeys() -> KeyRing {
    let mut ring = KeyRing::default();
    for sid in ["xaa", "xab"] {
        let b64 = std::fs::read_to_string(vectors().join(format!("testkeys/{sid}.public.b64")))
            .unwrap_or_else(|e| panic!("testkeys/{sid}.public.b64: {e}"));
        ring.insert(
            sid,
            decode_verifying_key(b64.trim()).expect("a 44-character raw key"),
        );
    }
    ring
}

/// Read a fixture file as BYTES. The byte-exact files carry no trailing
/// newline, because a canonical form is bytes and not a text file — read them
/// any other way and the whole suite tests something else.
fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The two packages in a fixture directory, in the order their filenames sort.
fn pair(dir: &Path) -> (PathBuf, PathBuf) {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let n = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            n.starts_with("package-")
                && n.ends_with(".canonical.json")
                && !n.contains("signing-input")
        })
        .collect();
    found.sort();
    assert_eq!(
        found.len(),
        2,
        "{} should hold exactly two packages",
        dir.display()
    );
    (found[0].clone(), found[1].clone())
}

const COMPOSING: [&str; 12] = [
    "compose-collation-utf16",
    "compose-crossed-extremes",
    "compose-expiry-boundary",
    "compose-full",
    "compose-listener-second-party",
    "compose-no-flows",
    "compose-no-transport",
    "compose-one-way",
    "compose-party-order-not-trust-domain",
    "compose-preference-and-tiebreak",
    "compose-transport-one-sided",
    "compose-unsorted-release",
];

/// Rule → fixture. Nine refusal fixtures over §6's seven rules; two rules
/// carry two fixtures each because in both cases an implementation can pass
/// one and fail the other.
const REFUSALS: [(&str, &str); 10] = [
    ("refuse-bad-signature", "C_SIGNATURE"),
    // A pair whose two `spec` values differ, caught by any version check…
    ("refuse-version-mismatch", "C_SPEC_PIN"),
    // …and a pair that AGREES WITH ITSELF on a version that is not this one,
    // which an implementation comparing the two packages with each other
    // composes happily. The pin is literal, not an equality test.
    ("refuse-both-v2", "C_SPEC_PIN"),
    ("refuse-same-sovereign-id", "C_SAME_SOVEREIGN_ID"),
    ("refuse-same-trust-domain", "C_SAME_TRUST_DOMAIN"),
    ("refuse-expired", "C_EXPIRED"),
    ("refuse-expired-first-party", "C_EXPIRED"),
    // Disjoint sets, which a raw intersection test catches…
    (
        "refuse-no-transport-overlap",
        "C_NO_TRANSPORT_IN_PREFERENCE",
    ),
    // …and a NON-EMPTY intersection outside §7's preference order, which it
    // does not: two parties both offering `quic` do intersect.
    (
        "refuse-transport-outside-preference",
        "C_NO_TRANSPORT_IN_PREFERENCE",
    ),
    ("refuse-neither-listener", "C_NEITHER_LISTENER"),
];

const MALFORMED: [(&str, &str); 10] = [
    ("duplicate-member.json", "M_DUPLICATE_MEMBER"),
    ("empty-number-token.json", "M_EMPTY_VALUE"),
    ("empty-release.json", "M_EMPTY_SET_ARRAY"),
    ("integer-above-range.json", "M_INT_RANGE"),
    ("leading-zero-number.json", "M_LEADING_ZERO"),
    ("negative-zero.json", "M_NEGATIVE_ZERO"),
    ("no-signature.json", "M_MISSING_MEMBER"),
    ("non-integer-number.json", "M_NON_INTEGER"),
    ("null-member.json", "M_NULL"),
    ("unpaired-surrogate.json", "M_UNPAIRED_SURROGATE"),
];

#[test]
fn every_composing_pair_reproduces_its_agreement_and_digest() {
    let now = pinned_now();
    let keys = testkeys();
    for name in COMPOSING {
        let dir = vectors().join(name);
        let (pa, pb) = pair(&dir);

        let mut parsed = Vec::new();
        for path in [&pa, &pb] {
            let raw = bytes(path);
            let pkg =
                parse_package(&raw).unwrap_or_else(|f| panic!("{name}/{}: {f}", path.display()));

            // The committed file IS the canonical form. An implementation whose
            // canonicaliser disagrees produces a different signing input and a
            // different digest, with no wrong value anywhere to find.
            assert_eq!(
                pkg.canonical().as_bytes(),
                raw.as_slice(),
                "{name}: {} is not reproduced byte-for-byte by the canonicaliser",
                path.display()
            );

            // The signature covers the canonical form with `signature` removed.
            let si_path = path.with_file_name(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .replace(".canonical.json", ".signing-input.canonical.json"),
            );
            assert_eq!(
                pkg.signing_input().as_bytes(),
                bytes(&si_path).as_slice(),
                "{name}: signing input for {} differs from the committed bytes",
                path.display()
            );
            parsed.push(pkg);
        }

        match compose(&parsed[0], &parsed[1], now, &keys) {
            Composed::Refused(f) => panic!("{name}: composed pair refused — {f}"),
            Composed::Agreement {
                canonical, digest, ..
            } => {
                assert_eq!(
                    canonical.as_bytes(),
                    bytes(&dir.join("agreement.canonical.json")).as_slice(),
                    "{name}: agreement bytes differ from the committed canonical form"
                );
                let want =
                    std::fs::read_to_string(dir.join("agreement.digest.txt")).expect("digest");
                assert_eq!(digest, want.trim(), "{name}: digest");
            }
        }
    }
}

/// Order does not matter: the agreement has no "us" and "them", so both sides
/// compute byte-identical output whichever way round the pair is handed in.
/// This is §2, and it is the whole idea — everything else is bookkeeping.
#[test]
fn composition_has_no_perspective() {
    let now = pinned_now();
    let keys = testkeys();
    for name in COMPOSING {
        let dir = vectors().join(name);
        let (pa, pb) = pair(&dir);
        let a = parse_package(&bytes(&pa)).expect("well-formed");
        let b = parse_package(&bytes(&pb)).expect("well-formed");
        let forward = compose(&a, &b, now, &keys);
        let backward = compose(&b, &a, now, &keys);
        match (forward, backward) {
            (Composed::Agreement { digest: f, .. }, Composed::Agreement { digest: r, .. }) => {
                assert_eq!(
                    f, r,
                    "{name}: composing the pair the other way round changed the digest"
                )
            }
            _ => panic!("{name}: expected an agreement both ways"),
        }
    }
}

/// One party offering a transport must produce exactly what neither offering
/// produces — a different input reaching the same bytes.
#[test]
fn one_sided_transport_matches_no_transport() {
    let a =
        std::fs::read_to_string(vectors().join("compose-transport-one-sided/agreement.digest.txt"))
            .unwrap();
    let b = std::fs::read_to_string(vectors().join("compose-no-transport/agreement.digest.txt"))
        .unwrap();
    assert_eq!(a.trim(), b.trim());
}

#[test]
fn every_refusal_fires_its_own_rule() {
    let now = pinned_now();
    let keys = testkeys();
    for (name, rule) in REFUSALS {
        let dir = vectors().join(name);
        let (pa, pb) = pair(&dir);
        // Both packages in a refusal fixture are structurally valid: only the
        // named rule may fire, and it fires at composition.
        let a =
            parse_package(&bytes(&pa)).unwrap_or_else(|f| panic!("{name}: {} — {f}", pa.display()));
        let b =
            parse_package(&bytes(&pb)).unwrap_or_else(|f| panic!("{name}: {} — {f}", pb.display()));
        match compose(&a, &b, now, &keys) {
            Composed::Agreement { digest, .. } => {
                panic!("{name}: composed to {digest}; it must refuse with {rule}")
            }
            Composed::Refused(f) => assert_eq!(
                f.rule, rule,
                "{name}: refused with {} — the right verdict from the wrong place is a passing \
                 test hiding a broken implementation. Detail: {}",
                f.rule, f.detail
            ),
        }
    }
}

/// A malformed package is refused by §3/§4 BEFORE composition begins, and is
/// not one of §6's seven refusals. Asserting the `M_` prefix is the point:
/// reporting one of these as a composition refusal is the defect
/// `malformed/README.md` calls out by name.
#[test]
fn every_malformed_package_is_refused_before_composition() {
    for (file, rule) in MALFORMED {
        let path = vectors().join("malformed").join(file);
        match parse_package(&bytes(&path)) {
            Ok(_) => panic!("{file}: accepted; it must be refused with {rule}"),
            Err(f) => {
                assert_eq!(
                    f.rule, rule,
                    "{file}: refused with {} — {}",
                    f.rule, f.detail
                );
                assert!(
                    f.rule.starts_with("M_"),
                    "{file}: refused with {}, which is a composition verdict on a pair rather \
                     than a property of one package",
                    f.rule
                );
            }
        }
    }
}

/// §9's third case, end to end: the same agreement, two digests, and the
/// answer is one measurement rather than a re-run of everything.
#[test]
fn identical_agreements_with_different_digests_name_their_cause() {
    use super::diff::{diff, ByteCause, DiffOutcome, Side};
    let base = bytes(&vectors().join("compose-full/agreement.canonical.json"));

    let mut newline = base.clone();
    newline.push(b'\n');
    match diff(&base, &newline, "ours", "theirs") {
        DiffOutcome::Bytes(b) => {
            assert_eq!(
                b.cause,
                ByteCause::TrailingNewline {
                    longer: Side::Right
                }
            );
            assert!(
                b.conclusion.contains("trailing newline"),
                "{}",
                b.conclusion
            );
        }
        other => panic!("expected a byte finding, got {other:?}"),
    }

    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(&base);
    match diff(&bom, &base, "ours", "theirs") {
        DiffOutcome::Bytes(b) => {
            assert_eq!(b.cause, ByteCause::ByteOrderMark { longer: Side::Left });
            assert!(b.conclusion.contains("byte-order mark"), "{}", b.conclusion);
        }
        other => panic!("expected a byte finding, got {other:?}"),
    }

    // The measured numbers §9 quotes, so the vectors' own example is pinned
    // here too rather than only in prose.
    assert_eq!(
        digest_of(&base),
        "sha256:b29bb23071a5c668d0d0f2a3aec0a1c85337ed8a9d4cf94feae1f052d34410df"
    );
    assert_eq!(
        digest_of(&newline),
        "sha256:27f0dcf0f79994f7e4e463151a302b058458bb803fe83661412b48cc1e63d5ad"
    );
}

/// An indented rendering hashed by mistake is a real operator error and is NOT
/// one of §9's three cases — so it must not be reported as one.
#[test]
fn a_pretty_file_hashed_by_mistake_is_named_as_such() {
    use super::diff::{diff, ByteCause, DiffOutcome};
    let canonical = bytes(&vectors().join("compose-full/agreement.canonical.json"));
    let pretty = bytes(&vectors().join("compose-full/agreement.pretty.json"));
    match diff(&canonical, &pretty, "ours", "theirs") {
        DiffOutcome::Bytes(b) => {
            assert!(
                matches!(b.cause, ByteCause::NotCanonical { .. }),
                "{:?}",
                b.cause
            );
            assert!(
                b.conclusion.contains("not the canonical form"),
                "{}",
                b.conclusion
            );
        }
        other => panic!("expected a byte finding, got {other:?}"),
    }
}

/// A genuine disagreement names the field. This is what an operator reaches
/// for at the moment two digests differ.
#[test]
fn a_real_disagreement_names_the_field() {
    use super::diff::{diff, DiffOutcome};
    let full = bytes(&vectors().join("compose-full/agreement.canonical.json"));

    // One field apart: `compose-crossed-extremes` puts the earlier expiry on
    // parties[0], and nothing else in that agreement moves.
    let crossed = bytes(&vectors().join("compose-crossed-extremes/agreement.canonical.json"));
    match diff(&full, &crossed, "ours", "theirs") {
        DiffOutcome::Fields(diffs) => {
            let paths: Vec<&str> = diffs.iter().map(|d| d.path.as_str()).collect();
            assert_eq!(paths, vec!["expires"], "{paths:?}");
            assert!(diffs[0].left.contains("2026-11-20"), "{:?}", diffs[0]);
            assert!(diffs[0].right.contains("2026-10-01"), "{:?}", diffs[0]);
        }
        other => panic!("expected field differences, got {other:?}"),
    }

    // A structural difference: one side composed a flow the other did not, so
    // the path has to reach into the array rather than stopping at `flows`.
    let no_flows = bytes(&vectors().join("compose-no-flows/agreement.canonical.json"));
    match diff(&full, &no_flows, "ours", "theirs") {
        DiffOutcome::Fields(diffs) => {
            let paths: Vec<&str> = diffs.iter().map(|d| d.path.as_str()).collect();
            assert!(paths.iter().any(|p| p.starts_with("flows[")), "{paths:?}");
            assert!(
                diffs.iter().any(|d| d.right == "(absent)"),
                "a flow present on one side and absent on the other must say so: {diffs:?}"
            );
        }
        other => panic!("expected field differences, got {other:?}"),
    }
}

/// The one collation, on the one input where the languages disagree. Byte-wise
/// UTF-8 and code-point order are the same ordering and both differ from this
/// rule on a character above the basic multilingual plane.
#[test]
fn set_arrays_sort_by_utf16_code_unit() {
    use super::json::normalise_set;
    // U+10000 is a surrogate pair beginning D800, so UTF-16 places it BEFORE
    // U+E000–U+FFFF where a code-point comparison places it after.
    let astral = "release/\u{10000}".to_string();
    let bmp = "release/\u{ff00}".to_string();
    let sorted = normalise_set(&[bmp.clone(), astral.clone()]);
    assert_eq!(
        sorted,
        vec![astral, bmp],
        "UTF-16 order, not code-point order"
    );
}

/// Duplicates are removed and order is not inherited from the package.
#[test]
fn set_arrays_are_de_duplicated() {
    use super::json::normalise_set;
    let out = normalise_set(&["b".into(), "a".into(), "b".into()]);
    assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
}

/// Composition needs a key BOUND to the party. The key is never taken from the
/// package: a package carrying its own key authenticates nothing.
#[test]
fn composition_refuses_without_a_bound_key() {
    let now = pinned_now();
    let dir = vectors().join("compose-full");
    let (pa, pb) = pair(&dir);
    let a = parse_package(&bytes(&pa)).expect("well-formed");
    let b = parse_package(&bytes(&pb)).expect("well-formed");
    match compose(&a, &b, now, &KeyRing::default()) {
        Composed::Refused(f) => {
            assert_eq!(f.rule, "C_SIGNATURE");
            assert!(f.detail.contains("no verifying key"), "{}", f.detail);
        }
        Composed::Agreement { .. } => panic!("composed an unauthenticated pair"),
    }
}
