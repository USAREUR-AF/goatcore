// SPDX-License-Identifier: Apache-2.0
//
// The meter's own tests: fact assembly, the rollup's grading, the intra-goatnet
// skew band, and the fail-closed cases. Everything here is about what the
// surface says when something is MISSING, because that is where a health
// surface either tells the truth or manufactures a false green.

use super::*;

const CURRENT: &str = "sha256:b8524860d3e52515114189f866711d746de6898913bdd90bd75348339d17409c";
const PRIOR: &str = "sha256:0303030303030303030303030303030303030303030303030303030303030303";
const NOWHERE: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn testdata_generation(stem: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .join("testdata/generations")
        .join(format!("{stem}.intent.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("corpus fixture")).unwrap()
}

/// Whether this checkout carries its signed intent artifacts. A public clone of
/// goatcore has none; the tests that read them return early there.
fn has_committed_intent() -> bool {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).unwrap().join("intent").is_dir()
}

fn committed_intent(goatnet: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .join("intent")
        .join(goatnet)
        .join("intent.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("committed artifact")).unwrap()
}

/// Watched paths the committed artifact declares for a member group — the same
/// sum the projection makes (every governed component's `render_watch`, minus
/// groups declared `absent`), so growth in the watch set cannot split these
/// tests from the artifact the way pinned fleet sizes did.
fn declared_watches(intent: &Value, group: &str) -> usize {
    intent["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| {
            c.get("members")
                .and_then(|m| m.get(group))
                .and_then(|e| e.get("state"))
                .and_then(|s| s.as_str())
                != Some("absent")
        })
        .filter_map(|c| c.get("render_watch").and_then(|v| v.as_array()).map(|a| a.len()))
        .sum()
}

/// 2026-08-04T00:00:00Z — the same read-time instant the corpus uses, so ages
/// in these assertions match the ones a reviewer already read there.
const NOW_MS: i64 = 1_785_801_600_000;

fn node(host: &str, member: &str, hash: &str) -> ConsistencyReportV1 {
    ConsistencyReportV1 {
        host_id: host.to_string(),
        member_group: member.to_string(),
        intent_content_hash: hash.to_string(),
        intent_verified: !hash.is_empty(),
        collected_at_unix_ms: NOW_MS,
        ..Default::default()
    }
}

fn detail(env: &EnvHealthV1) -> &ConsistencyDetailV1 {
    match &env.detail {
        Some(EnvDetail::Consistency(d)) => d,
        _ => panic!("expected a consistency arm"),
    }
}

fn roll(intent: Option<&Value>, note: &str, reports: &[ConsistencyReportV1]) -> EnvHealthV1 {
    let g = Grammar::load().unwrap();
    roll_up(intent, note, reports, "zagros-sandbox", 30, NOW_MS, 900_000, &g).unwrap()
}

// -- fact assembly ----------------------------------------------------------

#[test]
fn the_record_carries_facts_and_no_verdict() {
    let snap: Snapshot = serde_json::from_str(
        r#"{"host_id":"zagros-a","goatnet":"zagros","member_group":"zagros_hosts",
            "intent_content_hash":"sha256:aa","intent_serial":3,
            "intent_generated_at":"2026-08-01T00:00:00Z","intent_commit":"deadbeef",
            "intent_verified":true,"applied_at_unix_ms":1700000000000,
            "collected_at_unix_ms":1700000001000,"collector_note":"",
            "components":[{"component":"goat-zenoh-portal","artifact_type":"image",
                           "ref":"ghcr.io/x/p@sha256:cc","prior_ref":"","ts_unix_ms":5,
                           "reversible":true,"reconciled":"confirmed","live_ref":"ghcr.io/x/p@sha256:cc"}],
            "renders":[]}"#,
    )
    .expect("snapshot parses");
    let r = report_from(&snap, "fallback");
    assert_eq!(r.host_id, "zagros-a");
    assert_eq!(r.intent_serial, 3);
    assert_eq!(r.intent_generated_at_unix_ms, 1_785_542_400_000);
    assert_eq!(r.components.len(), 1);
    assert_eq!(r.components[0].reconciled, "confirmed");

    // Round-trips through protobuf — the wire tags match the canonical schema.
    let back = ConsistencyReportV1::decode(r.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(back.intent_commit, "deadbeef");
    assert_eq!(back.components[0].r#ref, "ghcr.io/x/p@sha256:cc");
}

#[test]
fn a_missing_snapshot_publishes_an_empty_record_that_still_identifies_the_node() {
    // A node that stops publishing vanishes from the denominator; one that
    // publishes an empty record stays visible and grades indeterminate, which
    // is what ADR 1109 P15 requires.
    let r = unreadable_report("zagros-a", "snapshot missing");
    assert_eq!(r.host_id, "zagros-a");
    assert!(r.components.is_empty());
    assert!(!r.collector_note.is_empty(), "an empty record must say WHY it is empty");

    let env = roll(Some(&testdata_generation("accountable-tail")), "", &[r]);
    let d = detail(&env);
    assert_eq!(d.nodes_reporting, 1, "it is still counted as reporting");
    assert_eq!(d.nodes[0].grade, "indeterminate");
    assert_eq!(d.nodes[0].color, "idle");
    assert_eq!(env.overall_color, "idle", "ungrounded is never green");
}

// -- the generation grades --------------------------------------------------

#[test]
fn a_node_on_the_current_generation_is_green() {
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", CURRENT)],
    );
    assert_eq!(env.overall_color, "green");
    let d = detail(&env);
    assert_eq!(d.nodes[0].grade, "match");
    assert_eq!(d.nodes[0].behind_by, 0);
    // Age is REPORTED on a matching node and grades nothing: a goatnet whose
    // config has not changed in 90 days is correct, not stale (§5.3).
    assert_eq!(d.nodes[0].intent_age_seconds, 259_200);
    assert!(d.nodes[0].rows[0].remediation.is_empty(), "match earns no pointer");
}

#[test]
fn a_node_in_the_history_tail_is_amber_with_a_reconverge_pointer() {
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", PRIOR)],
    );
    assert_eq!(env.overall_color, "amber");
    let row = &detail(&env).nodes[0].rows[0];
    assert_eq!(row.grade, "SKEW");
    assert_eq!(row.remediation_kind, "reconverge");
    assert!(!row.remediation_refuses);
    assert!(
        row.remediation.contains("refresh.yml"),
        "the ordinary lagging node is pointed at its refresh play, verbatim: {}",
        row.remediation
    );
    assert_eq!(detail(&env).nodes[0].behind_by, 1);
}

