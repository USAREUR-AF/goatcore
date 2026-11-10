// SPDX-License-Identifier: Apache-2.0
//
// The console's own tests. Everything here is about what the PAGE says when
// something is missing, stale, unverified or live — because a projection
// either tells the truth at render time or it manufactures the false green
// the whole surface exists to prevent.
//
// The live fire at the bottom is the §9 CF-6 gate in its local form: a real
// rollup envelope, published between two zenoh peer sessions over loopback
// TCP (the CF-3 harness verbatim), received by the console's own ingest path
// and rendered into the fixture goatnet's grid. What it does NOT prove, stated so nobody
// reads it as more: mTLS against a real router, the router's ACL (a denied write is
// silent), the oauth2 fence, or any real node's ledger. The
// on-goatnet render awaits a deploy decision.

use super::*;
use goat_consistency::{project_goatnet, ObservedComponent, ObservedNode};
use goat_consistency_wire::{
    ConsistencyDetailV1, ConsistencyReportV1, ConsistencyRowV1, EnvDetail, EnvHealthV1,
    NodeConsistencyV1,
};
use prost::Message as _;
use serde_json::Value;
use std::path::Path;

/// 2026-08-04T00:00:00Z — the corpus's read-time instant.
const NOW_MS: i64 = 1_785_801_600_000;

fn consistency_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

fn testdata_generation(stem: &str) -> Value {
    let path = consistency_dir().join("testdata/generations").join(format!("{stem}.intent.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("corpus fixture")).unwrap()
}

fn grammar() -> Grammar {
    Grammar::load().unwrap()
}

fn observed(host: &str, member: &str, hash: &str) -> ObservedNode {
    ObservedNode {
        host_id: host.to_string(),
        member_group: member.to_string(),
        intent_content_hash: hash.to_string(),
        intent_serial: 0,
        intent_verified: !hash.is_empty(),
        report_age_ms: 0,
        components: Vec::new(),
        renders: Vec::new(),
    }
}

/// Build the envelope the rollup meter would publish for these nodes — the
/// same projection call, mapped onto the same wire structs. Test-only: in
/// production the console never builds an envelope, it only receives one.
fn envelope_from(intent: &Value, nodes: &[ObservedNode], site: &str, now_ms: i64) -> EnvHealthV1 {
    let g = grammar();
    let p = project_goatnet(Some(intent), nodes, now_ms / 1000, &g).expect("project");
    let nodes: Vec<NodeConsistencyV1> = p
        .nodes
        .iter()
        .map(|n| NodeConsistencyV1 {
            host_id: n.host_id.clone(),
            member_group: n.member_group.clone(),
            color: n.color.clone(),
            grade: n.grade.clone(),
            intent_serial: n.intent_serial,
            intent_content_hash: n.intent_content_hash.clone(),
            behind_by: n.behind_by,
            intent_age_seconds: n.intent_age_seconds,
            report_age_ms: n.report_age_ms,
            intent_verified: n.intent_verified,
            components_governed: n.components_governed as u32,
            components_observed: n.components_observed as u32,
            renders_declared: n.renders_declared as u32,
            renders_observed: n.renders_observed as u32,
            coverage_gaps: n.coverage_gaps as u32,
            rows: n
                .rows
                .iter()
                .map(|r| ConsistencyRowV1 {
                    subject: r.subject.clone(),
                    name: r.name.clone(),
                    artifact_type: r.artifact_type.clone(),
                    grade: r.grade.clone(),
                    color: r.color.clone(),
                    why: r.why.clone(),
                    observed: r.observed.clone(),
                    expected: r.expected.clone(),
                    remediation_kind: r.remediation.as_ref().map(|x| x.kind.clone()).unwrap_or_default(),
                    remediation: r.remediation.as_ref().map(|x| x.pointer.clone()).unwrap_or_default(),
                    remediation_refuses: r.remediation.as_ref().map(|x| x.refuses).unwrap_or(false),
                    coverage: r.coverage.clone(),
                })
                .collect(),
        })
        .collect();
    EnvHealthV1 {
        domain: "consistency".into(),
        scope: site.into(),
        ts_unix_ms: now_ms,
        overall_color: p.overall_color.clone(),
        sample_count: nodes.len() as u64,
        window_secs: 30,
        detail: Some(EnvDetail::Consistency(ConsistencyDetailV1 {
            unreported_members: p.unreported_members.clone(),
            nodes,
            goatnet: p.goatnet.clone(),
            current_serial: p.current_serial,
            current_content_hash: p.current_content_hash.clone(),
            nodes_expected: p.nodes_expected,
            nodes_reporting: p.nodes_reporting,
            skew_band_generations: p.skew_band_generations,
            skew_band_seconds: p.skew_band_seconds,
            intent_readable: true,
            intent_note: String::new(),
        })),
    }
}

struct Fixture {
    latest: Latest,
    intent: (Option<Value>, String),
    siblings: Siblings,
    grammar: Grammar,
}

impl Fixture {
    fn new() -> Self {
        Fixture {
            latest: Latest::default(),
            intent: (None, "cannot read /intent/intent.json: test fixture".into()),
            siblings: Siblings::default(),
            grammar: grammar(),
        }
    }

    fn render(&self, chosen: &str) -> String {
        render::render_page(&render::View {
            chosen_site: chosen,
            latest: &self.latest,
            intent: &self.intent,
            siblings: &self.siblings,
            grammar: &self.grammar,
            now_ms: NOW_MS,
            stale_after_ms: 900_000,
            version: "test",
        })
    }
}

/// A committed, signed artifact through the REAL verify-at-read path — the
/// same call the request handler makes, for any governed goatnet.
fn verified_committed_intent_for(goatnet: &str) -> (Option<Value>, String) {
    let path = consistency_dir().join(format!("intent/{goatnet}/intent.json"));
    let pubkeys = consistency_dir().join("../enrollment/public-keys");
    let out = goat_consistency::verify::load_verified(&path, &pubkeys);
    assert!(out.0.is_some(), "the committed {goatnet} artifact must verify: {}", out.1);
    out
}

fn verified_committed_intent() -> (Option<Value>, String) {
    verified_committed_intent_for("zagros")
}

// ---------------------------------------------------------------------------
// Honesty: absence, staleness, refusal
// ---------------------------------------------------------------------------

#[test]
fn an_absent_rollup_renders_nothing_heard_never_healthy() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    let fx = Fixture::new();
    let page = fx.render("zagros-sandbox");
    assert!(page.contains("nothing heard"), "absence is an answer");
    assert!(page.contains("chip-unmetered"), "absence of signal is unmetered");
    assert!(page.contains("silently denied"), "the silent-denial hypothesis is named");
    assert!(
        !page.contains("chip chip-green"),
        "an empty console must not render a green chip anywhere (the stylesheet may define one)"
    );
}

#[test]
fn a_stale_rollup_reads_as_history_not_as_the_current_state() {
    let mut fx = Fixture::new();
    let intent = testdata_generation("accountable-tail");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &current)], "zagros-sandbox", NOW_MS - 3_600_000);
    fx.latest.rollups.insert(
        "zagros-sandbox".into(),
        StoredEnvelope { env, received_unix_ms: NOW_MS - 3_600_000 },
    );
    let page = fx.render("zagros-sandbox");
    assert!(page.contains("stale — last heard"), "an hour-old reading says its age");
    assert!(
        page.contains("shown as\nhistory, not as the current state")
            || page.contains("history, not as the current state"),
        "staleness is stated, not implied"
    );
}

#[test]
fn an_unverified_intent_refuses_on_the_page_and_renders_no_repo_grid() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    // Tamper the committed artifact in a scratch dir and read it through the
    // console's own read path — the refusal must be VERBATIM on the page.
    let dir = std::env::temp_dir().join(format!("gcc-refusal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("intent.json");
    let committed = consistency_dir().join("intent/zagros/intent.json");
    let pubkeys = consistency_dir().join("../enrollment/public-keys");
    let tampered = std::fs::read_to_string(&committed)
        .unwrap()
        .replace("\"goatnet\": \"zagros\"", "\"goatnet\": \"somebody-elses\"");
    std::fs::write(&path, tampered).unwrap();
    std::fs::copy(
        committed.with_file_name("intent.json.sig"),
        path.with_file_name("intent.json.sig"),
    )
    .unwrap();

    let mut fx = Fixture::new();
    fx.intent = goat_consistency::verify::load_verified(&path, &pubkeys);
    assert!(fx.intent.0.is_none(), "the tamper must refuse");

    let page = fx.render("zagros-sandbox");
    assert!(page.contains("committed intent — REFUSED"));
    assert!(
        page.contains("FAILED signature verification"),
        "the refusal reason renders verbatim"
    );
    assert!(
        !page.contains("committed-intent grid <"),
        "no repo-vantage grid is rendered from an unverified artifact"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_verified_committed_artifact_renders_the_repo_grid() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    let mut fx = Fixture::new();
    fx.intent = verified_committed_intent();
    let page = fx.render("zagros-sandbox");
    assert!(page.contains("signature verified"));
    assert!(page.contains("committed-intent grid"));
    // The fixture goatnet's real components and members appear as the grid's rows/columns.
    assert!(page.contains("portal"), "a governed component row renders");
    assert!(page.contains("zagros_hosts") && page.contains("zagros_b_hosts"));
    // The declared secondary-member portal absence is a declared answer, not a gap —
    // the cell says the state in one dimmed word (the chip and the row's own
    // disclosure carry the declaredness; drift-page-grid vocabulary dedup).
    assert!(page.contains(r#"<span class="gval mono"><span class="dim">absent</span></span>"#));
    assert!(!page.contains("absent (declared)"), "no wrapping parenthetical in a cell");
}

// ---------------------------------------------------------------------------
// Remediation: verbatim, copyable, refusals rendered as facts
// ---------------------------------------------------------------------------

#[test]
fn remediation_renders_verbatim_with_a_copy_affordance() {
    let mut fx = Fixture::new();
    let intent = testdata_generation("accountable-tail");
    let prior = intent["history"][0]["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &prior)], "zagros-sandbox", NOW_MS);

    // The exact pointer the library emitted, before any rendering.
    let emitted = match &env.detail {
        Some(EnvDetail::Consistency(d)) => d.nodes[0].rows[0].remediation.clone(),
        None => unreachable!(),
    };
    assert!(emitted.contains("refresh.yml"), "fixture sanity: a SKEW row carries the reconverge play");

    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let page = fx.render("zagros-sandbox");
    assert!(
        page.contains(&format!(r#"<code class="rem-ptr">{}</code>"#, render::esc(&emitted))),
        "the pointer is rendered VERBATIM, unshortened and unparameterized"
    );
    assert!(page.contains(r#"onclick="cp(this)""#), "one click copies the exact command");
    assert!(page.contains("chip-amber"), "SKEW renders amber with its grade name");
    assert!(page.contains("SKEW"));
}

// ---------------------------------------------------------------------------
// Block 94
// ---------------------------------------------------------------------------

#[test]
fn unset_siblings_hide_links_and_set_siblings_emit_contract_params() {
    let mut fx = Fixture::new();
    let intent = testdata_generation("accountable-tail");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &current)], "zagros-sandbox", NOW_MS);
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });

    // Unset: hidden, not dead.
    let page = fx.render("zagros-sandbox");
    assert!(!page.contains("panoscope &#8599;"));
    assert!(!page.contains("stethoscope &#8599;"));

    // Set: the contract params, no new param minted.
    fx.siblings.panoscope = Some("https://panoscope.zagros.example".into());
    fx.siblings.stethoscope = Some("https://stethoscope.zagros.example".into());
    let page = fx.render("zagros-sandbox");
    assert!(
        page.contains("https://stethoscope.zagros.example/?domain=consistency&amp;site=zagros-sandbox"),
        "the env-health hop: domain= + site="
    );
    assert!(
        page.contains("https://panoscope.zagros.example/?peer=zagros-a&amp;site=zagros-sandbox"),
        "the node hop: peer= + site="
    );
}

#[test]
fn site_param_is_consumed_and_unknown_params_are_ignored_silently() {
    assert_eq!(site_param(Some("site=zagros-sandbox")), Some("zagros-sandbox".into()));
    assert_eq!(
        site_param(Some("domain=consistency&site=zagros%2Dsandbox&mystery=1")),
        Some("zagros-sandbox".into()),
        "site= decodes; domain= and unknown params are ignored, the URL stays shareable"
    );
    assert_eq!(site_param(Some("key=ops%2Ffoo")), None, "key= is not consumed (§7.7)");
    assert_eq!(site_param(None), None);
}

// ---------------------------------------------------------------------------
// Ingest: the key is the identity
// ---------------------------------------------------------------------------

#[test]
fn ingest_trusts_the_key_for_identity_not_the_payload() {
    let mut latest = Latest::default();
    let report = ConsistencyReportV1 { host_id: "liar".into(), ..Default::default() };
    ingest(&mut latest, "ops/consistency/report/zagros-a", &report.encode_to_vec(), NOW_MS);
    assert!(latest.reports.contains_key("zagros-a"));
    assert_eq!(latest.reports["zagros-a"].report.host_id, "zagros-a");
    assert!(!latest.reports.contains_key("liar"));

    let env = EnvHealthV1 { scope: "spoofed-scope".into(), ..Default::default() };
    ingest(&mut latest, "ops/consistency/health/zagros-sandbox", &env.encode_to_vec(), NOW_MS);
    assert!(latest.rollups.contains_key("zagros-sandbox"));
    assert_eq!(latest.rollups["zagros-sandbox"].env.scope, "zagros-sandbox");

    // Garbage is dropped, never partially believed.
    ingest(&mut latest, "ops/consistency/health/zagros-sandbox", b"\xff\xfe not proto", NOW_MS);
    assert_eq!(latest.rollups["zagros-sandbox"].env.scope, "zagros-sandbox");
    // A non-leaf key is ignored rather than guessed at.
    ingest(&mut latest, "ops/consistency/health/a/b", &EnvHealthV1::default().encode_to_vec(), NOW_MS);
    assert!(!latest.rollups.contains_key("a"));
}

// ---------------------------------------------------------------------------
// The C1 island test (§7.9), stated so it can be failed
// ---------------------------------------------------------------------------

/// With the WAN severed and the rest of the fleet unreachable, the console
/// renders its own goatnet completely from its two inputs: a file on disk and
/// a bus that never leaves the site. Here: the verified committed artifact +
/// an in-memory rollup, and NOTHING else — no other scope, no aggregator, no
/// network call in the render path (render_page is a pure function of local
/// state, which is the strongest form of the claim).
#[test]
fn c1_island_the_own_goatnet_renders_completely_from_local_facts() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    let mut fx = Fixture::new();
    fx.intent = verified_committed_intent();
    let intent = fx.intent.0.clone().unwrap();
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &current)], "zagros-sandbox", NOW_MS);
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });

    let page = fx.render("zagros-sandbox");
    // The full altitude stack renders: band grounding, node grid, repo grid.
    // The denominator is the committed artifact's member count, not a pinned
    // literal — the fleet grows (this broke at 2 when #397 made it 4).
    let expected = intent["members"].as_array().unwrap().len();
    assert!(page.contains("live rollup"));
    assert!(page.contains(&format!("1 of {expected} expected nodes reporting")));
    assert!(page.contains("zagros-a"));
    assert!(page.contains("committed-intent grid"));
    // And the cross-goatnet row it cannot see is simply not fabricated: no
    // other scope is rendered, and nothing about the wider fleet is claimed.
    // The claim is that no UNSEEN GOATNET IS RENDERED — not that the other
    // goatnet's name is absent from the document, because a declared `reason` can
    // mention a sibling in prose. Explanatory text naming a sibling is not a
    // fabricated row, so this asserts on the things that exist only when the other
    // goatnet is actually rendered: its node and its scope heading.
    assert!(
        !page.contains("mgmt-zagros-prod-01-orin"),
        "no unseen goatnet's node is invented"
    );
    assert!(
        !page.contains(">zagros-prod<"),
        "no unseen goatnet's scope is rendered"
    );
}