#[test]
fn a_hash_no_generation_explains_is_red_and_never_amber() {
    // The F-272 / F-258 shape. Re-running the play would erase the evidence, so
    // this must NOT grade SKEW and must NOT be pointed at a refresh play.
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", NOWHERE)],
    );
    assert_eq!(env.overall_color, "red");
    let row = &detail(&env).nodes[0].rows[0];
    assert_eq!(row.grade, "unaccountable");
    assert!(
        !row.remediation.contains("refresh.yml"),
        "an unexplained node must never be pointed at a re-run: {}",
        row.remediation
    );
}

#[test]
fn no_applied_intent_stamp_is_indeterminate_never_unaccountable() {
    // Every node in the fleet is in this state until a stamping checkout
    // deploys it. Grading it unaccountable would point the whole fleet at
    // rollback.sh on day one.
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", "")],
    );
    let row = &detail(&env).nodes[0].rows[0];
    assert_eq!(row.grade, "indeterminate");
    assert_ne!(row.grade, "unaccountable");
    assert_eq!(env.overall_color, "idle");
}

#[test]
fn a_failed_integrity_check_is_unreadable_and_can_never_reach_match() {
    // P14 fail-closed. Note the hash is the CURRENT one — values agreeing
    // perfectly must not rescue an intent whose integrity did not verify.
    let mut n = node("zagros-a", "zagros_hosts", CURRENT);
    n.intent_verified = false;
    let env = roll(Some(&testdata_generation("accountable-tail")), "", &[n]);
    assert_eq!(detail(&env).nodes[0].rows[0].grade, "unreadable");
    assert_eq!(env.overall_color, "idle");
}

// -- fail-closed on intent itself ------------------------------------------

#[test]
fn an_unreadable_intent_refuses_to_grade_rather_than_grading_green() {
    let env = roll(None, "cannot read /intent/intent.json", &[node("zagros-a", "zagros_hosts", CURRENT)]);
    let d = detail(&env);
    assert!(!d.intent_readable);
    assert!(!d.intent_note.is_empty(), "the refusal must say why");
    assert!(d.nodes.is_empty(), "no node is graded against an intent nobody can trust");
    assert_eq!(env.overall_color, "idle");
    assert_eq!(d.nodes_reporting, 1, "the nodes are still counted — they did report");
}

/// The repo layout relative to this crate: ops/consistency and the committed
/// SPKI trust root at ops/enrollment/public-keys.
fn consistency_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

/// The CF-5 refusal, red-then-reverted, through the meter's own read path
/// (§9's gate in test form): the committed signed artifact verifies; a
/// tampered body refuses with an honest message; a stripped signature
/// refuses; and each revert goes green again. The verification key comes
/// from the committed public-keys set — never from the artifact.
#[test]
fn a_tampered_or_unsigned_artifact_is_refused_and_reverting_is_green() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    let dir = std::env::temp_dir().join(format!("gch-tamper-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("intent.json");
    let pubkeys = consistency_dir().join("../enrollment/public-keys");

    let committed = consistency_dir().join("intent/zagros/intent.json");
    std::fs::copy(&committed, &path).unwrap();
    std::fs::copy(
        committed.with_file_name("intent.json.sig"),
        path.with_file_name("intent.json.sig"),
    )
    .unwrap();
    assert!(
        load_intent(&path, &pubkeys).0.is_some(),
        "the committed, signed artifact verifies through the meter's read path"
    );

    // RED 1 — tamper the body after signing. The signature layer refuses
    // before any grading sees the bytes.
    let good_bytes = std::fs::read(&path).unwrap();
    let tampered = String::from_utf8(good_bytes.clone())
        .unwrap()
        .replace("\"goatnet\": \"zagros\"", "\"goatnet\": \"somebody-elses\"");
    std::fs::write(&path, tampered).unwrap();
    let (doc, note) = load_intent(&path, &pubkeys);
    assert!(doc.is_none(), "a tampered body is refused, never graded against");
    assert!(note.contains("signature"), "{note}");

    // REVERT — the same artifact is trusted again.
    std::fs::write(&path, &good_bytes).unwrap();
    assert!(load_intent(&path, &pubkeys).0.is_some(), "reverting the tamper restores green");

    // RED 2 — strip the signature. An unsigned intent is refused outright.
    std::fs::remove_file(path.with_file_name("intent.json.sig")).unwrap();
    let (doc, note) = load_intent(&path, &pubkeys);
    assert!(doc.is_none(), "an unsigned artifact is refused");
    assert!(note.contains("signature"), "{note}");

    std::fs::remove_dir_all(&dir).ok();
}

/// The content-hash layer still guards BEHIND the signature: a body that is
/// validly signed (here by a scratch CA) but whose content_hash is not the
/// hash of its own body is refused at the self-consistency layer. Layer
/// order is the point — signature proves WHO, the hash proves the body's own
/// claim, and passing one never waives the other.
#[test]
fn a_signed_but_self_inconsistent_artifact_still_fails_its_integrity_check() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    use base64::Engine as _;
    use p256::ecdsa::signature::Signer as _;
    use p256::pkcs8::EncodePublicKey as _;

    let dir = std::env::temp_dir().join(format!("gch-hash-{}", std::process::id()));
    let pubkeys = dir.join("public-keys");
    std::fs::create_dir_all(&pubkeys).unwrap();
    let path = dir.join("intent.json");

    let mut scalar = [0u8; 32];
    scalar[31] = 9;
    let key = p256::ecdsa::SigningKey::from_slice(&scalar).unwrap();
    let spki = key.verifying_key().to_public_key_der().unwrap().as_bytes().to_vec();
    let b64 = base64::engine::general_purpose::STANDARD.encode(spki);
    std::fs::write(
        pubkeys.join("gch-test-ca.pem"),
        format!("-----BEGIN PUBLIC KEY-----\nCA-ID: gch-test-ca\n\n{b64}\n-----END PUBLIC KEY-----\n"),
    )
    .unwrap();

    let mut bad = committed_intent("zagros");
    bad["ca_id"] = Value::String("gch-test-ca".into());
    // body changed (ca_id), content_hash left as committed => self-inconsistent
    let bytes = serde_json::to_string_pretty(&bad).unwrap() + "\n";
    std::fs::write(&path, &bytes).unwrap();
    let sig: p256::ecdsa::Signature = key.sign(bytes.as_bytes());
    let sig_b64 =
        base64::engine::general_purpose::STANDARD.encode(sig.to_der().as_bytes()) + "\n";
    std::fs::write(path.with_file_name("intent.json.sig"), sig_b64).unwrap();

    let (doc, note) = load_intent(&path, &pubkeys);
    assert!(doc.is_none(), "a validly-signed but self-inconsistent body is refused");
    assert!(note.contains("integrity check"), "{note}");

    std::fs::remove_dir_all(&dir).ok();
}