// ---------------------------------------------------------------------------
// Local live fire — the §9 gate: a real rollup envelope over a real wire,
// rendered into the fixture goatnet's grid. Harness identical to the CF-3 meter's.
// ---------------------------------------------------------------------------

async fn peer_pair() -> (zenoh::Session, zenoh::Session) {
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
async fn live_fire_a_published_rollup_renders_zagross_grid_over_a_real_zenoh_wire() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    use zenoh::qos::{CongestionControl, Priority};

    let (console_side, meter_side) = peer_pair().await;
    let subscriber = console_side
        .declare_subscriber(HEALTH_SELECTOR)
        .await
        .expect("declare subscriber");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // The envelope the meter would publish: the fixture's committed (signed) intent,
    // one node on the current generation with a confirmed portal ledger tip.
    let (intent_opt, _) = verified_committed_intent();
    let intent = intent_opt.unwrap();
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let expected_portal = intent["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "portal")
        .and_then(|c| c["members"]["zagros_hosts"]["value"].as_str())
        .expect("zagros_hosts pins the portal")
        .to_string();
    let mut node = observed("zagros-a", "zagros_hosts", &current);
    node.components.push(ObservedComponent {
        component: "goat-zenoh-portal".into(),
        artifact_type: "image".into(),
        reference: expected_portal.replace("{{ ghcr_org }}", "someorg"),
        reconciled: "confirmed".into(),
        live_ref: "x".into(),
    });
    let env = envelope_from(&intent, &[node], "zagros-sandbox", NOW_MS);

    meter_side
        .put("ops/consistency/health/zagros-sandbox", env.encode_to_vec())
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await
        .expect("put the envelope");

    // Receive what actually crossed the wire, through the console's own
    // ingest path — key-derived identity and all.
    let sample = tokio::time::timeout(std::time::Duration::from_secs(10), subscriber.recv_async())
        .await
        .expect("an envelope should arrive within 10s")
        .expect("subscriber recv");
    assert_eq!(sample.key_expr().as_str(), "ops/consistency/health/zagros-sandbox");

    let mut latest = Latest::default();
    ingest(
        &mut latest,
        sample.key_expr().as_str(),
        sample.payload().to_bytes().as_ref(),
        NOW_MS,
    );
    assert!(latest.rollups.contains_key("zagros-sandbox"), "the wire-crossed envelope decoded");

    // The denominator is DERIVED from the committed artifact, never hard-coded:
    // a count the committed artifact owns and this test does not. One observed
    // component over however many the artifact declares for the member.
    // ...and the derivation has to apply the SAME rule the console applies.
    // `components_governed` (project.rs) skips a member whose declared state is
    // `absent`, because "this goatnet does not run that" is a declared answer and
    // not a coverage gap, so counting the raw array would overstate the denominator.
    let declared = intent["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["members"]["zagros_hosts"].is_null())
        .filter(|c| c["members"]["zagros_hosts"]["state"].as_str() != Some("absent"))
        .count();

    // Render the page a browser would get.
    let fx = Fixture {
        latest,
        intent: (Some(intent), String::new()),
        siblings: Siblings::default(),
        grammar: grammar(),
    };
    let page = fx.render("zagros-sandbox");

    // The fixture's grid, from a live (locally-published) rollup:
    assert!(page.contains("live rollup"));
    assert!(page.contains("zagros-a"), "the node row renders");
    assert!(
        page.contains("chip chip-green"),
        "a grounded match renders a green chip element"
    );
    let expected_counts = format!("1 of {declared} observed");
    assert!(page.contains(&expected_counts) || page.contains(&format!("components 1 of {declared}")),
        "counts with denominators — the ledger coverage gap stays a number (wanted '{}'): {}",
        expected_counts,
        &page[page.find("components").unwrap_or(0)..page.find("components").map(|i| (i + 120).min(page.len())).unwrap_or(0)]);
    assert!(page.contains("portal"), "the joined component row renders");
    assert!(!page.contains("nothing heard on"), "a heard rollup is not rendered as absent");

    let _ = meter_side.close().await;
    let _ = console_side.close().await;
}

// ---------------------------------------------------------------------------
// CF-7 — the cross-goatnet aggregator (§7.8). The §9 gate in its local form:
// it renders TWO goatnets, and killing it is a no-op by construction. What
// these tests do NOT prove, stated so nobody reads them as more: mTLS to
// either goatnet's real router through an env-door, or any real rollup. The
// live cross-goatnet render awaits the operator running
// ops/consistency/aggregator/serve.sh.
// ---------------------------------------------------------------------------

/// (name, chosen_site, latest, verified-intent) — one aggregator input.
type GoatnetInput<'a> = (&'a str, &'a str, &'a Latest, &'a (Option<Value>, String));

/// Build the aggregate page for a slice of goatnet inputs — the same View
/// construction the handler performs.
fn aggregate_page(goatnets: &[GoatnetInput], grammar: &Grammar) -> String {
    let siblings = Siblings::default();
    let views: Vec<render::NamedView> = goatnets
        .iter()
        .enumerate()
        .map(|(i, (name, site, latest, intent))| render::NamedView {
            // first goatnet only, so the "one link per configured URL" count
            // in the sibling test is meaningful
            console_url: (i == 0).then_some("https://consistency.zagros.netbird.zagros.example"),
            name,
            view: render::View {
                chosen_site: site,
                latest,
                intent,
                siblings: &siblings,
                grammar,
                now_ms: NOW_MS,
                stale_after_ms: 900_000,
                version: "test",
            },
        })
        .collect();
    render::render_aggregate_page(&views, "test")
}

/// The §7.4 band table's slice of the page — the goatnet-level verdict row
/// set, where the reachability honesty rules bind hardest.
fn band_of(page: &str) -> &str {
    let start = page.find("<h2>goatnet band").expect("the aggregate page has a band");
    let end = start + page[start..].find("</table>").expect("the band is a table");
    &page[start..end]
}

/// The two-goatnet gate over REAL transport: two SEPARATE local fabrics (one
/// peer pair each — separate sessions, exactly as the aggregator holds one
/// session per goatnet because they are separate CAs), each publishing a real
/// envelope built from that goatnet's COMMITTED SIGNED artifact; both
/// received through the console's own ingest path and rendered as the §7.4
/// band plus both grids.
/// A current-generation node whose CF-4 render baseline agrees — the fully
/// GROUNDED case, so the band's worst-grounded-grade column has a grounded
/// verdict to show. (Without the render record, the committed artifacts'
/// declared-but-unrecorded `render_watch` correctly grades the whole node
/// `indeterminate` — the default-off recorder honesty, §5.2a.)
fn grounded_node(host: &str, member: &str, hash: &str) -> ObservedNode {
    let mut node = observed(host, member, hash);
    node.renders.push(goat_consistency::ObservedRender {
        component: "nova-zenoh".into(),
        path: "/opt/zagros-zenoh/docker-compose.yml".into(),
        sha256: "sha256:same".into(),
        recorded_sha256: "sha256:same".into(),
        note: String::new(),
    });
    node
}

#[tokio::test(flavor = "multi_thread")]
async fn live_fire_two_goatnets_two_fabrics_render_the_band_and_both_grids() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    use zenoh::qos::{CongestionControl, Priority};

    // Real wall-clock now: the committed artifacts' generated_at postdates
    // the corpus instant, and generation AGE (a §7.4 band column) is only
    // statable when now is after it.
    let now_ms = now_unix_ms();

    let (zagros_console, zagros_meter) = peer_pair().await;
    let (prod_console, prod_meter) = peer_pair().await;
    let sub_zagros = zagros_console.declare_subscriber(HEALTH_SELECTOR).await.expect("sub zagros");
    let sub_prod = prod_console.declare_subscriber(HEALTH_SELECTOR).await.expect("sub prod");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let (zagros_intent, _) = verified_committed_intent_for("zagros");
    let zagros_intent = zagros_intent.unwrap();
    let (prod_intent, _) = verified_committed_intent_for("zagros_prod");
    let prod_intent = prod_intent.unwrap();

    let zagros_hash = zagros_intent["content_hash"].as_str().unwrap().to_string();
    let prod_hash = prod_intent["content_hash"].as_str().unwrap().to_string();
    let env_zagros = envelope_from(
        &zagros_intent,
        &[grounded_node("zagros-a", "zagros_hosts", &zagros_hash)],
        "zagros-sandbox",
        now_ms,
    );
    let env_prod = envelope_from(
        &prod_intent,
        &[grounded_node("mgmt-zagros-prod-01-orin", "zagros_prod_hosts", &prod_hash)],
        "zagros-prod",
        now_ms,
    );

    zagros_meter
        .put("ops/consistency/health/zagros-sandbox", env_zagros.encode_to_vec())
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await
        .expect("put zagros envelope");
    prod_meter
        .put("ops/consistency/health/zagros-prod", env_prod.encode_to_vec())
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await
        .expect("put zagros-prod envelope");

    // Receive what crossed each wire, through the console's own ingest path,
    // into each goatnet's OWN slot — the aggregator never merges fabrics.
    let mut latest_zagros = Latest::default();
    let mut latest_prod = Latest::default();
    for (sub, latest) in [(&sub_zagros, &mut latest_zagros), (&sub_prod, &mut latest_prod)] {
        let sample = tokio::time::timeout(std::time::Duration::from_secs(10), sub.recv_async())
            .await
            .expect("an envelope should arrive within 10s")
            .expect("subscriber recv");
        ingest(latest, sample.key_expr().as_str(), sample.payload().to_bytes().as_ref(), now_ms);
    }
    assert!(latest_zagros.rollups.contains_key("zagros-sandbox"));
    assert!(latest_prod.rollups.contains_key("zagros-prod"));

    let g = grammar();
    let zagros_v = (Some(zagros_intent), String::new());
    let prod_v = (Some(prod_intent), String::new());
    let siblings = Siblings::default();
    let views = [
        render::NamedView {
            console_url: Some("https://consistency.zagros.netbird.zagros.example"),
            name: "zagros",
            view: render::View {
                chosen_site: "zagros-sandbox",
                latest: &latest_zagros,
                intent: &zagros_v,
                siblings: &siblings,
                grammar: &g,
                now_ms,
                stale_after_ms: 900_000,
                version: "test",
            },
        },
        render::NamedView {
            console_url: None,
            name: "zagros-prod",
            view: render::View {
                chosen_site: "zagros-prod",
                latest: &latest_prod,
                intent: &prod_v,
                siblings: &siblings,
                grammar: &g,
                now_ms,
                stale_after_ms: 900_000,
                version: "test",
            },
        },
    ];
    let page = render::render_aggregate_page(&views, "test");

    // The band: one row per goatnet, each anchoring its own grid stack.
    assert!(page.contains("goatnet band"));
    assert!(page.contains(r##"href="#gn-zagros""##), "zagros's band row links its grid");
    assert!(page.contains(r##"href="#gn-zagros-prod""##), "zagros-prod's band row links its grid");
    assert!(page.contains(r#"id="gn-zagros""#) && page.contains(r#"id="gn-zagros-prod""#));
    // Both goatnets on the current generation: a grounded green match verdict
    // per band row, and only per band row — two.
    eprintln!("BAND<<<{}>>>END", band_of(&page));
    // The grade chip renders the plain label with the grammar term on hover.
    assert_eq!(
        band_of(&page).matches(r#"chip chip-green chip-grade" title="grammar grade: match"#).count(),
        2,
        "one grounded verdict per goatnet"
    );
    // Both grid stacks render through the single-mode code path: two verified
    // intent panels, two live rollups, two repo-vantage grids.
    assert_eq!(page.matches(r#"ok-badge">signature verified"#).count(), 2);
    assert_eq!(page.matches("live rollup").count(), 2);
    assert_eq!(page.matches("committed-intent grid").count(), 2);
    assert!(page.contains("zagros-a") && page.contains("mgmt-zagros-prod-01-orin"));
    assert!(page.contains("zagros_hosts") && page.contains("zagros_prod_hosts"));
    // Grounding per row: n over expected, serial, oldest generation age.
    let band = band_of(&page);
    let zagros_members = views[0].view.intent.0.as_ref().unwrap()["members"].as_array().unwrap().len();
    let prod_members = views[1].view.intent.0.as_ref().unwrap()["members"].as_array().unwrap().len();
    assert!(band.contains(&format!("1 of {zagros_members}")), "zagros reports 1 of its {zagros_members} members");
    assert!(band.contains(&format!("1 of {prod_members}")), "zagros-prod reports 1 of its {prod_members} member(s)");
    assert!(
        band.contains(&format!("serial {}", views[0].view.intent.0.as_ref().unwrap()["serial"]))
            && band.contains(&format!("serial {}", views[1].view.intent.0.as_ref().unwrap()["serial"])),
        "each row carries its goatnet's current serial"
    );
    assert!(!band.contains(">—<"), "both fresh rows state a real oldest-generation age");

    let _ = zagros_meter.close().await;
    let _ = prod_meter.close().await;
    let _ = zagros_console.close().await;
    let _ = prod_console.close().await;
}

/// §7.8 reachability honesty: a configured goatnet with no live envelope
/// renders indeterminate with the age of its last reading (or "never heard")
/// — never green, never absent. Three goatnets: one fresh, one never heard,
/// one stale.
#[test]
fn an_unreachable_goatnet_renders_indeterminate_with_age_never_green_never_absent() {
    let g = grammar();
    let intent = testdata_generation("accountable-tail");
    let current = intent["content_hash"].as_str().unwrap().to_string();

    let mut fresh = Latest::default();
    fresh.rollups.insert(
        "site-fresh".into(),
        StoredEnvelope {
            env: envelope_from(&intent, &[observed("n1", "zagros_hosts", &current)], "site-fresh", NOW_MS),
            received_unix_ms: NOW_MS,
        },
    );
    let silent = Latest::default(); // session up or down — nothing heard either way
    let mut stale = Latest::default();
    stale.rollups.insert(
        "site-stale".into(),
        StoredEnvelope {
            env: envelope_from(&intent, &[observed("n2", "zagros_hosts", &current)], "site-stale", NOW_MS - 3_600_000),
            received_unix_ms: NOW_MS - 3_600_000,
        },
    );

    let iv = (Some(intent.clone()), String::new());
    let page = aggregate_page(
        &[
            ("alpha", "site-fresh", &fresh, &iv),
            ("bravo", "site-silent", &silent, &iv),
            ("charlie", "site-stale", &stale, &iv),
        ],
        &g,
    );

    // Never absent: every configured goatnet has a band row and a grid stack.
    for id in ["gn-alpha", "gn-bravo", "gn-charlie"] {
        assert!(page.contains(&format!(r#"id="{id}""#)), "{id} is present");
    }
    // Never heard: plain words on the chip, the grammar term on hover, with
    // the honest n and the repo serial.
    assert!(page.contains(">never heard</span>"));
    assert!(page.contains("grammar grade: indeterminate — nothing ever heard"));
    assert!(page.contains("never green, never omitted"));
    // Stale: silence stated WITH THE AGE of the last reading, grammar on hover.
    assert!(page.contains("silent — last heard 1h ago"));
    assert!(page.contains("grammar grade: indeterminate — the reading is older than the staleness window"));

    // Never green: the goatnet-level verdict for an unreachable goatnet is
    // never a green chip. (Below the band, a stale grid may still show its
    // HISTORICAL node chips under the explicit shown-as-history banner — the
    // single-mode §7.5 behaviour, unchanged — and the repo-vantage grid is
    // the tree's own claim, labelled "no host consulted". The band is the
    // goatnet verdict, and there the rule is absolute.)
    let band = band_of(&page);
    assert_eq!(
        band.matches("chip chip-green").count(),
        1,
        "exactly one green band verdict — the fresh goatnet's; unheard and stale are never green"
    );
    let page_dark = aggregate_page(
        &[("bravo", "site-silent", &silent, &iv), ("charlie", "site-stale", &stale, &iv)],
        &g,
    );
    assert!(
        !band_of(&page_dark).contains("chip-green"),
        "a band with only unreachable goatnets carries no green verdict at all"
    );
}

/// §7.8 verify-at-read independence: one goatnet's tampered intent refuses
/// ITS grid only — the other goatnet's verified panel, live rollup and repo
/// grid render untouched, and the page never fails whole.
#[test]
fn one_goatnets_tampered_intent_refuses_its_own_grid_only() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    let g = grammar();

    // Goatnet A: verified, with a live envelope.
    let (zagros_intent_opt, _) = verified_committed_intent_for("zagros");
    let zagros_intent = zagros_intent_opt.unwrap();
    let zagros_hash = zagros_intent["content_hash"].as_str().unwrap().to_string();
    let mut latest_zagros = Latest::default();
    latest_zagros.rollups.insert(
        "zagros-sandbox".into(),
        StoredEnvelope {
            env: envelope_from(&zagros_intent, &[observed("zagros-a", "zagros_hosts", &zagros_hash)], "zagros-sandbox", NOW_MS),
            received_unix_ms: NOW_MS,
        },
    );
    let zagros_v = (Some(zagros_intent), String::new());

    // Goatnet B: TAMPERED on the aggregator's disk (its meter-side
    // copy verified fine, so its live envelope still arrives and renders).
    let dir = std::env::temp_dir().join(format!("gcc-agg-tamper-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let committed = consistency_dir().join("intent/zagros_prod/intent.json");
    let pubkeys = consistency_dir().join("../enrollment/public-keys");
    let tampered_path = dir.join("intent.json");
    std::fs::write(
        &tampered_path,
        std::fs::read_to_string(&committed)
            .unwrap()
            .replace("\"goatnet\": \"zagros_prod\"", "\"goatnet\": \"somebody-elses\""),
    )
    .unwrap();
    std::fs::copy(committed.with_file_name("intent.json.sig"), tampered_path.with_file_name("intent.json.sig")).unwrap();
    let prod_v = goat_consistency::verify::load_verified(&tampered_path, &pubkeys);
    assert!(prod_v.0.is_none(), "the tamper must refuse");

    let (prod_meter_intent_opt, _) = verified_committed_intent_for("zagros_prod");
    let prod_meter_intent = prod_meter_intent_opt.unwrap();
    let prod_hash = prod_meter_intent["content_hash"].as_str().unwrap().to_string();
    let mut latest_prod = Latest::default();
    latest_prod.rollups.insert(
        "zagros-prod".into(),
        StoredEnvelope {
            env: envelope_from(&prod_meter_intent, &[observed("mgmt-zagros-prod-01-orin", "zagros_prod_hosts", &prod_hash)], "zagros-prod", NOW_MS),
            received_unix_ms: NOW_MS,
        },
    );

    let page = aggregate_page(
        &[
            ("zagros", "zagros-sandbox", &latest_zagros, &zagros_v),
            ("zagros-prod", "zagros-prod", &latest_prod, &prod_v),
        ],
        &g,
    );

    // B refuses, verbatim, exactly once — and only B.
    assert_eq!(page.matches("committed intent — REFUSED").count(), 1);
    assert!(page.contains("FAILED signature verification"));
    // Exactly ONE repo-vantage grid (A's) and ONE verified badge (A's).
    assert_eq!(page.matches("committed-intent grid").count(), 1);
    assert_eq!(page.matches(r#"ok-badge">signature verified"#).count(), 1);
    // A renders in full — the poison did not cross goatnets.
    assert!(page.contains("zagros_hosts") && page.contains("zagros-a"));
    // B is refused, not absent: its section, band row and LIVE rollup (the
    // meter's own verification state, §7 as-built) still render.
    assert!(page.contains(r#"id="gn-zagros-prod""#));
    assert!(page.contains("mgmt-zagros-prod-01-orin"));
    // B's band serial column says the intent refused rather than inventing a
    // repo serial — but its live reading still carries the wire facts.
    // (`<h2>` anchored: B's refusal prose also SAYS "live rollup".)
    assert_eq!(page.matches("<h2>live rollup").count(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

/// §7.9 C1 converse + §9 gate clause 2: killing the aggregator changes
/// nothing on any goatnet — BY CONSTRUCTION, pinned two ways. (1) Statically:
/// the binary's non-test sources declare no publisher, put nothing, and
/// write no file — there is no state a kill could lose and no output a
/// goatnet could miss. (2) The render is a pure function of local state:
/// same inputs, byte-identical page — nothing external is consulted, so
/// nothing external can depend on the render having happened.
#[test]
fn killing_the_aggregator_is_a_no_op_by_construction() {
    for (name, whole) in [
        ("main.rs", include_str!("main.rs")),
        ("render.rs", include_str!("render.rs")),
        ("aggregate.rs", include_str!("aggregate.rs")),
        ("api.rs", include_str!("api.rs")),
    ] {
        // Non-test source only: a #[cfg(test)] module may build fixtures.
        let src = whole.split("#[cfg(test)]").next().unwrap();
        assert!(!src.contains("declare_publisher"), "{name} must declare no publisher");
        assert!(!src.contains(".put("), "{name} must publish nothing");
        assert!(!src.contains("fs::write") && !src.contains("File::create"), "{name} must write no file");
    }

    let g = grammar();
    let intent = testdata_generation("accountable-tail");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let mut latest = Latest::default();
    latest.rollups.insert(
        "site-a".into(),
        StoredEnvelope {
            env: envelope_from(&intent, &[observed("n1", "zagros_hosts", &current)], "site-a", NOW_MS),
            received_unix_ms: NOW_MS,
        },
    );
    let iv = (Some(intent.clone()), String::new());
    let silent = Latest::default();
    let inputs = [("alpha", "site-a", &latest, &iv), ("bravo", "site-b", &silent, &iv)];
    // Rendering twice changes nothing and depends on nothing but the inputs.
    let once = aggregate_page(&inputs, &g);
    let twice = aggregate_page(&inputs, &g);
    assert_eq!(once, twice, "the aggregate render is a pure function of its inputs");
}

// ---------------------------------------------------------------------------
// Aggregator-product (§7.8 as-built): the drift-first view + the JSON API.
// The cross-env verdicts come from goat_consistency::diff_intents (grade_row
// with one column per goatnet:member — its own unit tests live in the
// library); what is pinned HERE is the projection: the panel answers the
// platform-admin question above the fold, and the API carries the same
// honesty grammar in machine shape.
// ---------------------------------------------------------------------------

/// Two synthetic committed-artifact-shaped intents exercising every verdict
/// class the drift view renders: identical, differing-declared,
/// differing-undeclared, and present-in-one-artifact-only.
fn drift_fixture_intents() -> (Value, Value) {
    let a = serde_json::json!({
        "goatnet": "alpha",
        "members": ["alpha_hosts"],
        "serial": 3,
        "content_hash": "sha256:af1",
        "generated_at": "2026-08-01T00:00:00Z",
        "components": [
            {"name": "same-image", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/x/same@sha256:aaaa"}}},
            {"name": "drifted-image", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/x/thing@sha256:new1"}}},
            {"name": "declared-split", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/x/split@sha256:mine",
                                 "declared": {"reason": "alpha runs the fork on purpose", "scope": "alpha_hosts"}}}},
            {"name": "only-in-alpha", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/x/only@sha256:solo"}}}
        ],
        "capabilities": []
    });
    let b = serde_json::json!({
        "goatnet": "bravo",
        "members": ["bravo_hosts"],
        "serial": 9,
        "content_hash": "sha256:bf1",
        "generated_at": "2026-08-01T00:00:00Z",
        "components": [
            {"name": "same-image", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": "ghcr.io/x/same@sha256:aaaa"}}},
            {"name": "drifted-image", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": "ghcr.io/x/thing@sha256:old9"}}},
            {"name": "declared-split", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": "ghcr.io/x/split@sha256:theirs"}}}
        ],
        "capabilities": []
    });
    (a, b)
}

#[test]
fn the_drift_panel_answers_the_platform_admin_question_above_the_fold() {
    let g = grammar();
    let (a, b) = drift_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );

    // The panel exists and sits ABOVE the band; the full grids sit behind a
    // drill-down, rendered in full but collapsed.
    let drift_pos = page.find("<h2>cross-env drift").expect("the drift panel renders");
    let band_pos = page.find("<h2>goatnet band").expect("the band renders");
    assert!(drift_pos < band_pos, "the drift question is answered above the fold");
    assert!(page.contains(r#"<details class="goatnet""#), "full grids are a drill-down");
    assert!(page.contains("committed-intent grid"), "collapsed is presentation, not omission");

    // Summary line first: ONE coherent reading — the denominator once, each
    // category once, admin words (the grammar term stays on the rows).
    assert!(page.contains("4 rows compared across alpha + bravo"));
    assert!(page.contains("1 undeclared drift"), "the drift signal, in admin words");
    assert!(page.contains("1 declared difference"));
    assert!(page.contains("1 match"));
    assert!(page.contains("1 not comparable"), "present-in-one-only is an answer, not a blank");

    // Every verdict still carries its library-generated why, verbatim —
    // behind the expansion and on the title attribute (conclusion-first).
    assert!(page.contains("they differ and nothing declares why"));
    assert!(page.contains("the difference is declared with a reason"));
    assert!(page.contains("not governed in bravo&#39;s intent artifact"));
    // The undeclared row carries the grammar's remediation, verbatim + copyable.
    assert!(page.contains("deviations:"), "the declare pointer renders");
}

#[test]
fn a_refused_intent_makes_the_drift_panel_say_not_comparable_never_less() {
    let g = grammar();
    let (a, _) = drift_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b: (Option<Value>, String) =
        (None, "intent.json FAILED signature verification (test fixture)".into());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    assert!(page.contains("cross-env drift"), "the panel is present even when not comparable");
    assert!(page.contains("not comparable"));
    assert!(page.contains("1 of 2 configured goatnet(s) verified"));
    assert!(
        page.contains("bravo is NOT compared"),
        "the refused env is named, with its refusal"
    );
}

/// The drift panel's own section of the page — assertions about the drift
/// TABLE must not accidentally match the per-goatnet grids behind it.
fn drift_section(page: &str) -> &str {
    let start = page.find("<h2>cross-env drift").expect("the drift panel renders");
    let end = page[start..].find("</section>").map(|i| start + i).unwrap_or(page.len());
    &page[start..end]
}

/// Drift-page usability: the row STRUCTURE. Match rows collapse
/// to one compact line with the single shared value; every non-match row
/// carries its why + remediation in a full-width sub-row spanning the table
/// (never a narrow trailing column); row order is the panel's own priority
/// statement — SKEW first, matches last.
#[test]
fn drift_rows_sort_priority_first_and_match_rows_collapse_compact() {
    let g = grammar();
    let (a, b) = drift_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);

    // Priority order, top to bottom: undeclared SKEW (THE drift signal),
    // then indeterminate, then declared, matches collapsed at the bottom.
    let pos = |name: &str| {
        drift
            .find(&format!(r#"title="component {name}""#))
            .unwrap_or_else(|| panic!("{name} row renders"))
    };
    assert!(pos("drifted-image") < pos("only-in-alpha"), "SKEW sorts before indeterminate");
    assert!(pos("only-in-alpha") < pos("declared-split"), "indeterminate sorts before declared");
    assert!(pos("declared-split") < pos("same-image"), "declared sorts before match");

    // why/remediation are OUT of the column tracks: no such headers exist —
    // each non-match row carries its bound explanation block INSIDE the row
    // block instead, under the subject column.
    assert!(!drift.contains(">why<"), "why is not a grid column");
    assert!(!drift.contains(">remediation<"), "remediation is not a grid column");
    assert_eq!(
        drift.matches(r#"<div class="gdetail">"#).count(),
        3,
        "SKEW + indeterminate + declared each carry a bound explanation block"
    );
    // Density tiers (drift-page-grid): the SKEW and indeterminate rows render
    // EXPANDED with their severity edge; the declared row is ONE line whose
    // explanation sits behind the row's own disclosure.
    assert_eq!(drift.matches(r#"<div class="grow has-detail sev-"#).count(), 2);
    assert!(drift.contains(r#"<div class="grow has-detail sev-amber">"#));
    assert_eq!(
        drift.matches(r#"<details class="grow gdecl sev-"#).count(),
        1,
        "the declared row collapses behind its disclosure"
    );

    // The match row is one compact grid-aligned line: no explanation block,
    // no why, no remediation.
    assert_eq!(drift.matches(r#"<div class="grow row-match">"#).count(), 1);
    let m_start = drift.find(r#"<div class="grow row-match">"#).unwrap();
    let m_end = m_start + drift[m_start..].find("</div></div>").unwrap();
    let match_row = &drift[m_start..m_end];
    assert!(!match_row.contains("why"), "a match needs no explanation");
    assert!(!match_row.contains("rem-ptr"), "a match carries no remediation");

    // The env column headers are a sticky row with explicit tracks — the
    // grid template names the env-column count.
    assert!(drift.contains(r#"<div class="grid" style="--envn:2">"#));
    assert!(drift.contains(r#"<div class="ghead">"#), "the header row renders (sticky via CSS)");
}

/// Drift-page usability: when every env's cell is IDENTICAL the value renders
/// ONCE, across a spanned cell — never repeated per column.
#[test]
fn identical_values_across_envs_render_once_not_per_column() {
    let g = grammar();
    let (a, b) = drift_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(
        drift.contains(r#"<span class="gval gval-agree mono"><span class="dim">all:</span> <details class="ref"><summary>aaaa…</summary>"#),
        "the agreed value spans the env columns, stated once — human identity by default"
    );
    assert_eq!(
        drift.matches("ghcr.io/x/same@sha256:aaaa").count(),
        1,
        "the shared verbatim ref is printed once (behind expansion), not once per env"
    );
    // Differing rows keep their per-env cells.
    assert!(drift.contains("ghcr.io/x/thing@sha256:new1"));
    assert!(drift.contains("ghcr.io/x/thing@sha256:old9"));

    // No phantom column: the diff's env-level "not governed here" cell folds
    // into that env's member column instead of minting a `bravo` column of
    // "no column" filler across every other row.
    assert!(
        !drift.contains(r#"<span class="gh gh-env mono" title="bravo">"#),
        "no env-only phantom column"
    );
    assert!(!drift.contains("no column"), "no filler cells anywhere in the drift table");
    assert!(
        drift.contains("unset: not governed in bravo&#39;s intent artifact"),
        "the absence still renders as its own statement, in bravo's member column"
    );
}

/// Drift-page usability: a literal `{{ var }}` templating token in a declared
/// value (the CF-4 ghcr_org rule) renders as a subdued `<var>` token —
/// DISPLAY-ONLY: the verbatim string stays on the title attribute, and the
/// API (untouched by this change) keeps serving the artifact's exact bytes.
#[test]
fn template_braces_render_as_subdued_tokens_display_only() {
    let g = grammar();
    let templated = |gn: &str, member: &str| {
        serde_json::json!({
            "goatnet": gn,
            "members": [member],
            "serial": 1,
            "content_hash": format!("sha256:{gn}1"),
            "generated_at": "2026-08-01T00:00:00Z",
            "components": [
                {"name": "mgmt-image", "artifact_type": "image", "members": {
                    member: {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/netbird-mgmt:latest"}}}
            ],
            "capabilities": []
        })
    };
    let iv_a = (Some(templated("alpha", "alpha_hosts")), String::new());
    let iv_b = (Some(templated("bravo", "bravo_hosts")), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(
        drift.contains(
            r#"<span class="tmpl" title="verbatim in the artifact: {{ ghcr_org }}">&lt;ghcr_org&gt;</span>"#
        ),
        "the templating token renders subdued, with the verbatim string on hover"
    );
    assert!(
        drift.contains(r#"ghcr.io/<span class="tmpl""#),
        "the braces are out of the visible text flow"
    );
}

/// Drift-page usability, defect 6: the per-node grid (single mode and the
/// per-goatnet stacks — same render path) gets the same structural fix: no
/// why/remediation trailing columns; a non-match row explains itself in a
/// full-width sub-row; a match-shaped value states itself once.
#[test]
fn node_grid_nonmatch_rows_explain_full_width_below_the_row() {
    let mut fx = Fixture::new();
    let intent = testdata_generation("accountable-tail");
    let prior = intent["history"][0]["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &prior)], "zagros-sandbox", NOW_MS);
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let page = fx.render("zagros-sandbox");

    let node_start = page.find(r#"<article class="node">"#).expect("the node grid renders");
    let node_end = node_start + page[node_start..].find("</article>").unwrap();
    let node = &page[node_start..node_end];
    assert!(!node.contains("<th>why</th>"), "why is not a node-grid column");
    assert!(!node.contains("<th>remediation"), "remediation is not a node-grid column");
    assert!(
        node.contains(r#"<tr class="rowdetail sev-amber"><td colspan="4">"#),
        "the SKEW row's why + pointer span the node table's full width"
    );
    assert!(node.contains("rem-ptr"), "the verbatim pointer still renders, copyable");
}

/// Build the API inputs the handler builds, over the same fixture shape the
/// render tests use.
#[allow(clippy::too_many_arguments)]
fn api_env<'a>(
    name: &'a str,
    endpoint: &'a str,
    site: &'a str,
    latest: &'a Latest,
    intent: &'a (Option<Value>, String),
    grammar: &'a Grammar,
    siblings: &'a Siblings,
    now_ms: i64,
) -> api::ApiEnv<'a> {
    api::ApiEnv {
        name,
        endpoint,
        view: render::View {
            chosen_site: site,
            latest,
            intent,
            siblings,
            grammar,
            now_ms,
            stale_after_ms: 900_000,
            version: "test",
        },
    }
}

/// The §7.8 API gate in its local form: two goatnets' envelopes built from
/// BOTH committed signed artifacts, published over two real local Zenoh
/// transports, received through the console's own ingest path — and the
/// /api/aggregate document asserted field by field. What this does NOT
/// prove, stated so nobody reads it as more: mTLS through an env-door, a
/// real router's ACL, or the hosted fence.
#[tokio::test(flavor = "multi_thread")]
async fn live_fire_the_aggregate_api_carries_both_goatnets_and_the_drift_section() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    use zenoh::qos::{CongestionControl, Priority};

    let now_ms = now_unix_ms();
    let (zagros_console, zagros_meter) = peer_pair().await;
    let (prod_console, prod_meter) = peer_pair().await;
    let sub_zagros = zagros_console.declare_subscriber(HEALTH_SELECTOR).await.expect("sub zagros");
    let sub_prod = prod_console.declare_subscriber(HEALTH_SELECTOR).await.expect("sub prod");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let (zagros_intent, _) = verified_committed_intent_for("zagros");
    let zagros_intent = zagros_intent.unwrap();
    let (prod_intent, _) = verified_committed_intent_for("zagros_prod");
    let prod_intent = prod_intent.unwrap();
    // Denominators come from the committed artifacts, not pinned literals —
    // the fleet grows (2 broke to 4 with #397, 1 to 2 with #442).
    let zagros_expected = zagros_intent["members"].as_array().unwrap().len();
    let prod_expected = prod_intent["members"].as_array().unwrap().len();
    let zagros_hash = zagros_intent["content_hash"].as_str().unwrap().to_string();
    let prod_hash = prod_intent["content_hash"].as_str().unwrap().to_string();

    zagros_meter
        .put(
            "ops/consistency/health/zagros-sandbox",
            envelope_from(&zagros_intent, &[grounded_node("zagros-a", "zagros_hosts", &zagros_hash)], "zagros-sandbox", now_ms)
                .encode_to_vec(),
        )
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await
        .expect("put zagros envelope");
    prod_meter
        .put(
            "ops/consistency/health/zagros-prod",
            envelope_from(&prod_intent, &[grounded_node("mgmt-zagros-prod-01-orin", "zagros_prod_hosts", &prod_hash)], "zagros-prod", now_ms)
                .encode_to_vec(),
        )
        .congestion_control(CongestionControl::Block)
        .priority(Priority::InteractiveHigh)
        .await
        .expect("put zagros-prod envelope");

    let mut latest_zagros = Latest::default();
    let mut latest_prod = Latest::default();
    for (sub, latest) in [(&sub_zagros, &mut latest_zagros), (&sub_prod, &mut latest_prod)] {
        let sample = tokio::time::timeout(std::time::Duration::from_secs(10), sub.recv_async())
            .await
            .expect("an envelope should arrive within 10s")
            .expect("subscriber recv");
        ingest(latest, sample.key_expr().as_str(), sample.payload().to_bytes().as_ref(), now_ms);
    }

    let g = grammar();
    let siblings = Siblings::default();
    let zagros_serial = zagros_intent["serial"].as_u64().unwrap();
    let iv_zagros = (Some(zagros_intent), String::new());
    let iv_prod = (Some(prod_intent), String::new());
    let envs = [
        api_env("zagros", "tls/127.0.0.1:17447", "zagros-sandbox", &latest_zagros, &iv_zagros, &g, &siblings, now_ms),
        api_env("zagros-prod", "tls/127.0.0.1:27447", "zagros-prod", &latest_prod, &iv_prod, &g, &siblings, now_ms),
    ];
    let doc = api::aggregate_doc(&envs, "aggregate", "test", now_ms);

    // The document dates and versions itself.
    assert_eq!(doc["schema"], "aggregate-v1");
    assert_eq!(doc["mode"], "aggregate");
    assert_eq!(doc["generated_unix_ms"], now_ms);

    // Both goatnets, by name, live, with intent identity.
    let gns = doc["goatnets"].as_array().unwrap();
    assert_eq!(gns.len(), 2);
    let zagros = &gns[0];
    assert_eq!(zagros["goatnet"], "zagros");
    assert_eq!(zagros["site"], "zagros-sandbox");
    assert_eq!(zagros["reachability"]["state"], "live");
    assert!(zagros["reachability"]["last_heard_age_s"].is_i64());
    assert_eq!(zagros["intent"]["verified"], true);
    assert_eq!(zagros["intent"]["serial"], zagros_serial);
    assert!(zagros["intent"]["content_hash"].as_str().unwrap().starts_with("sha256:"));

    // Convergence: counts with denominators, the shared worst-grounded
    // collapse, and nodes whose every graded row carries its why.
    let conv = &zagros["convergence"];
    assert_eq!(conv["counts"]["nodes_reporting"], 1);
    assert_eq!(conv["counts"]["nodes_expected"], zagros_expected as u64);
    assert!(conv["worst_grounded_grade"]["grade"].is_string());
    assert!(conv["worst_grounded_grade"]["color"].is_string());
    let nodes = conv["nodes"].as_array().unwrap();
    assert_eq!(nodes[0]["host"], "zagros-a");
    assert!(nodes[0]["generation"]["serial"].is_number());
    for row in nodes[0]["rows"].as_array().unwrap() {
        assert!(row["grade"].is_string() && !row["grade"].as_str().unwrap().is_empty());
        assert!(row["why"].is_string() && !row["why"].as_str().unwrap().is_empty(),
            "every graded row carries its why: {row}");
    }
    assert_eq!(gns[1]["goatnet"], "zagros-prod");
    assert_eq!(gns[1]["reachability"]["state"], "live");
    assert_eq!(gns[1]["convergence"]["counts"]["nodes_expected"], prod_expected as u64);

    // The cross-env section: the same library report, counts summing to the
    // denominator, cells verbatim from the artifacts.
    let ce = &doc["cross_env"];
    assert_eq!(ce["comparable"], true);
    let compared = ce["summary"]["rows_compared"].as_u64().unwrap();
    let by_grade = ce["summary"]["by_grade"].as_object().unwrap();
    assert_eq!(by_grade.values().map(|v| v.as_u64().unwrap()).sum::<u64>(), compared);
    assert!(ce["summary"]["undeclared_drift"].is_u64());
    let rows = ce["rows"].as_array().unwrap();
    assert_eq!(rows.len() as u64, compared);
    for row in rows {
        assert!(row["why"].is_string() && !row["why"].as_str().unwrap().is_empty());
        assert!(row["cells"].is_object());
    }
    // Column identity is goatnet:member — a cell of one goatnet and one of
    // another both appear on the shared subjects.
    let portal = rows.iter().find(|r| r["name"] == "zenoh-router").expect("both artifacts govern the router");
    assert!(portal["cells"].get("zagros:zagros_hosts").is_some());
    assert!(portal["cells"].get("zagros-prod:zagros_prod_hosts").is_some());

    let _ = zagros_meter.close().await;
    let _ = prod_meter.close().await;
    let _ = zagros_console.close().await;
    let _ = prod_console.close().await;
}

/// §7.8 reachability honesty, machine-shaped: a configured goatnet with no
/// live envelope is an ENTRY WITH A STATED STATE — never-heard with a null
/// age, stale with the age of its last reading — never green, never omitted.
#[test]
fn the_api_states_unreachability_with_age_never_omits_a_goatnet() {
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intent").is_dir() {
        return; // reads this deployment's own artifacts; a public clone has none
    }
    let g = grammar();
    let siblings = Siblings::default();
    let intent = testdata_generation("accountable-tail");
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let iv = (Some(intent.clone()), String::new());

    let silent = Latest::default();
    let mut stale = Latest::default();
    stale.rollups.insert(
        "site-stale".into(),
        StoredEnvelope {
            env: envelope_from(&intent, &[observed("n2", "zagros_hosts", &current)], "site-stale", NOW_MS - 3_600_000),
            received_unix_ms: NOW_MS - 3_600_000,
        },
    );

    let envs = [
        api_env("bravo", "tcp/1:1", "site-silent", &silent, &iv, &g, &siblings, NOW_MS),
        api_env("charlie", "tcp/1:2", "site-stale", &stale, &iv, &g, &siblings, NOW_MS),
    ];
    let doc = api::aggregate_doc(&envs, "aggregate", "test", NOW_MS);
    let gns = doc["goatnets"].as_array().unwrap();
    assert_eq!(gns.len(), 2, "an unreachable goatnet is never omitted");

    let never = &gns[0];
    assert_eq!(never["reachability"]["state"], "never-heard");
    assert!(never["reachability"]["last_heard_age_s"].is_null(), "no fabricated age");
    assert!(never["reachability"]["note"].as_str().unwrap().contains("silently denied"));
    assert!(never["convergence"].is_null(), "no convergence is fabricated");
    assert_eq!(never["intent"]["verified"], true, "the repo-side facts still carry");

    let stale_gn = &gns[1];
    assert_eq!(stale_gn["reachability"]["state"], "stale");
    assert_eq!(stale_gn["reachability"]["last_heard_age_s"], 3600);
    // The stale reading itself is still served — as history with its ages,
    // exactly like the page.
    assert_eq!(stale_gn["convergence"]["counts"]["nodes_reporting"], 1);

    // One goatnet alone: cross_env says not comparable, with the reason.
    let one = [api_env("bravo", "tcp/1:1", "site-silent", &silent, &iv, &g, &siblings, NOW_MS)];
    let doc = api::aggregate_doc(&one, "single", "test", NOW_MS);
    assert_eq!(doc["cross_env"]["comparable"], false);
    assert!(doc["cross_env"]["note"].as_str().unwrap().contains("at least two"));
}

// ---------------------------------------------------------------------------
// Drift-page human-first pass (drift-page-human): the operator's
// four asks, each pinned. 1) ONE coherent summary — every row counted once,
// one denominator, admin words. 2) Machine hashes behind expansion, the tag
// by default. 3a) Drift distance only where deploy-ledger provenance grounds
// it — never fabricated. 3b) Action-safety labels from the closed grammar
// set. 4) Same defaults on the shared render path (cells everywhere).
// ---------------------------------------------------------------------------

/// 64-hex digests (commit-tag-shaped refs) for the expansion + distance
/// fixtures — realistic length, so the "machine-only by default" rule is
/// exercised on what production renders.
const DIG_A: &str = "e68a6ae0e68a6ae0e68a6ae0e68a6ae0e68a6ae0e68a6ae0e68a6ae0e68a6ae0";
const DIG_B: &str = "a7e524c9a7e524c9a7e524c9a7e524c9a7e524c9a7e524c9a7e524c9a7e524c9";
const DIG_C: &str = "0123abcd0123abcd0123abcd0123abcd0123abcd0123abcd0123abcd0123abcd";
/// 2026-08-07T00:00:00Z / 2026-07-30T00:00:00Z.
const TS_AUG7_MS: i64 = 1_786_060_800_000;
const TS_JUL30_MS: i64 = 1_785_369_600_000;

/// Two intents whose one governed component drifts between commit-shaped
/// tagged refs (the production shape: `:sha-<commit>@sha256:<digest>`).
fn tagged_drift_intents() -> (Value, Value) {
    let mk = |gn: &str, member: &str, tag: &str, dig: &str| {
        serde_json::json!({
            "goatnet": gn,
            "members": [member],
            "serial": 1,
            "content_hash": format!("sha256:{gn}1"),
            "generated_at": "2026-08-01T00:00:00Z",
            "components": [
                {"name": "app", "artifact_type": "image", "members": {
                    member: {"state": "resolved",
                             "value": format!("ghcr.io/{{{{ ghcr_org }}}}/app:{tag}@sha256:{dig}")}}}
            ],
            "capabilities": []
        })
    };
    (
        mk("alpha", "alpha_hosts", "sha-e68a6ae", DIG_A),
        mk("bravo", "bravo_hosts", "sha-a7e524c", DIG_B),
    )
}

/// A raw per-node report carrying one confirmed deploy-ledger tip — what the
/// aggregator hears on `ops/consistency/report/<host>` and the only grounded
/// per-component provenance the console holds.
fn report_with_tip(host: &str, component: &str, r#ref: &str, prior: &str, ts: i64, reconciled: &str) -> StoredReport {
    StoredReport {
        report: ConsistencyReportV1 {
            host_id: host.into(),
            components: vec![goat_consistency_wire::ComponentStateV1 {
                component: component.into(),
                artifact_type: "image".into(),
                r#ref: r#ref.into(),
                prior_ref: prior.into(),
                ts_unix_ms: ts,
                reversible: true,
                reconciled: reconciled.into(),
                live_ref: String::new(),
            }],
            ..Default::default()
        },
        received_unix_ms: NOW_MS,
    }
}

#[test]
fn the_drift_summary_counts_each_row_once_and_speaks_admin_language() {
    let g = grammar();
    let (a, b) = drift_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);

    // The old shape is GONE: no chip that repeats the SKEW row under a second
    // vocabulary, no denominator on one category only.
    assert!(!drift.contains("UNDECLARED DRIFT"), "no shouting duplicate of the SKEW count");
    assert!(!drift.contains("of 4 rows"), "the denominator is stated once, up front");
    assert!(!drift.contains("1 SKEW"), "the summary speaks admin words, not the grammar term");

    // Each category exactly once, one denominator: 4 rows = 1 + 1 + 1 + 1.
    let summary_start = drift.find(r#"<p class="drift-summary">"#).expect("summary renders");
    let summary_end = summary_start + drift[summary_start..].find("</p>").unwrap();
    let summary = &drift[summary_start..summary_end];
    assert!(summary.contains("4 rows compared across alpha + bravo:"));
    assert_eq!(summary.matches("undeclared drift").count(), 1);
    assert!(summary.contains("1 undeclared drift"));
    assert!(summary.contains("1 declared difference"));
    assert!(summary.contains("1 not comparable"));
    assert!(summary.contains("1 match"));

    // The grammar term is not lost: every chip carries it on hover — the row
    // chip renders the plain words ("undeclared drift"), never a bare term.
    assert!(!drift.contains(">SKEW<"), "no bare grammar term in the visible flow");
    assert!(
        drift.contains(r#"chip-grade" title="grammar grade: SKEW — an undeclared difference — THE drift signal">undeclared drift</span>"#),
        "the row chip speaks plain words with the grammar term on hover"
    );
    assert!(summary.contains("grammar grade: SKEW"), "the summary chip names its grammar grade on hover");
}

#[test]
fn zero_drift_states_no_undeclared_drift_rather_than_omitting_the_category() {
    let g = grammar();
    let (a, _) = drift_fixture_intents();
    // bravo = alpha's twin under another name: every row matches.
    let mut b = a.clone();
    b["goatnet"] = serde_json::json!("bravo");
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(
        drift.contains(r#"chip-green drift-chip" title="grammar grade: SKEW — zero rows carry an undeclared difference">no undeclared drift"#),
        "a zero is a stated green answer, never an omitted category"
    );
    assert_eq!(
        drift.matches("undeclared drift").count(),
        1,
        "the drift category is stated exactly once even at zero"
    );
}

#[test]
fn digests_and_verbatim_refs_render_behind_expansion_not_by_default() {
    let g = grammar();
    let (a, b) = tagged_drift_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);

    // Default cell = the version identity a human compares: the tag.
    assert!(drift.contains("<summary>sha-e68a6ae</summary>"), "alpha's cell leads with its tag");
    assert!(drift.contains("<summary>sha-a7e524c</summary>"), "bravo's cell leads with its tag");
    // The verbatim templated ref + full digest sit behind the expansion.
    assert!(
        drift.contains(&format!(r#"<code class="ref-full">ghcr.io/<span class="tmpl" title="verbatim in the artifact: {{{{ ghcr_org }}}}">&lt;ghcr_org&gt;</span>/app:sha-e68a6ae@sha256:{DIG_A}</code>"#)),
        "the machine ref is one click away, verbatim (tokens subdued)"
    );
    // No cell renders a raw ref in the default flow.
    assert!(!drift.contains(r#"<span class="gval mono">ghcr"#), "no default cell starts with a raw ref");

    // The why enumeration (behind the conclusion's expansion) shortens
    // machine-length digests; the verbatim sentence stays on the title.
    assert!(
        drift.contains(&format!("@sha256:{}…", &DIG_A[..12])),
        "the why enumeration carries the shortened digest"
    );
    let why_start =
        drift.find(r#"<div class="why-full" title="#).expect("the why carries its verbatim title");
    let why_end = why_start + drift[why_start..].find("</div>").unwrap();
    assert!(drift[why_start..why_end].contains(DIG_A), "the verbatim why is on the title attribute");

    // API UNCHANGED: the machine surface still serves the exact bytes.
    let siblings = Siblings::default();
    let (a2, b2) = tagged_drift_intents();
    let iv_a2 = (Some(a2), String::new());
    let iv_b2 = (Some(b2), String::new());
    let envs = [
        api_env("alpha", "tcp/1:1", "site-a", &silent_a, &iv_a2, &g, &siblings, NOW_MS),
        api_env("bravo", "tcp/1:2", "site-b", &silent_b, &iv_b2, &g, &siblings, NOW_MS),
    ];
    let doc = api::aggregate_doc(&envs, "aggregate", "test", NOW_MS);
    let cells = &doc["cross_env"]["rows"][0]["cells"];
    assert_eq!(
        cells["alpha:alpha_hosts"]["value"],
        format!("ghcr.io/{{{{ ghcr_org }}}}/app:sha-e68a6ae@sha256:{DIG_A}"),
        "the API cell is verbatim — display shaping never touches the machine side"
    );
}

#[test]
fn a_digest_only_ref_defaults_to_the_shortened_digest_its_only_identity() {
    let g = grammar();
    let (mut a, mut b) = tagged_drift_intents();
    a["components"][0]["members"]["alpha_hosts"]["value"] =
        serde_json::json!(format!("ghcr.io/x/app@sha256:{DIG_A}"));
    b["components"][0]["members"]["bravo_hosts"]["value"] =
        serde_json::json!(format!("ghcr.io/x/app@sha256:{DIG_B}"));
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(
        drift.contains(&format!("<summary>{}…</summary>", &DIG_A[..12])),
        "with no tag, the shortened digest is the only honest identity — full behind expansion"
    );
}

// -- 3a: drift distance, grounded or absent ---------------------------------

#[test]
fn drift_distance_states_direction_only_when_a_ledger_moved_off_the_other_version() {
    let g = grammar();
    let (a, b) = tagged_drift_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    // alpha's own fabric heard a confirmed ledger tip: current = alpha's
    // intent digest, prior = bravo's current digest — alpha provably moved
    // OFF bravo's version .
    let mut latest_a = Latest::default();
    latest_a.reports.insert(
        "a1".into(),
        report_with_tip(
            "a1",
            "app",
            &format!("ghcr.io/realorg/app@sha256:{DIG_A}"),
            &format!("ghcr.io/realorg/app@sha256:{DIG_B}"),
            TS_AUG7_MS,
            "confirmed",
        ),
    );
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &latest_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(
        drift.contains("versions differ — alpha ahead since 2026-08-07 — its deploy ledger moved off bravo&#39;s current version then"),
        "direction is claimed only from the prior_ref linkage, with the ledger date: {}",
        &drift[drift.find("drift-distance").map(|i| i.saturating_sub(0)).unwrap_or(0)..drift.find("drift-distance").map(|i| (i + 300).min(drift.len())).unwrap_or(0)]
    );
}

#[test]
fn drift_distance_dates_without_direction_disclaim_ordering() {
    let g = grammar();
    let (a, b) = tagged_drift_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    // Each env's ledger confirms its own value's deploy date; neither prior
    // links to the other's version, so no direction may be claimed.
    let mut latest_a = Latest::default();
    latest_a.reports.insert(
        "a1".into(),
        report_with_tip("a1", "app", &format!("ghcr.io/o/app@sha256:{DIG_A}"), &format!("ghcr.io/o/app@sha256:{DIG_C}"), TS_AUG7_MS, "confirmed"),
    );
    let mut latest_b = Latest::default();
    latest_b.reports.insert(
        "b1".into(),
        report_with_tip("b1", "app", &format!("ghcr.io/o/app@sha256:{DIG_B}"), "", TS_JUL30_MS, "confirmed"),
    );
    let page = aggregate_page(
        &[("alpha", "site-a", &latest_a, &iv_a), ("bravo", "site-b", &latest_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(drift.contains("alpha deployed its value 2026-08-07"));
    assert!(drift.contains("bravo deployed its value 2026-07-30"));
    assert!(
        drift.contains("a deploy date alone does not order versions"),
        "dates without a prior_ref linkage carry the honest caveat"
    );
    assert!(!drift.contains("ahead since"), "no direction is invented from dates");
}

#[test]
fn drift_distance_is_never_fabricated_without_confirmed_ledger_provenance() {
    let g = grammar();
    let (a, b) = tagged_drift_intents();

    // No reports at all: the line says why the distance is not statable.
    let iv_a = (Some(a.clone()), String::new());
    let iv_b = (Some(b.clone()), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(drift.contains("versions differ — how far is not statable: no confirmed deploy-ledger record heard"));
    assert!(!drift.contains("ahead since") && !drift.contains("deployed its value"));

    // An UNRECONCILED tip is an optimistic row (§5.2) — it grounds nothing,
    // even when its refs would imply a direction.
    let mut latest_a = Latest::default();
    latest_a.reports.insert(
        "a1".into(),
        report_with_tip("a1", "app", &format!("ghcr.io/o/app@sha256:{DIG_A}"), &format!("ghcr.io/o/app@sha256:{DIG_B}"), TS_AUG7_MS, "unreconciled"),
    );
    let iv_a2 = (Some(a), String::new());
    let iv_b2 = (Some(b), String::new());
    let page = aggregate_page(
        &[("alpha", "site-a", &latest_a, &iv_a2), ("bravo", "site-b", &silent_b, &iv_b2)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(drift.contains("how far is not statable"), "an unreconciled tip is not evidence");
}

#[test]
fn drift_distance_renders_only_on_differing_component_rows() {
    let g = grammar();
    // The mixed fixture: 1 component SKEW, 1 declared, 1 match, 1
    // present-in-one-only — and no capability distance ever.
    let (mut a, b) = drift_fixture_intents();
    a["capabilities"] = serde_json::json!([
        {"var": "cap_flip", "members": {"alpha_hosts": {"state": "resolved", "value": "true"}}}
    ]);
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    // Only the differing-component rows carry a distance line — here the SKEW
    // row and the declared split; the fixture's digests are not digest-shaped
    // ("new1"/"old9" are not hex), so even those honestly decline a distance.
    assert!(
        !drift.contains("drift-distance") || drift.matches("drift-distance").count() <= 2,
        "no distance line on capability, match or absent rows"
    );
}

// -- 3b: action-safety labels from the closed set ---------------------------

#[test]
fn remediation_pointers_carry_the_grammars_action_safety_label() {
    let g = grammar();
    // Repo vantage (the drift table): SKEW dispatches to `declare` — a
    // policy edit, and the label says so.
    let (a, b) = drift_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    assert!(
        drift.contains(r#"<span class="rem-safety safety-policy" title="a reviewed PR — changes no host">policy edit</span>"#),
        "the declare pointer is labelled as a policy edit, explanation on hover"
    );
    assert!(
        drift.contains("declare the difference with a reason (pins file, reviewed PR)"),
        "the default is one short action phrase"
    );

    // Live vantage (the node grid): SKEW dispatches to `reconverge` — the
    // engineering-baseline §2.2 idempotent re-run guarantee, said visibly.
    let mut fx = Fixture::new();
    let intent = testdata_generation("accountable-tail");
    let prior = intent["history"][0]["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &prior)], "zagros-sandbox", NOW_MS);
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let page = fx.render("zagros-sandbox");
    assert!(
        page.contains(r#"<span class="rem-safety safety-safe" title="safe: an idempotent re-run — the engineering-baseline §2.2 re-run-without-fear guarantee">safe</span>"#),
        "the reconverge pointer is labelled safe — the §2.2 re-run guarantee on hover"
    );
    assert!(page.contains("re-run the env&#39;s refresh play"), "one short action phrase");
}

#[test]
fn refusing_kinds_say_why_they_refuse_and_unknown_kinds_get_no_invented_label() {
    // Wire rows carrying every ADR 1060 dispatch class the live vantage can
    // emit — plus one unknown kind, which must get NO safety label (the
    // console does not invent safety claims).
    let mk_row = |name: &str, kind: &str, refuses: bool| ConsistencyRowV1 {
        coverage: "observed".into(),
        subject: "component".into(),
        name: name.into(),
        artifact_type: "x".into(),
        grade: "unaccountable".into(),
        color: "red".into(),
        why: format!("{name} why"),
        observed: "sha256:aaaa1111aaaa1111".into(),
        expected: "sha256:bbbb2222bbbb2222".into(),
        remediation_kind: kind.into(),
        remediation: format!("pointer for {name}"),
        remediation_refuses: refuses,
    };
    let env = EnvHealthV1 {
        domain: "consistency".into(),
        scope: "zagros-sandbox".into(),
        ts_unix_ms: NOW_MS,
        overall_color: "red".into(),
        sample_count: 1,
        window_secs: 30,
        detail: Some(EnvDetail::Consistency(ConsistencyDetailV1 {
            unreported_members: vec![],
            nodes: vec![NodeConsistencyV1 {
                coverage_gaps: 0,
                host_id: "zagros-a".into(),
                member_group: "zagros_hosts".into(),
                color: "red".into(),
                grade: "unaccountable".into(),
                intent_serial: 1,
                intent_content_hash: "sha256:cccc".into(),
                behind_by: 0,
                intent_age_seconds: 60,
                report_age_ms: 1000,
                intent_verified: true,
                rows: vec![
                    mk_row("img", "rollback", false),
                    mk_row("db", "restore", true),
                    mk_row("leaf", "rotate", true),
                    mk_row("fw", "refuse", true),
                    mk_row("odd", "mystery-kind", false),
                ],
                components_governed: 5,
                components_observed: 5,
                renders_declared: 0,
                renders_observed: 0,
            }],
            goatnet: "zagros".into(),
            current_serial: 1,
            current_content_hash: "sha256:cccc".into(),
            nodes_expected: 1,
            nodes_reporting: 1,
            skew_band_generations: 0,
            skew_band_seconds: 0,
            intent_readable: true,
            intent_note: String::new(),
        })),
    };
    let mut fx = Fixture::new();
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let page = fx.render("zagros-sandbox");

    // rollback: a non-refusing caution chip, explanation on hover, one phrase.
    assert!(page.contains(r#"<span class="rem-safety safety-caution" title="re-deploys the prior ledger generation">caution</span>"#));
    assert!(page.contains("roll back to the prior generation"));
    // The refusing kinds carry ONE safety signal — the REFUSES badge, with
    // the grammar's own reason on hover — plus their action phrase.
    assert!(page.contains(r#"<span class="refuses" title="rollback of data is a lie — restore instead">REFUSES</span>"#));
    assert!(page.contains(r#"<span class="refuses" title="rollback of cert is a lie — rotate re-issues in place">REFUSES</span>"#));
    assert!(page.contains(r#"<span class="refuses" title="forward-only: no undo path — converge forward">REFUSES</span>"#));
    assert_eq!(page.matches(">REFUSES</span>").count(), 3);
    // Exactly one non-refusing known kind (rollback) carries a safety chip;
    // the unknown kind carries none and no invented phrase — its pointer
    // renders in the long form, verbatim and visible.
    assert_eq!(page.matches(r#"class="rem-safety"#).count(), 1, "no invented label for an unknown kind");
    assert!(page.contains(r#"<span class="rem-kind">mystery-kind</span>"#));
    assert!(page.contains("pointer for odd"), "the unknown kind's pointer still renders verbatim");
}

// ---------------------------------------------------------------------------
// Drift-page polish (drift-page-polish): the operator's third
// pass, pinned. 1) Why lines lead with a plain CONCLUSION; the library's
// env=value enumeration sits behind the same expansion pattern as the
// digests, verbatim on the title and untouched on the API. 2) Remediation
// defaults to safety label + ONE action phrase; the exact command is one
// click away, verbatim + copyable. 3) EVERY grammar term the page renders
// has a human-first form with the grammar term on hover — a sweep through
// one table, not spot fixes. 4) A fixture render covering every row class,
// for the visual pass to iterate against.
// ---------------------------------------------------------------------------

#[test]
fn every_grammar_grade_has_a_human_first_form_with_the_term_on_hover() {
    let g = grammar();
    for grade in g.grade_names() {
        let (label, title) = render::grade_human(grade);
        assert!(!label.is_empty(), "{grade} renders a label");
        let prefix = format!("grammar grade: {grade} — ");
        assert!(title.starts_with(&prefix), "{grade}'s hover names the grammar term: {title}");
        assert!(title.len() > prefix.len(), "{grade}'s hover explains, not just names");
        assert!(
            !title.contains("not a grade this console"),
            "grammar grade '{grade}' fell through to the unknown-grade fallback — give it a human form"
        );
    }
    // The jargon-bearing terms translate to plain words; self-evident ones
    // may keep their word. The grammar itself is untouched — translation only.
    assert_eq!(render::grade_human("SKEW").0, "undeclared drift");
    assert_eq!(render::grade_human("unaccountable").0, "unexplained value");
    assert_eq!(render::grade_human("unreadable").0, "unreadable value");
    assert_eq!(render::grade_human("indeterminate").0, "no reading");
    // An unknown grade renders as itself with the honest fallback hover.
    let (label, title) = render::grade_human("mystery");
    assert_eq!(label, "mystery");
    assert!(title.contains("not a grade"));
}

/// The polish fixture: two comparable goatnets exercising every drift-row
/// class — matches, undeclared drift (with ledger-grounded direction),
/// declared-absent (the operator's example), declared-differing,
/// present-in-one-only — plus a stale goatnet and a refused intent.
fn polish_fixture_intents() -> (Value, Value) {
    let a = serde_json::json!({
        "goatnet": "alpha",
        "members": ["alpha_hosts"],
        "serial": 12,
        "content_hash": "sha256:aaaa1111aaaa1111",
        "generated_at": "2026-08-01T00:00:00Z",
        "components": [
            {"name": "zenoh-router", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/zenoh-router:sha-77ab001@sha256:77ab00177ab00177ab00177ab00177ab00177ab00177ab00177ab00177ab0017"}}},
            {"name": "postgres-spock", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": format!("ghcr.io/{{{{ ghcr_org }}}}/postgres-spock@sha256:{DIG_C}")}}},
            {"name": "app", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": format!("ghcr.io/{{{{ ghcr_org }}}}/app:sha-e68a6ae@sha256:{DIG_A}")}}},
            {"name": "console", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/console:sha-7c9a662@sha256:79d40ca79d40ca79d40ca79d40ca79d40ca79d40ca79d40ca79d40ca79d40ca7"}}},
            {"name": "fork-split", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/split:sha-11aa22b@sha256:11aa22b11aa22b11aa22b11aa22b11aa22b11aa22b11aa22b11aa22b11aa22b1",
                                "declared": {"reason": "alpha deliberately runs the fork build; upstream drops our patch. Re-evaluate at the next upstream release.", "scope": "alpha_hosts"}}}},
            {"name": "only-in-alpha", "artifact_type": "image", "members": {
                "alpha_hosts": {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/solo:sha-99ff00a@sha256:99ff00a99ff00a99ff00a99ff00a99ff00a99ff00a99ff00a99ff00a99ff00a9"}}}
        ],
        "capabilities": []
    });
    let b = serde_json::json!({
        "goatnet": "bravo",
        "members": ["bravo_hosts"],
        "serial": 9,
        "content_hash": "sha256:bbbb2222bbbb2222",
        "generated_at": "2026-08-01T00:00:00Z",
        "components": [
            {"name": "zenoh-router", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/zenoh-router:sha-77ab001@sha256:77ab00177ab00177ab00177ab00177ab00177ab00177ab00177ab00177ab0017"}}},
            {"name": "postgres-spock", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": format!("ghcr.io/{{{{ ghcr_org }}}}/postgres-spock@sha256:{DIG_C}")}}},
            {"name": "app", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": format!("ghcr.io/{{{{ ghcr_org }}}}/app:sha-a7e524c@sha256:{DIG_B}")}}},
            {"name": "console", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "absent",
                                "declared": {"reason": "single-homed by design; the dormant standby runs no route surfaces. Promote bravo before enabling any served console.", "scope": "bravo_hosts"}}}},
            {"name": "fork-split", "artifact_type": "image", "members": {
                "bravo_hosts": {"state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/split:sha-33cc44d@sha256:33cc44d33cc44d33cc44d33cc44d33cc44d33cc44d33cc44d33cc44d33cc44d3"}}}
        ],
        "capabilities": []
    });
    (a, b)
}

#[test]
fn why_lines_lead_with_the_conclusion_and_keep_the_enumeration_behind_expansion() {
    let g = grammar();
    let (a, b) = polish_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);

    // The operator's example, in shape: "<who> doesn't run <what> — <the
    // declarer's reason>", derived from the declaration. No "declared:"
    // prefix — the chip and the section heading already say it
    // (drift-page-grid vocabulary dedup: status ONCE per row).
    assert!(
        drift.contains("<summary>bravo doesn&#39;t run console — single-homed by design; the dormant standby runs no route surfaces</summary>"),
        "the declared-absent conclusion names who lacks it and why"
    );
    // The truncated reason's FULL text is in the expansion, not lost.
    assert!(
        drift.contains(r#"<div class="why-reason">single-homed by design; the dormant standby runs no route surfaces. Promote bravo before enabling any served console.</div>"#),
        "the full declaration reason sits behind the expansion"
    );
    // Declared-differing: the conclusion states the decision, not the values.
    assert!(
        drift.contains("<summary>the values deliberately differ — alpha deliberately runs the fork build; upstream drops our patch</summary>"),
    );
    // The status word appears ONCE per row — on the chip; no sentence
    // re-states it as a prefix.
    assert!(!drift.contains("<summary>declared"), "no 'declared:' sentence prefix anywhere");
    // Undeclared drift: plain words.
    assert!(drift.contains("<summary>the values differ, and nothing declares why</summary>"));
    // Present-in-one-only: not comparable, with the library's own detail.
    assert!(drift.contains("<summary>not comparable — not governed in bravo&#39;s intent artifact</summary>"));

    // The env=value enumeration renders ONLY behind the expansion: every
    // library sentence lives in a .why-full box, and no <summary> carries one.
    assert!(drift.contains(r#"<div class="why-full" title="#));
    assert!(drift.contains("bravo:bravo_hosts=absent"), "the enumeration is present, expanded");
    for (start, _) in drift.match_indices("<summary>") {
        let end = start + drift[start..].find("</summary>").unwrap();
        let summary = &drift[start..end];
        assert!(
            !summary.contains("_hosts="),
            "no env=value enumeration in a default-visible summary: {summary}"
        );
    }

    // Wire rows (the node grid) get the same conclusion-first shape from the
    // library clause: build one SKEW node row and check the summary.
    let mut fx = Fixture::new();
    let intent = testdata_generation("accountable-tail");
    let prior = intent["history"][0]["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(&intent, &[observed("zagros-a", "zagros_hosts", &prior)], "zagros-sandbox", NOW_MS);
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let node_page = fx.render("zagros-sandbox");
    assert!(
        node_page.contains("<summary>observed and expected differ, and nothing declares why</summary>"),
        "the node grid's why leads with the plain clause"
    );
}

#[test]
fn remediation_defaults_to_label_and_phrase_with_the_exact_command_behind_expansion() {
    let g = grammar();
    let (a, b) = polish_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);

    // Default = safety label + phrase; the command is behind the expansion.
    assert!(drift.contains(r#"<details class="rem-more"><summary>exact command</summary>"#));
    // Every verbatim pointer sits INSIDE a rem-more expansion: the pointer
    // code element only ever appears after an "exact command" summary.
    for (i, _) in drift.match_indices(r#"<code class="rem-ptr">"#) {
        let before = &drift[..i];
        let last_summary = before.rfind(r#"<details class="rem-more"><summary>exact command</summary>"#);
        assert!(
            last_summary.is_some() && before[last_summary.unwrap()..].matches("</details>").count() == 0,
            "a verbatim pointer rendered outside the exact-command expansion"
        );
    }
    // The pointer is still verbatim and copyable once expanded.
    assert!(drift.contains("deviations:"), "the declare pointer text is present, verbatim");
    assert!(drift.contains(r#"onclick="cp(this)""#), "one click copies the exact command");
}

/// The drift-table section flow: problems → declared → matches, each group
/// bounded by a heading row that states its count.
#[test]
fn the_drift_table_groups_problems_then_declared_then_matches() {
    let g = grammar();
    let (a, b) = polish_fixture_intents();
    let iv_a = (Some(a), String::new());
    let iv_b = (Some(b), String::new());
    let silent_a = Latest::default();
    let silent_b = Latest::default();
    let page = aggregate_page(
        &[("alpha", "site-a", &silent_a, &iv_a), ("bravo", "site-b", &silent_b, &iv_b)],
        &g,
    );
    let drift = drift_section(&page);
    // app=SKEW + only-in-alpha=indeterminate; console + fork-split declared;
    // zenoh-router + postgres-spock match.
    let p = drift.find("needs attention (2)").expect("problems group head");
    let d = drift.find("declared differences (2)").expect("declared group head");
    let m = drift.find("matches (2)").expect("matches group head");
    assert!(p < d && d < m, "problems → declared → matches");
}

// ---------------------------------------------------------------------------
// The PRODUCTION-SHAPED visual fixture: a fixture of 2 envs and short strings is
// unrepresentative, so this one has the real shape: THREE goatnet member columns,
// real component names and values, and real declaration reasons from
// ops/pins/fleet-pins.yaml + ops/pins/parity-capabilities.yaml. A SNAPSHOT, not a
// live read: the committed artifacts move with every pin bump, and a visual
// fixture's pinned counts must not.
// ---------------------------------------------------------------------------

/// Two goatnets — the aggregate page's shape, snapshotted.
/// 21 union rows: 9 declared (6 component absences + 3 capability
/// doors, each with its reason), 2 undeclared drifts (snitch_enabled +
/// node_net_health_enabled), 10 matches.
fn production_fixture_intents() -> (Value, Value) {
    // The real declaration reasons, verbatim from ops/pins/fleet-pins.yaml.
    const R_NB_ZAGROS: &str = "pre-existing; tighten at next bump";
    const R_NB_PROD: &str = "pre-existing fleet reality; fleet-wide digest tightening is its own value-change PR post-standup";
    const R_PORTAL: &str = r#"Admin mutation surfaces (portal + onboarding engine, id-console, netbird-dashboard) stay single-homed on zagros-a — ADR 1084 §D3. Beyond implying a service that isn't there, rendering portal_image into zagros_b_hosts would make it a "portal env" to ops/ci/check-portal-cert-identity-override.sh, which would then fail-closed on the six per-goatnet cert-identity keys zagros-b has no reason to set."#;
    const R_CONSOLE: &str = "The console is a served admin route surface; route surfaces stay single-homed on zagros-a and ABSENT on the dormant standby (ADR 1084 §D3; ADR 1111 dormancy `absent`, the inspection-surfaces class in ops/standby/service-classes.yaml). The consistency-health PUBLISHER above deliberately has no such deviation: it is standby-safe (reveal-only self-publish to zagros-b's own dormant router), and a dormant node's convergence is exactly what an operator needs to see BEFORE promoting it.";
    const R_IDC_ABSENT: &str = "Admin mutation surfaces (portal + onboarding engine, id-console, netbird-dashboard) stay single-homed on zagros-a — ADR 1084 §D3, the same reasoning as the portal row above.";
    const R_IDC_RC: &str = "RC (2026-08-10): F-309 — self-service pages refused every visitor. Deployed + verified here first; prod followed the same day by hotfix rather than soak (see below). Record 20260810-df992bf1e279.yaml";
    const R_IDC_HOTFIX: &str = "HOTFIX (2026-08-10, ack person-900@example.org): F-309 — the entire self-service surface is down on prod (reported there); fix is console-only and verified on zagros. Operator directed immediate deploy, no soak. Record 20260810-hotfix-df992bf1e279.yaml";
    const R_RUNNER: &str = "The onboarding engine is an admin mutation surface and stays single-homed on zagros-a — ADR 1084 §D3, the same reasoning as the id-admin-console row above.";
    const R_AUDIT: &str = "zagros-b carries no audit-subscriber hand pin today and runs none — the durable-write audit consumer is single-homed beside its MinIO sink on zagros-a (ADR 1084 §D3 single-homed class). Rendering a new var into the dormant standby's pins would be a rendered-value change, not a migration no-op.";
    const R_MICRO: &str = "Inspection surfaces stay single-homed on zagros-a and ABSENT on the dormant standby (ADR 1084 §D3, the inspection-surfaces class in ops/standby/service-classes.yaml) — same reasoning as consistency-console above; zagros-b holds no microscope pin today.";
    // …and from ops/pins/parity-capabilities.yaml.
    const R_DOOR: &str = r#"zagros-b does not run goat-id-console at all (id_admin_console_enabled is not true there; the console is single-homed on zagros-a, ADR 1084 §D3), so it resolves this key from the role default and appears to disagree with zagros-a's declared "". That is a component it does not have, not a door it chose differently. Declared so the row reads as "not applicable here" rather than as skew — and so that if zagros-b ever DOES stand the console up, removing this stanza is the step that forces a conscious decision about its door instead of letting it inherit the restrictive default. Presence on the members that DO run the console is enforced by ops/ci/check-console-door-declared.sh, which derives its list from id_admin_console_enabled rather than from this file."#;
    const R_SNAPSHOT: &str = "Dormant-deferred. The DNS-down name-fallback resolver (ADR 1082) needs the mesh_dns_* config keys — bundles carry no nameservers (name-fallback-resolver.md §9) — which land with zagros-b's later serving-surface phase (P2/P3 of the full-standby build). Declared false with a reason now so the skew is intentional, not a missed second-node key; revisit when the mesh_dns_* keys are set.";
    const R_ISITUP: &str = "Single-homed on zagros-a while zagros-b is a DORMANT standby. isitup publishes per-service DNS + health records; on a dormant node that would advertise zagros-b service names that must not resolve to it until promotion (the F-197 declared-but-dark shape). The service-DNS/health vantage stays on zagros-a; zagros-b's isitup is started as part of the promotion runbook, alongside the other dormant serving surfaces.";

    // The real pinned values (ADR-length refs, templated org, full digests).
    const NB: &str = "ghcr.io/{{ ghcr_org }}/netbird-mgmt:latest";
    const PG: &str = "ghcr.io/{{ ghcr_org }}/postgres-spock@sha256:038f12f0e5339b2722754c06273a2277fcc4e4e17056c8afd2f52f28ed16e56c";
    const SIG: &str = "ghcr.io/{{ ghcr_org }}/signal-server@sha256:3a26d0e8884e90edcf8dfc0fba5261a16ce45c7e585d9ca325a0913d9029241d";
    const ZR: &str = "ghcr.io/{{ ghcr_org }}/zenoh-pqc-fips@sha256:f3a16fdfda03cb6534c7ca27f2e120dd2f7e33c3cc8072f1c6a44643d9112f48";
    const PORTAL: &str = "ghcr.io/{{ ghcr_org }}/goat-zenoh-portal@sha256:b8a3da7b3ddbd87794aab5bb26639bf182b754d0319bc6c1800e114e75459a1d";
    const CH: &str = "ghcr.io/{{ ghcr_org }}/goat-consistency-health:sha-7c9a662@sha256:09d330069d1d812b06fcf1bfebc3555ac43e80ef14bd49e12988aa0ac02ac718";
    const CC: &str = "ghcr.io/{{ ghcr_org }}/goat-consistency-console:sha-7c9a662@sha256:79d40ca11dcc108a5a516d40caf797690e96c8640221a1434c66cdaaa3f1da8f";
    const IDC: &str = "ghcr.io/{{ ghcr_org }}/goat-id-console@sha256:60401f3cff3e84d1de92735294ed70a818094f8c267284a1f427cda59fc517c8";
    const RUNNER: &str = "ghcr.io/{{ ghcr_org }}/onboard-runner@sha256:12cb6a42fed11397e7e31fb376be9b1fe7454d27895afe50d1634ac1bfe23929";
    const AUDIT: &str = "ghcr.io/{{ ghcr_org }}/goat-audit-subscriber@sha256:6aacd50c4f6a8937e1750374a4e0b9a1150ddffe66a81d37fdb77c9e1ff49ae5";
    const MICRO: &str = "ghcr.io/{{ ghcr_org }}/microscope@sha256:8a5462ffbc6c766373d6771dcd36e4ebec92453c2a9b187fbf1db9b19b815364";

    let both = |v: &str| {
        serde_json::json!({
            "zagros_hosts": {"state": "resolved", "value": v},
            "zagros_b_hosts": {"state": "resolved", "value": v},
        })
    };
    let a_only = |v: &str, reason: &str| {
        serde_json::json!({
            "zagros_hosts": {"state": "resolved", "value": v},
            "zagros_b_hosts": {"state": "absent",
                             "declared": {"reason": reason, "scope": "zagros_b_hosts"}},
        })
    };
    let cap_both = |v: &str| both(v);
    // The capabilities declaredness shape is ROW-LEVEL (`declared:` keyed by
    // member), exactly as a goatnet's ops/consistency/intent/<site>/intent.json carries it.
    let cap_split = |a: &str, b: &str| {
        serde_json::json!({
            "zagros_hosts": {"state": "resolved", "value": a},
            "zagros_b_hosts": {"state": "resolved", "value": b},
        })
    };
    let row_declared = |reason: &str| serde_json::json!({"zagros_b_hosts": {"reason": reason}});
    let mut idc_members = a_only(IDC, R_IDC_ABSENT);
    idc_members["zagros_hosts"]["declared"] =
        serde_json::json!({"reason": R_IDC_RC, "scope": "zagros_hosts"});
    let mut nb_members = both(NB);
    for m in ["zagros_hosts", "zagros_b_hosts"] {
        nb_members[m]["declared"] = serde_json::json!({"reason": R_NB_ZAGROS, "scope": m});
    }

    let zagros = serde_json::json!({
        "goatnet": "zagros",
        "members": ["zagros_hosts", "zagros_b_hosts"],
        "serial": 48,
        "content_hash": "sha256:d3f1df485aac6b068addb76ad8bb9b524d99156909bedf14411c07d84ad36112",
        "generated_at": "2026-08-10T13:52:31Z",
        "components": [
            {"name": "netbird-mgmt", "artifact_type": "image", "members": nb_members},
            {"name": "postgres-spock", "artifact_type": "image", "members": both(PG)},
            {"name": "signal-server", "artifact_type": "image", "members": both(SIG)},
            {"name": "zenoh-router", "artifact_type": "image", "members": both(ZR)},
            {"name": "portal", "artifact_type": "image", "members": a_only(PORTAL, R_PORTAL)},
            {"name": "consistency-health", "artifact_type": "image", "members": both(CH)},
            {"name": "consistency-console", "artifact_type": "image", "members": a_only(CC, R_CONSOLE)},
            {"name": "id-admin-console", "artifact_type": "image", "members": idc_members},
            {"name": "onboard-runner", "artifact_type": "image", "members": a_only(RUNNER, R_RUNNER)},
            {"name": "audit-subscriber", "artifact_type": "image", "members": a_only(AUDIT, R_AUDIT)},
            {"name": "microscope", "artifact_type": "image", "members": a_only(MICRO, R_MICRO)},
        ],
        "capabilities": [
            {"var": "id_admin_console_oauth2_proxy_allowed_groups",
             "members": cap_split("", "PLATFORM-ADMIN,IAM-ADMIN"),
             "declared": row_declared(R_DOOR)},
            {"var": "rosenpass_enabled", "members": cap_both("true")},
            {"var": "name_snapshot_enabled", "members": cap_split("true", "false"),
             "declared": row_declared(R_SNAPSHOT)},
            {"var": "netbird_selfheal_enabled", "members": cap_both("true")},
            {"var": "ory_spock_enabled", "members": cap_both("true")},
            {"var": "snitch_enabled", "members": cap_both("false")},
            {"var": "isitup_enabled", "members": cap_split("true", "false"),
             "declared": row_declared(R_ISITUP)},
            {"var": "node_net_health_enabled", "members": cap_both("true")},
            {"var": "host_net_publish_enabled", "members": cap_both("true")},
            {"var": "oci_t2_sync_enabled", "members": cap_both("false")},
        ]
    });

    let prod_c = |v: &str| serde_json::json!({"zagros_prod_hosts": {"state": "resolved", "value": v}});
    let mut idc_prod = prod_c(IDC);
    idc_prod["zagros_prod_hosts"]["declared"] =
        serde_json::json!({"reason": R_IDC_HOTFIX, "scope": "zagros_prod_hosts"});
    let mut nb_prod = prod_c(NB);
    nb_prod["zagros_prod_hosts"]["declared"] =
        serde_json::json!({"reason": R_NB_PROD, "scope": "zagros_prod_hosts"});

    let zagros_prod = serde_json::json!({
        "goatnet": "zagros_prod",
        "members": ["zagros_prod_hosts"],
        "serial": 51,
        "content_hash": "sha256:1614116c0e6d955eabc4698024190f11837c3af0afd55b609a05bdab98fc0c97",
        "generated_at": "2026-08-10T14:15:17Z",
        "components": [
            {"name": "netbird-mgmt", "artifact_type": "image", "members": nb_prod},
            {"name": "postgres-spock", "artifact_type": "image", "members": prod_c(PG)},
            {"name": "signal-server", "artifact_type": "image", "members": prod_c(SIG)},
            {"name": "zenoh-router", "artifact_type": "image", "members": prod_c(ZR)},
            {"name": "portal", "artifact_type": "image", "members": prod_c(PORTAL)},
            {"name": "consistency-health", "artifact_type": "image", "members": prod_c(CH)},
            {"name": "consistency-console", "artifact_type": "image", "members": prod_c(CC)},
            {"name": "id-admin-console", "artifact_type": "image", "members": idc_prod},
            {"name": "onboard-runner", "artifact_type": "image", "members": prod_c(RUNNER)},
            {"name": "audit-subscriber", "artifact_type": "image", "members": prod_c(AUDIT)},
            {"name": "microscope", "artifact_type": "image", "members": prod_c(MICRO)},
        ],
        "capabilities": [
            {"var": "id_admin_console_oauth2_proxy_allowed_groups", "members": prod_c("")},
            {"var": "rosenpass_enabled", "members": prod_c("true")},
            {"var": "name_snapshot_enabled", "members": prod_c("true")},
            {"var": "netbird_selfheal_enabled", "members": prod_c("true")},
            {"var": "ory_spock_enabled", "members": prod_c("true")},
            // Two live skews: snitch runs on one goatnet only,
            // the net-health publisher on the other only — neither declared.
            {"var": "snitch_enabled", "members": prod_c("true")},
            {"var": "isitup_enabled", "members": prod_c("true")},
            {"var": "node_net_health_enabled", "members": prod_c("false")},
            {"var": "host_net_publish_enabled", "members": prod_c("true")},
            {"var": "oci_t2_sync_enabled", "members": prod_c("false")},
        ]
    });

    (zagros, zagros_prod)
}

/// The full fixture render for the visual pass, at the LIVE page's shape.
/// Pins the shape so it can't regress to the toy fixture: 3+ env columns,
/// 9+ declared rows behind one-line disclosures, exactly the 2 real drift
/// rows expanded. Set GCC_FIXTURE_DIR to write aggregate.html + single.html
/// for a browser; the structural checks run either way.
#[test]
fn the_visual_fixture_is_production_shaped_and_pins_the_column_count() {
    let g = grammar();
    let (zagros, zagros_prod) = production_fixture_intents();
    let iv_zagros = (Some(zagros), String::new());
    let iv_prod = (Some(zagros_prod), String::new());

    // Live rollups on both fabrics, from the corpus generation (the node
    // grids' shape is the corpus's business; the drift table's shape — the
    // round-4 subject — is the fixture intents' above).
    let tail = testdata_generation("accountable-tail");
    let current = tail["content_hash"].as_str().unwrap().to_string();
    let prior = tail["history"][0]["content_hash"].as_str().unwrap().to_string();
    let mut latest_zagros = Latest::default();
    latest_zagros.rollups.insert(
        "zagros-sandbox".into(),
        StoredEnvelope {
            env: envelope_from(
                &tail,
                &[observed("zagros-b", "zagros_b_hosts", &prior)],
                "zagros-sandbox",
                NOW_MS,
            ),
            received_unix_ms: NOW_MS,
        },
    );
    let mut latest_prod = Latest::default();
    latest_prod.rollups.insert(
        "zagros-prod".into(),
        StoredEnvelope {
            env: envelope_from(
                &tail,
                &[observed("mgmt-zagros-prod-01-orin", "zagros_prod_hosts", &current)],
                "zagros-prod",
                NOW_MS,
            ),
            received_unix_ms: NOW_MS,
        },
    );

    let page = aggregate_page(
        &[
            ("zagros", "zagros-sandbox", &latest_zagros, &iv_zagros),
            ("zagros-prod", "zagros-prod", &latest_prod, &iv_prod),
        ],
        &g,
    );
    let drift = drift_section(&page);

    // THE regression pin: the drift table carries the live page's THREE
    // goatnet member columns — never again the toy 2-column shape.
    assert_eq!(
        drift.matches(r#"class="gh gh-env mono""#).count(),
        3,
        "the fixture must carry the live page's 3 member columns"
    );
    for col in ["zagros-prod:zagros_prod_hosts", "zagros:zagros_b_hosts", "zagros:zagros_hosts"] {
        assert!(
            drift.contains(&format!(r#"title="{col}""#)),
            "column {col} is present with its full identity on hover"
        );
    }
    assert!(drift.contains(r#"<div class="grid" style="--envn:3">"#));

    // The live shape's counts, pinned: 21 rows, 2 real drifts, 9 declared.
    assert!(drift.contains("21 rows compared across zagros + zagros-prod"));
    assert!(drift.contains("2 undeclared drift"));
    assert!(drift.contains("needs attention (2)"));
    assert!(drift.contains("declared differences (9)"));
    assert!(drift.contains("matches (10)"));

    // Density tiers at this shape: exactly the 2 drift rows render expanded;
    // all 9 declared rows are one-line disclosures (≈9 lines, not ~36).
    assert_eq!(drift.matches(r#"<div class="grow has-detail sev-"#).count(), 2);
    assert_eq!(drift.matches(r#"<details class="grow gdecl sev-"#).count(), 9);
    // The real skews, named in plain words — who differs, no invented status.
    assert!(drift.contains("zagros-prod differs from the rest, and nothing declares why"));

    // The ADR-length declared reasons render VERBATIM (behind the row's
    // disclosure + the sentence's expansion), and the declared-absent
    // conclusion names who lacks the thing with the declarer's own lead.
    assert!(drift.contains("zagros/zagros-b doesn&#39;t run portal — Admin mutation surfaces"));
    assert!(drift.contains("ADR 1084 §D3"), "the ADR reference survives verbatim");
    assert!(drift.contains("Dormant-deferred"), "the capability reasons render");
    // Vocabulary dedup at this shape: an absent cell is a single dimmed word.
    assert!(page.contains(r#"<span class="gval mono"><span class="dim">absent</span></span>"#));
    assert!(!page.contains("absent (declared)"));
    assert!(!page.contains("[declared]"));

    // The single-mode page shares the code path — one goatnet's own console.
    let single = {
        let fx = Fixture {
            latest: latest_zagros,
            intent: iv_zagros.clone(),
            siblings: Siblings::default(),
            grammar: grammar(),
        };
        fx.render("zagros-sandbox")
    };
    assert!(single.contains("live rollup"));
    assert!(single.contains("committed-intent grid"));

    if let Ok(dir) = std::env::var("GCC_FIXTURE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aggregate.html"), &page).unwrap();
        std::fs::write(dir.join("single.html"), &single).unwrap();
        eprintln!("fixture pages written to {}", dir.display());
    }
}

// ---------------------------------------------------------------------------
// Observed generation + declaration expiry: WHICH generation the verdicts were
// computed for, and whether a `declared` row is still declared today.
// ---------------------------------------------------------------------------

#[test]
fn the_drift_panel_names_the_generation_each_verdict_was_computed_for() {
    let g = grammar();
    let (a, b) = drift_fixture_intents();
    let (iv_a, iv_b) = ((Some(a), String::new()), (Some(b), String::new()));
    let (sa, sb) = (Latest::default(), Latest::default());
    let page =
        aggregate_page(&[("alpha", "site-a", &sa, &iv_a), ("bravo", "site-b", &sb, &iv_b)], &g);

    // The serial is named per env, beside its age — the observedGeneration
    // idea. A reader can tell this page apart from one rendered off a
    // different generation.
    assert!(page.contains("alpha serial 3"), "each env's generation is named: {page}");
    assert!(page.contains("bravo serial 9"));
    assert!(page.contains("computed for"), "the line says what it is");

    // Age is stated, and an artifact older than a day wears the page's own
    // amber chip rather than reading as ordinary meta text. The fixture is
    // generated against a clock.
    assert!(page.contains("3d ago"), "the age is stated, not implied");
    assert!(
        page.contains(r#"class="chip chip-amber gen-stale""#),
        "a stale generation is flagged, not merely printed"
    );

    // And the comparison says which day it was dated for, because that is what
    // decides whether a declaration has expired.
    assert!(page.contains("dated 2026-08-04"));
}

#[test]
fn an_expired_declaration_stops_covering_its_row_on_the_page() {
    let g = grammar();
    let (mut a, b) = drift_fixture_intents();
    // `declared-split` is the one declared row in the fixture. Give its
    // declaration a date that has already passed at NOW_MS.
    a["components"][2]["members"]["alpha_hosts"]["declared"] = serde_json::json!({
        "reason": "alpha runs the fork on purpose",
        "scope": "alpha_hosts",
        "until": "2026-07-01",
    });
    let (iv_a, iv_b) = ((Some(a.clone()), String::new()), (Some(b.clone()), String::new()));
    let (sa, sb) = (Latest::default(), Latest::default());
    let page =
        aggregate_page(&[("alpha", "site-a", &sa, &iv_a), ("bravo", "site-b", &sb, &iv_b)], &g);

    // It has left the declared bucket and joined the drift signal — the whole
    // point of `until:`, which was rendered but never compared to a clock.
    assert!(page.contains("2 undeclared drift"), "the expired row is drift now: {page}");
    assert!(!page.contains("1 declared difference"), "and no longer declared");
    // The page says WHY it moved, naming the column and the date — a row that
    // silently went amber would be worse than one that never expired.
    assert!(page.contains("EXPIRED"));
    assert!(page.contains("2026-07-01"));

    // A declaration whose date has NOT passed is untouched.
    let mut a2 = a;
    a2["components"][2]["members"]["alpha_hosts"]["declared"]["until"] =
        serde_json::json!("2026-12-31");
    let iv_a2 = (Some(a2), String::new());
    let later =
        aggregate_page(&[("alpha", "site-a", &sa, &iv_a2), ("bravo", "site-b", &sb, &iv_b)], &g);
    assert!(later.contains("1 declared difference"));
    assert!(!later.contains("EXPIRED"));
}

#[test]
fn the_cross_env_api_carries_the_generations_it_was_computed_for() {
    let g = grammar();
    let siblings = Siblings::default();
    let (a, b) = drift_fixture_intents();
    let (iv_a, iv_b) = ((Some(a), String::new()), (Some(b), String::new()));
    let (sa, sb) = (Latest::default(), Latest::default());
    let envs = [
        api_env("alpha", "tcp/1:1", "site-a", &sa, &iv_a, &g, &siblings, NOW_MS),
        api_env("bravo", "tcp/1:2", "site-b", &sb, &iv_b, &g, &siblings, NOW_MS),
    ];
    let doc = api::cross_env_doc(&envs, NOW_MS);

    assert_eq!(doc["comparable"], true);
    assert_eq!(doc["dated_for"], "2026-08-04");
    let obs = doc["observed_generations"].as_array().expect("machine-shaped, not page-only");
    assert_eq!(obs.len(), 2);
    assert_eq!(obs[0]["env"], "alpha");
    assert_eq!(obs[0]["serial"], 3);
    assert_eq!(obs[0]["content_hash"], "sha256:af1");
    assert_eq!(obs[0]["generated_at"], "2026-08-01T00:00:00Z");
    assert_eq!(obs[0]["age_s"], 3 * 86_400, "age is stated in the API too, never implied");
    assert_eq!(obs[1]["serial"], 9);
}

// ---------------------------------------------------------------------------
// Coverage rows (ADR 1125 RJ-1). The rows exist in the library; what is pinned
// here is that the page SHOWS them — the whole failure this replaced was a gap
// nobody could see.
// ---------------------------------------------------------------------------

#[test]
fn coverage_gaps_render_as_rows_with_their_state_named() {
    let mut fx = Fixture::new();
    // A node holding the CURRENT generation and reporting no component tips:
    // its sync verdict is clean and every governed component is a coverage
    // gap. The corpus generations carry no components, so the fixture declares
    // its own — one ledger-covered, one declared `ledger_component: null`.
    let mut intent = testdata_generation("accountable-tail");
    intent["components"] = serde_json::json!([
        {"name": "portal", "artifact_type": "image", "members": {
            "zagros_hosts": {"state": "resolved", "value": "ghcr.io/x/portal@sha256:aaaa"}}},
        {"name": "hand-deployed", "artifact_type": "image", "ledger_component": null,
         "members": {
            "zagros_hosts": {"state": "resolved", "value": "ghcr.io/x/hand@sha256:bbbb"}}}
    ]);
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(
        &intent,
        &[observed("zagros-a", "zagros_hosts", &current)],
        "zagros-sandbox",
        NOW_MS,
    );
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let page = fx.render("zagros-sandbox");

    let node_start = page.find(r#"<article class="node">"#).expect("the node grid renders");
    let node_end = node_start + page[node_start..].find("</article>").unwrap();
    let node = &page[node_start..node_end];

    // The state is NAMED on the row, in the grammar's own word — not implied
    // by a missing row, which is what it replaced.
    assert!(node.contains("cov-badge"), "coverage rows carry a badge: {node}");
    assert!(node.contains(">silent<"), "a ledger-covered component nobody reported");
    assert!(
        node.contains(">no-instrument<"),
        "and a component nothing can ever observe is a DIFFERENT state, not the same absence"
    );
    // The count is stated beside the other denominators.
    assert!(node.contains("coverage gap(s)"), "the count rides the grounding line");
    // And the pointer is the coverage one, rendered verbatim like every other.
    assert!(node.contains("rem-ptr"));
}

#[test]
fn a_coverage_badge_never_appears_on_a_real_comparison_and_zero_is_stated() {
    // `observed` gets no badge: the badge exists to mark the rows that are NOT
    // comparisons, and badging the ordinary case makes the exception invisible
    // again — which is the failure mode, not a style preference.
    assert_eq!(render::coverage_badge_for_test("observed"), "");
    assert_eq!(render::coverage_badge_for_test(""), "", "an older publisher's empty value is not a state");
    assert!(render::coverage_badge_for_test("silent").contains("silent"));
    // An unknown state renders VERBATIM rather than being swallowed: what a
    // publisher says is a fact about the publisher.
    let unknown = render::coverage_badge_for_test("invented-by-someone");
    assert!(unknown.contains("invented-by-someone"));
    assert!(unknown.contains("unknown to this console"));

    // Zero gaps is STATED, never omitted — a category that vanishes when empty
    // reads as a category that does not exist.
    assert!(render::coverage_summary_for_test(0).contains("no coverage gaps"));
    assert!(render::coverage_summary_for_test(3).contains("3 coverage gap(s)"));
}

#[test]
fn a_governed_member_no_node_claims_is_named_on_the_page_not_left_to_subtraction() {
    let mut fx = Fixture::new();
    // Two governed members, one reporting. `1 of 2` is already rendered; what
    // this pins is that the page says WHICH one is missing — the F-324 lesson,
    // where a governed host nobody looked at was invisible inside a ratio.
    let mut intent = testdata_generation("accountable-tail");
    intent["members"] = serde_json::json!(["zagros_hosts", "zagros_b_hosts"]);
    let current = intent["content_hash"].as_str().unwrap().to_string();
    let env = envelope_from(
        &intent,
        &[observed("zagros-a", "zagros_hosts", &current)],
        "zagros-sandbox",
        NOW_MS,
    );
    fx.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env, received_unix_ms: NOW_MS });
    let page = fx.render("zagros-sandbox");

    assert!(page.contains("governed and unreported"), "the state is named: {page}");
    assert!(page.contains("zagros_b_hosts"), "and the MEMBER is named, not just counted");
    assert!(
        page.contains("NO node claimed"),
        "and it is distinguished from a slow node — nothing reports for it at all"
    );
    // The pointer is the grammar's `enrol`, printed verbatim — the state
    // declared one and nothing emitted it until this landed.
    assert!(page.contains("rem-ptr"), "the coverage state carries its pointer");
    assert!(page.contains("no node claims member"), "and it is the grammar's words: {page}");

    // With every member reporting, the line does not render at all: a
    // permanent zero-state banner is noise that teaches the eye to skip it.
    let mut fx2 = Fixture::new();
    let mut both = testdata_generation("accountable-tail");
    both["members"] = serde_json::json!(["zagros_hosts"]);
    let cur2 = both["content_hash"].as_str().unwrap().to_string();
    let env2 = envelope_from(&both, &[observed("zagros-a", "zagros_hosts", &cur2)], "zagros-sandbox", NOW_MS);
    fx2.latest.rollups.insert("zagros-sandbox".into(), StoredEnvelope { env: env2, received_unix_ms: NOW_MS });
    assert!(!fx2.render("zagros-sandbox").contains("governed and unreported"));
}

// ---------------------------------------------------------------------------
// One surface, three URLs. The same binary is deployed per-site
// and once more in aggregate mode, and until now none of them linked to the
// others — so an operator had to know which URL answered which question. The
// mechanism for this (Block 94 siblings) already existed and was wired to
// panoscope and stethoscope but not between the consistency pages themselves.
// ---------------------------------------------------------------------------

#[test]
fn the_site_page_links_to_the_cross_env_page_and_hides_the_link_when_unset() {
    let g = grammar();
    let intent = testdata_generation("accountable-tail");
    let iv = (Some(intent), String::new());
    let latest = Latest::default();
    let siblings = Siblings {
        crossenv: Some("https://consistency.zagros-prod.goat.example".into()),
        ..Default::default()
    };
    let v = render::View {
        chosen_site: "zagros-sandbox", latest: &latest, intent: &iv, siblings: &siblings,
        grammar: &g, now_ms: NOW_MS, stale_after_ms: 900_000, version: "test",
    };
    let page = render::render_page(&v);
    assert!(page.contains("https://consistency.zagros-prod.goat.example"), "the link renders");
    assert!(page.contains("cross-env drift"), "and says what it is");

    // Hidden, never dead — the §7.7 rule. An unset sibling must not emit an
    // empty href that 404s the operator.
    let none = Siblings::default();
    let v2 = render::View { siblings: &none, ..v };
    let page2 = render::render_page(&v2);
    assert!(!page2.contains("cross-env drift &rarr;"));
    assert!(!page2.contains("href=\"\""), "an unset sibling must hide, not emit a dangling href");
}

#[test]
fn the_cross_env_page_links_back_to_each_site_console_and_says_why() {
    let g = grammar();
    let (a, b) = drift_fixture_intents();
    let (iv_a, iv_b) = ((Some(a), String::new()), (Some(b), String::new()));
    let (sa, sb) = (Latest::default(), Latest::default());
    let page =
        aggregate_page(&[("alpha", "site-a", &sa, &iv_a), ("bravo", "site-b", &sb, &iv_b)], &g);
    // aggregate_page's fixture sets console_url on the first goatnet only.
    assert!(page.contains("https://consistency.zagros.netbird.zagros.example"));
    assert!(page.contains("site console"), "the link is labelled");
    // The REASON is on the link, because "why click through to a subset of
    // this page" is the obvious question and the answer is island mode.
    assert!(page.contains("island mode"), "the link says why it exists: {page}");
    // The goatnet with no console_url gets no link — one per configured URL,
    // not one per goatnet.
    assert_eq!(page.matches("site console &#8599;").count(), 1);
}