// -- the intra-goatnet skew band -------------------------------------------

#[test]
fn agreeing_nodes_have_a_zero_wide_band_even_when_all_are_behind() {
    // "They agree" and "they are right" are different findings. Two nodes on
    // the same superseded generation are a zero-wide band AND amber.
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", PRIOR), node("zagros-b", "zagros_b_hosts", PRIOR)],
    );
    let d = detail(&env);
    assert_eq!(d.skew_band_generations, 0, "they agree with each other");
    assert_eq!(d.skew_band_seconds, 0);
    assert_eq!(env.overall_color, "amber", "and they are both behind the artifact");
}

#[test]
fn a_split_goatnet_opens_the_band() {
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", CURRENT), node("zagros-b", "zagros_b_hosts", PRIOR)],
    );
    let d = detail(&env);
    assert_eq!(d.skew_band_generations, 1);
    assert_eq!(d.skew_band_seconds, 864_000 - 259_200);
    assert_eq!(env.overall_color, "amber");
}

#[test]
fn a_single_node_has_no_band_to_report() {
    // A spread needs two points. Reporting 0 for one node is right; inferring a
    // band from one reading would be inventing precision.
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", PRIOR)],
    );
    assert_eq!(detail(&env).skew_band_generations, 0);
}

#[test]
fn an_unplaceable_node_is_left_out_of_the_band_not_counted_as_agreeing() {
    // A node with no stamp cannot be placed on the generation line. Including
    // it as "agreeing" would narrow the band with a node nobody can locate.
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[
            node("zagros-a", "zagros_hosts", CURRENT),
            node("zagros-b", "zagros_b_hosts", PRIOR),
            node("zagros-c", "zagros_hosts", ""),
        ],
    );
    let d = detail(&env);
    assert_eq!(d.nodes_reporting, 3);
    assert_eq!(d.skew_band_generations, 1, "computed from the two placeable nodes");
    assert_eq!(env.overall_color, "amber");
}

// -- grounding --------------------------------------------------------------

#[test]
fn nodes_expected_is_the_denominator_and_comes_from_the_artifact() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    let intent = committed_intent("zagros");
    let env = roll(Some(&intent), "", &[node("zagros-a", "zagros_hosts", "")]);
    let d = detail(&env);
    // The denominator is whatever the committed artifact declares, not a
    // pinned literal — the fleet grows (2 broke to 4 with #397).
    let members = intent["members"].as_array().unwrap().len() as u32;
    assert_eq!(d.nodes_expected, members, "the artifact's member columns are the denominator");
    assert_eq!(d.nodes_reporting, 1);
    assert_eq!(d.goatnet, "zagros");
    assert_eq!(d.current_serial, intent["serial"].as_u64().unwrap());
}

#[test]
fn a_stale_record_drops_out_so_nodes_reporting_means_within_the_window() {
    let mut old = node("zagros-a", "zagros_hosts", CURRENT);
    old.collected_at_unix_ms = NOW_MS - 3_600_000; // an hour old, window is 15m
    let env = roll(Some(&testdata_generation("accountable-tail")), "", &[old]);
    let d = detail(&env);
    assert_eq!(d.nodes_reporting, 0);
    assert_eq!(env.overall_color, "unmetered", "nothing reported is unmetered, never green");
}

#[test]
fn nothing_reporting_at_all_is_unmetered() {
    let env = roll(Some(&testdata_generation("accountable-tail")), "", &[]);
    assert_eq!(env.overall_color, "unmetered");
    assert_eq!(detail(&env).nodes_reporting, 0);
}

#[test]
fn the_rolled_envelope_round_trips_and_declares_its_domain() {
    let env = roll(
        Some(&testdata_generation("accountable-tail")),
        "",
        &[node("zagros-a", "zagros_hosts", CURRENT)],
    );
    let back = EnvHealthV1::decode(env.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(back.domain, "consistency");
    assert_eq!(back.scope, "zagros-sandbox");
    assert_eq!(back.overall_color, "green");
    assert_eq!(detail(&back).nodes.len(), 1);
}

// -- the component join -----------------------------------------------------

#[test]
fn a_governed_component_joins_on_its_declared_ledger_key() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    let intent = committed_intent("zagros");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let expected_portal = intent["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "portal")
        .and_then(|c| c["members"]["zagros_hosts"]["value"].as_str())
        .expect("zagros_hosts pins the portal");

    let mut n = node("zagros-a", "zagros_hosts", &current);
    // The ledger key is `goat-zenoh-portal`, declared in fleet-pins.yaml — NOT
    // `portal`, which is what an inference from the component name would guess.
    n.components.push(ComponentStateV1 {
        component: "goat-zenoh-portal".into(),
        artifact_type: "image".into(),
        r#ref: expected_portal.replace("{{ ghcr_org }}", "some-other-org"),
        reconciled: "confirmed".into(),
        live_ref: "x".into(),
        ..Default::default()
    });
    let env = roll(Some(&intent), "", &[n]);
    let d = detail(&env);
    let row = d.nodes[0].rows.iter().find(|r| r.name == "portal").expect("a portal row");
    // The org differs and the digest does not: the digest is the identity,
    // because the same committed string resolves to a different org depending
    // on which checkout deploys (§3.1).
    assert_eq!(row.grade, "match", "why: {}", row.why);
    assert_eq!(d.nodes[0].components_observed, 1);
}

#[test]
fn ledger_coverage_gaps_stay_in_the_denominator() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    // postgres-spock and signal-server are declared `ledger_component: null` —
    // deployed from a playbook, not a ledger-writing role. They can never be
    // observed, and counting them is what keeps that gap visible rather than
    // silently missing.
    let intent = committed_intent("zagros");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    // The governed count is DERIVED from the committed artifact, never
    // hard-coded: a pinned literal here breaks the whole image build every
    // time a component joins the table (it did at the id-admin-console bump,
    // and the console's sibling assertion did the same at #2099) — a count
    // the artifact owns and this test does not.
    // ...and it must apply the same `absent` rule the projection applies:
    // components_governed (project.rs) skips a member declared `absent`,
    // because "this goatnet does not run that" is a declared answer and not a
    // coverage gap. Without this filter the denominator here would overcount.
    // The number must be derived by the SAME rule as the code under test.
    let governed = intent["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["members"]["zagros_hosts"].is_null())
        .filter(|c| c["members"]["zagros_hosts"]["state"].as_str() != Some("absent"))
        .count() as u32;
    let env = roll(Some(&intent), "", &[node("zagros-a", "zagros_hosts", &current)]);
    let n = &detail(&env).nodes[0];
    assert_eq!(n.components_governed, governed, "every zagros_hosts-governed component counts in the denominator");
    assert!(n.components_governed >= 6, "the table only grows for zagros_hosts (6 governed as of the id-admin-console bump)");
    assert_eq!(n.components_observed, 0, "this node reported no ledger tips");
    // And no SYNC verdict was fabricated for any of them. Since ADR 1124 each
    // unobserved governed component is a `coverage` ROW rather than a silent
    // subtraction from the numerator — the change that turns "6 of 6 missing"
    // from a number into six things with pointers.
    let cov: Vec<_> = n
        .rows
        .iter()
        .filter(|r| r.subject == "component" && r.coverage != "observed")
        .collect();
    assert_eq!(
        cov.len() as u32,
        governed,
        "every governed-but-unobserved component gets its own coverage row"
    );
    assert!(cov.iter().all(|r| r.grade == "indeterminate"), "nothing-to-read, never a match");
    assert!(
        cov.iter().all(|r| !r.remediation.is_empty() && !r.remediation_kind.is_empty()),
        "a coverage row without a pointer is the silence this replaced"
    );
    // The coverage rows do NOT poison the node's grade: coverage and sync are
    // orthogonal (ADR 1124 §A). What makes this node ungradable is that it
    // produced no comparable reading at all, which renders `unmetered` — the
    // absence of a reading, never the absence of a problem.
    assert_eq!(
        n.coverage_gaps as usize,
        n.rows.len() - 1,
        "every row but the generation row is a gap — the components plus the CF-4 render watch-set"
    );
    assert_ne!(n.color, "green");
    // The generation row is still first and is still a real comparison.
    assert_eq!(n.rows[0].subject, "generation");
    assert_eq!(n.rows[0].coverage, "observed");
    // The CF-4 render coverage row survives as its own subject.
    assert!(n.rows.iter().any(|r| r.subject == "render" && r.grade == "indeterminate"));
}

#[test]
fn an_unreconciled_tip_is_unreadable_not_a_converged_fact() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    let intent = committed_intent("zagros");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let expected_portal = intent["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "portal")
        .and_then(|c| c["members"]["zagros_hosts"]["value"].as_str())
        .unwrap()
        .to_string();

    let mut n = node("zagros-a", "zagros_hosts", &current);
    n.components.push(ComponentStateV1 {
        component: "goat-zenoh-portal".into(),
        artifact_type: "image".into(),
        r#ref: expected_portal,
        reconciled: "unreconciled".into(), // the node could not look
        ..Default::default()
    });
    let env = roll(Some(&intent), "", &[n]);
    let row = detail(&env).nodes[0]
        .rows
        .iter()
        .find(|r| r.name == "portal")
        .expect("a portal row");
    assert_eq!(row.grade, "unreadable", "recorded intent is not an observed fact");
    assert_eq!(
        env.overall_color, "idle",
        "and it drags the whole reading off green rather than being ignored"
    );
}

// -- the render watch (CF-4) ------------------------------------------------

/// The §9 CF-4 gate, run against the COMMITTED artifact: a hand-edited
/// rendered file grades `unaccountable` (red), its pointer is the reconverge
/// play — never rollback, which targets the prior generation for a file the
/// current one owns — and reverting the edit takes the same row back to green.
/// Red-then-reverted, in one test, over the real committed intent.
#[test]
fn a_hand_edited_render_is_red_reconverges_and_reverting_it_is_green() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    let intent = committed_intent("zagros");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    const RENDERED: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const EDITED: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    let render = |on_disk: &str| RenderStateV1 {
        component: "nova-zenoh".into(), // the declared ledger key, not "zenoh-router"
        path: "/opt/zagros-sandbox-zenoh/docker-compose.yml".into(),
        sha256: on_disk.into(),
        recorded_sha256: RENDERED.into(),
        recorded_at_unix_ms: NOW_MS - 60_000,
        note: String::new(),
    };

    // RED: the file on disk hashes to something no converge rendered.
    let mut edited = node("zagros-a", "zagros_hosts", &current);
    edited.renders.push(render(EDITED));
    let env = roll(Some(&intent), "", &[edited]);
    assert_eq!(env.overall_color, "red", "a hand-edit is the F-272 shape, not a shrug");
    let n = &detail(&env).nodes[0];
    assert_eq!(
        n.renders_declared as usize,
        declared_watches(&intent, "zagros_hosts"),
        "the declared count comes from the artifact, not a pinned literal"
    );
    assert_eq!(n.renders_observed, 1);
    let row = n
        .rows
        .iter()
        .find(|r| r.subject == "render" && r.coverage == "observed")
        .expect("the observed render row");
    assert_eq!(row.grade, "unaccountable");
    assert_eq!(row.remediation_kind, "reconverge");
    assert!(
        row.remediation.contains("refresh.yml"),
        "the undo of a mutated render is the idempotent re-render: {}",
        row.remediation
    );
    assert!(
        !row.remediation.contains("rollback.sh"),
        "rollback targets the PRIOR generation — a category error for a render: {}",
        row.remediation
    );
    assert_eq!(row.observed, EDITED);
    assert_eq!(row.expected, RENDERED);

    // REVERTED: the same watched file back at its converge-time bytes.
    let mut reverted = node("zagros-a", "zagros_hosts", &current);
    reverted.renders.push(render(RENDERED));
    let env = roll(Some(&intent), "", &[reverted]);
    let n = &detail(&env).nodes[0];
    let row = n
        .rows
        .iter()
        .find(|r| r.subject == "render" && r.coverage == "observed")
        .expect("the observed render row");
    assert_eq!(row.grade, "match", "the reverted file is back to its converge-time bytes");
    assert!(row.remediation.is_empty(), "match earns no pointer");
    // The SYNC verdict is clean — the edit is undone and every comparison this
    // node offered agrees.
    assert_eq!(n.grade, "match");
    // The COLOUR is idle, not green, and since ADR 1124 that is the honest
    // answer rather than a regression: this node reports tips for none of its
    // governed components, so it is not fully grounded. Before coverage rows
    // existed those gaps were a smaller numerator nobody read and the node
    // rendered green — the false-green shape F-324 is a case of.
    assert_eq!(env.overall_color, "idle");
    assert!(n.coverage_gaps > 0, "and the reason is a number, not a mood");
}

#[test]
fn a_declared_watch_set_with_no_manifest_is_idle_never_red() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    // The recorder is default-off, so EVERY node is in this state until a
    // recording converge runs. Same posture as a node with no applied-intent
    // stamp: nothing to read, never a finding — landing CF-4 must not paint
    // the fleet red on day one.
    let intent = committed_intent("zagros");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let env = roll(Some(&intent), "", &[node("zagros-a", "zagros_hosts", &current)]);
    let n = &detail(&env).nodes[0];
    let declared = declared_watches(&intent, "zagros_hosts");
    assert_eq!(
        n.renders_declared as usize,
        declared,
        "the declared count comes from the artifact, not a pinned literal"
    );
    assert_eq!(n.renders_observed, 0);
    let render_rows: Vec<_> = n.rows.iter().filter(|r| r.subject == "render").collect();
    assert_eq!(render_rows.len(), declared, "every declared watch gets its own coverage row");
    assert!(
        render_rows.iter().all(|r| r.grade == "indeterminate"),
        "nothing-to-read, never a match and never a finding"
    );
    assert_eq!(env.overall_color, "idle", "ungrounded is never green and never red");
}

#[test]
fn a_deleted_render_is_red_and_a_permissions_failure_is_idle() {
    if !has_committed_intent() {
        return; // the signed intent artifacts are this deployment's
    }
    // Deletion of a file the converge rendered is drift; a read failure is
    // ungrounded. The collector's `note` is what keeps those apart, and this
    // pins that the distinction survives the whole projection.
    let intent = committed_intent("zagros");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    const RENDERED: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let render = |note: &str| RenderStateV1 {
        component: "nova-zenoh".into(),
        path: "/opt/zagros-sandbox-zenoh/docker-compose.yml".into(),
        sha256: String::new(),
        recorded_sha256: RENDERED.into(),
        recorded_at_unix_ms: NOW_MS - 60_000,
        note: note.into(),
    };

    let mut gone = node("zagros-a", "zagros_hosts", &current);
    gone.renders.push(render("missing"));
    let env = roll(Some(&intent), "", &[gone]);
    assert_eq!(env.overall_color, "red", "a deleted render is unexplained absence");

    let mut unreadable = node("zagros-a", "zagros_hosts", &current);
    unreadable.renders.push(render("unreadable"));
    let env = roll(Some(&intent), "", &[unreadable]);
    assert_eq!(env.overall_color, "idle", "a failed read is ungrounded, not a verdict");
}

// ---------------------------------------------------------------------------
// Local live fire — a REAL Zenoh wire, no router and no container
// ---------------------------------------------------------------------------
//
// Two zenoh peer sessions over loopback TCP: one publishes an encoded
// ConsistencyReportV1 on ops/consistency/report/<host_id>, the other subscribes
// the real selector, decodes what actually arrived, and rolls it up.
//
// WHAT THIS PROVES: the topics, the protobuf encoding, the subscriber's
// key-derived host identity, and the whole publish -> receive -> grade -> roll
// path, end to end, over the transport that carries it in production.
//
// WHAT IT DOES NOT PROVE, stated so nobody reads it as more than it is: mTLS
// against a real router, the router's write ACL (a denied write is
// SILENT, so only a live subscriber on a real goatnet can show the ACL is
// right), the systemd timer, the container, or that any real node's ledger
// looks like the fixture. `goat sub --context <goatnet>
// 'ops/consistency/report/**' --take 2` is the live check.

async fn peer_pair() -> (zenoh::Session, zenoh::Session) {
    // Grab a free loopback port by binding and immediately dropping.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().unwrap().port()
    };
    let endpoint = format!("tcp/127.0.0.1:{port}");

    let mut listen_cfg = zenoh::Config::default();
    listen_cfg.insert_json5("mode", "\"peer\"").unwrap();
    listen_cfg
        .insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    listen_cfg
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let listener = zenoh::open(listen_cfg).await.expect("open listening peer");

    let mut connect_cfg = zenoh::Config::default();
    connect_cfg.insert_json5("mode", "\"peer\"").unwrap();
    connect_cfg
        .insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    connect_cfg
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let connector = zenoh::open(connect_cfg).await.expect("open connecting peer");

    (listener, connector)
}

#[tokio::test(flavor = "multi_thread")]
async fn live_fire_publish_receive_and_grade_over_a_real_zenoh_wire() {
    let (rollup_side, node_side) = peer_pair().await;

    let subscriber = rollup_side
        .declare_subscriber(REPORT_SELECTOR)
        .await
        .expect("declare subscriber");
    // Let the two peers finish establishing before the first put.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let intent = testdata_generation("accountable-tail");

    // The node publishes facts: it holds the PRIOR generation, i.e. it is one
    // behind. It does not say so — it does not know.
    let mut report = node("zagros-a", "zagros_hosts", PRIOR);
    report.intent_serial = 3;
    report.components.push(ComponentStateV1 {
        component: "goat-zenoh-portal".into(),
        artifact_type: "image".into(),
        r#ref: "ghcr.io/example/goat-zenoh-portal@sha256:1111".into(),
        reconciled: "confirmed".into(),
        live_ref: "ghcr.io/example/goat-zenoh-portal@sha256:1111".into(),
        ..Default::default()
    });
    node_side
        .put("ops/consistency/report/zagros-a", report.encode_to_vec())
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await
        .expect("put the report");

    // Receive what actually crossed the wire.
    let sample = tokio::time::timeout(Duration::from_secs(10), subscriber.recv_async())
        .await
        .expect("a report should arrive within 10s")
        .expect("subscriber recv");
    assert_eq!(sample.key_expr().as_str(), "ops/consistency/report/zagros-a");

    let received = wire::decode_report(sample.payload().to_bytes().as_ref())
        .expect("the payload decodes as a ConsistencyReportV1");
    assert_eq!(received.intent_content_hash, PRIOR);
    assert_eq!(received.components.len(), 1);

    // Identity comes from the KEY, not the payload — the key is what a router's
    // write ACL authorises, so a record claiming to be someone else is graded
    // as the host that was allowed to publish it.
    let mut owned = received.clone();
    owned.host_id = sample
        .key_expr()
        .as_str()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(owned.host_id, "zagros-a");

    let g = Grammar::load().unwrap();
    let env = roll_up(
        Some(&intent), "", &[owned], "zagros-sandbox", 30, NOW_MS, 900_000, &g,
    )
    .expect("roll up");

    // The reading a `goat health --domain consistency` would render.
    assert_eq!(env.domain, "consistency");
    assert_eq!(env.overall_color, "amber");
    let d = detail(&env);
    assert_eq!(d.nodes_reporting, 1);
    assert_eq!(d.nodes[0].host_id, "zagros-a");
    assert_eq!(d.nodes[0].grade, "SKEW");
    assert_eq!(d.nodes[0].behind_by, 1);
    let row = &d.nodes[0].rows[0];
    assert!(
        row.remediation.contains("refresh.yml") && row.remediation.contains("zagros-a"),
        "the operator is handed a copy-pasteable re-convergence: {}",
        row.remediation
    );

    // And the rolled envelope survives its own encode/decode, which is what the
    // CLI on the other end will do.
    let back = EnvHealthV1::decode(env.encode_to_vec().as_slice()).expect("decode envelope");
    assert_eq!(back.overall_color, "amber");
    assert_eq!(detail(&back).nodes[0].behind_by, 1);

    let _ = node_side.close().await;
    let _ = rollup_side.close().await;
}
