//! `survey`: how consistent the software is inside each goatnet and across the
//! goatnets of the release ladder, in one read-only pass. Design of record:
//! docs/design/goatbuilder.md §"Verbs" (`survey`); status in
//! goatbuilder-conformance.md §"What is built".
//!
//! Inside a goatnet (intra): main's declared ref against each host's newest
//! ledger row (the read `diff` does, one Ansible call per member group), hosts
//! of one goatnet that run different refs of one component, the goat CLI each
//! host runs against the one its goatnet serves, and the release tags a
//! goat-get host serves against the tags main declares.
//!
//! Across goatnets (inter): one row per component, one column per goatnet, with
//! the ladder's own grade for each declared difference (`promote status`):
//! `candidate` and `hotfix-backfill-owed` are declared and in flight;
//! `UNDECLARED-SKEW` is a difference nothing declares.
//!
//! Every goatnet is read in parallel. A host or goatnet that cannot be read is
//! unknown, never consistent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::converger::Converger;
use crate::inspect::{self, Drift, HostFacts};
use crate::{promote, verbs, OutputFormat, Report, EXIT_NOT_CONVERGED, EXIT_OK};

pub const SCHEMA: &str = "goatbuilder.survey/v1";

/// The wall-time target for the three ladder goatnets read in parallel, doors
/// included. The measured time is printed beside it on every run.
pub const BUDGET_SECS: u64 = 90;

pub struct Options<'a> {
    pub envs: &'a [String],
    pub all: bool,
    pub intra: bool,
    pub inter: bool,
    pub html: Option<&'a Path>,
    pub output: OutputFormat,
}

/// One goatnet as it was read.
#[derive(Debug, Clone, Serialize, Default)]
pub struct GoatnetRead {
    pub goatnet: String,
    pub rung: Option<String>,
    pub transport: String,
    /// Set when the goatnet could not be read at all.
    pub error: Option<String>,
    pub rows: Vec<Drift>,
    pub hosts: BTreeMap<String, HostFacts>,
    /// What main declares for the payloads goat-get serves, per member group:
    /// group → (`goat-cli` | `goatbuilder`) → release tag. A payload a group
    /// does not serve is not listed.
    pub declared_served: BTreeMap<String, BTreeMap<String, String>>,
    /// Components the pins place here that have no deploy play goatbuilder can
    /// run (no `<name>-deploy.yml`, no `deploy_playbook:`).
    pub no_play: BTreeSet<String>,
    pub elapsed_secs: f64,
}

/// The ladder's view of one component: the value each rung declares and the
/// grade of each hop (`promote status`).
#[derive(Debug, Clone, Serialize, Default)]
pub struct LadderRow {
    pub component: String,
    pub grade: String,
    /// rung → declared value (None: absent on that rung)
    pub declared: BTreeMap<String, Option<String>>,
    /// "dev->preprod" → grade
    pub pairs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Finding {
    /// intra | inter
    pub scope: String,
    /// drift | undeclared-drift | hosts-disagree | unknown | served-payload |
    /// goat-cli-host | undeclared-skew | prod-differs | unread
    pub kind: String,
    /// drift (something differs that nothing declares) | unknown (could not be
    /// read) | declared (a difference the ladder carries, in flight)
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goatnet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Cell {
    /// The declared ref, shortened (`portal:v1.4.0@sha256:0b2a6c1d3e4f`).
    pub declared: String,
    /// same | drift | mixed | partial (some hosts unread, the rest same) |
    /// unknown | no-coverage | absent
    pub state: String,
    pub hosts: usize,
    /// hosts whose ledger or containers answered
    pub read: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MatrixRow {
    pub component: String,
    pub ladder_grade: String,
    /// goatnet → cell
    pub cells: BTreeMap<String, Cell>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Summary {
    pub goatnets: usize,
    pub components: usize,
    pub drift: usize,
    pub unknown: usize,
    pub declared_in_flight: usize,
    /// components whose prod declaration differs from dev's
    pub prod_differs_from_dev: usize,
    /// prod hosts not running what prod declares
    pub prod_running_off_declared: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Survey {
    pub schema: &'static str,
    pub generated_at: String,
    pub budget_secs: u64,
    pub elapsed_secs: f64,
    /// Goatnets in ladder order (dev first).
    pub order: Vec<String>,
    pub summary: Summary,
    pub findings: Vec<Finding>,
    pub matrix: Vec<MatrixRow>,
    pub goatnets: Vec<GoatnetRead>,
}

fn f(scope: &str, kind: &str, severity: &str, goatnet: Option<&str>, component: Option<&str>, detail: String, fix: Option<String>) -> Finding {
    Finding {
        scope: scope.into(),
        kind: kind.into(),
        severity: severity.into(),
        goatnet: goatnet.map(String::from),
        component: component.map(String::from),
        detail,
        fix,
    }
}

fn name(g: &str) -> String {
    g.replace('_', "-")
}

/// `goat-cli-v0.5.14` / `goat 0.5.14` / `0.5.14` → `0.5.14`.
fn version_of(s: &str) -> String {
    let t = s.trim();
    let t = t.rsplit('/').next().unwrap_or(t);
    let t = t.trim_start_matches("goat-cli-v").trim_start_matches("goat ").trim_start_matches('v');
    t.split_whitespace().next().unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// analysis: pure, so the fixture and the unit tests drive it directly
// ---------------------------------------------------------------------------

/// What a host's containers say about one declared image.
#[derive(Debug, Clone, PartialEq)]
enum Ctr {
    /// docker could not be read on the host
    Unread,
    /// no container runs that image
    None,
    /// every container from that image runs the declared digest (their names)
    Same(String),
    /// some container from that image runs another digest (what runs)
    Differs(String),
}

impl Ctr {
    fn note(&self) -> String {
        match self {
            Ctr::Same(n) => format!(" (yet {n} runs the declared digest)"),
            Ctr::Differs(n) => format!(" ({n})"),
            _ => String::new(),
        }
    }
}

/// The digests a container runs: its registry digests for its own image name,
/// else the digest in the ref it was created from.
fn container_digests(c: &inspect::Container, name: &str) -> Vec<String> {
    let mut v: Vec<String> = c.repo_digests.iter().filter(|r| inspect::image_name(r) == name).map(|r| inspect::digest(r).to_string()).collect();
    if v.is_empty() && c.image.contains("@sha256:") {
        v.push(inspect::digest(&c.image).to_string());
    }
    v
}

/// `container management-a`, or `container management-b (exited)` when it is not running.
fn label(c: &inspect::Container) -> String {
    if c.state.is_empty() || c.state == "running" { format!("container {}", c.name) } else { format!("container {} ({})", c.name, c.state) }
}

fn container_view(h: Option<&HostFacts>, declared: &str) -> Ctr {
    // A release component is a file: no container answers for it, only its ledger.
    if crate::pins::is_release_ref(declared) {
        return Ctr::Unread;
    }
    let Some(cs) = h.and_then(|h| h.containers.as_ref()) else { return Ctr::Unread };
    let name = inspect::image_name(declared);
    let want = inspect::digest(declared);
    let all: Vec<&inspect::Container> = cs.iter().filter(|c| inspect::image_name(&c.image) == name || c.repo_digests.iter().any(|r| inspect::image_name(r) == name)).collect();
    // A running container from the image is the component; a stopped one beside
    // it is a leftover. Stopped containers are judged only when none runs: the
    // warm standby that keeps its stack stopped.
    let running: Vec<&inspect::Container> = all.iter().copied().filter(|c| c.state.is_empty() || c.state == "running").collect();
    let mine = if running.is_empty() { all } else { running };
    if mine.is_empty() {
        return Ctr::None;
    }
    // A declaration pinned by tag alone (no digest) is met by a container
    // created from that same name and tag.
    let pinned = declared.contains("@sha256:");
    let tag = |r: &str| r.split('@').next().unwrap_or(r).rsplit('/').next().unwrap_or(r).to_string();
    let meets = |c: &inspect::Container| if pinned { container_digests(c, name).iter().any(|d| d == want) } else { tag(&c.image) == tag(declared) };
    let off: Vec<String> = mine
        .iter()
        .filter(|c| !meets(c))
        .map(|c| {
            let d = container_digests(c, name).first().cloned().unwrap_or_else(|| c.image.clone());
            format!("{} runs {}", label(c), d.chars().take(19).collect::<String>())
        })
        .collect();
    if off.is_empty() {
        Ctr::Same(mine.iter().map(|c| label(c)).collect::<Vec<_>>().join(", "))
    } else {
        Ctr::Differs(off.join("; "))
    }
}

/// Findings, matrix and summary from what was read. `rungs` is the ladder's
/// (rung name, goatnet) in order; a goatnet off the ladder has no rung.
pub fn analyze(reads: &[GoatnetRead], ladder: &[LadderRow], rungs: &[(String, String)]) -> (Vec<Finding>, Vec<MatrixRow>, Summary) {
    let mut out: Vec<Finding> = Vec::new();
    let rung_of = |g: &str| rungs.iter().find(|(_, n)| n == g).map(|(r, _)| r.clone());

    for r in reads {
        let gn = name(&r.goatnet);
        if let Some(e) = &r.error {
            out.push(f("intra", "unread", "unknown", Some(&gn), None, format!("{gn} could not be read: {e}"), Some(format!("goatbuilder doctor --env {}", r.goatnet))));
            continue;
        }
        // Declared against running, host by host: the ledger's newest row, and
        // the containers the host runs from that image.
        // One line per unreachable host, and one per host whose containers run
        // what is declared but carry no ledger, not one line per component.
        let mut unread: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut unledgered: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut unprovisioned: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for d in &r.rows {
            let ctr = container_view(r.hosts.get(&d.host), &d.declared);
            if d.state == "unreachable" {
                unread.entry(d.host.clone()).or_default().push(d.component.clone());
                continue;
            }
            if d.state == "not-provisioned" {
                unprovisioned.entry(d.group.clone()).or_insert_with(|| (d.running.clone(), Vec::new())).1.push(d.component.clone());
                continue;
            }
            if let ("no-ledger", Ctr::Same(_)) = (d.state.as_str(), &ctr) {
                unledgered.entry(d.host.clone()).or_default().push(d.component.clone());
                continue;
            }
            let deploy = Some(if r.no_play.contains(&d.component) {
                format!("{} has no deploy play goatbuilder can run: declare deploy_playbook for it in ops/pins/fleet-pins.yaml, or converge it with its standup phase", d.component)
            } else {
                format!("goatbuilder deploy --component {} --env {}", d.component, r.goatnet)
            });
            let decl = inspect::short(&d.declared);
            let mut push = |kind: &str, sev: &str, detail: String, fix: Option<String>| out.push(f("intra", kind, sev, Some(&gn), Some(&d.component), detail, fix));
            match (d.state.as_str(), &ctr) {
                ("same", Ctr::Differs(names)) => push("ledger-vs-container", "drift", format!("{}: the ledger names {decl}, but {names}", d.host), deploy),
                ("same", _) => {}
                // The ledger's newest row names another ref, but no container from the
                // image is on the host: the row outlived what it records.
                ("drift", Ctr::None) => push(
                    "stale-ledger",
                    "drift",
                    format!("{}: the ledger's newest row names {}, but no container from that image is on the host; main declares {decl}", d.host, inspect::short(&d.running)),
                    deploy,
                ),
                ("drift", _) => push(
                    "undeclared-drift",
                    "drift",
                    format!("{} runs {}; main declares {decl}{}", d.host, inspect::short(&d.running), ctr.note()),
                    Some(format!("goatbuilder deploy --component {} --env {}   (or declare what runs: goatbuilder adopt --env {})", d.component, r.goatnet, r.goatnet)),
                ),
                ("unreachable", _) => push("unknown", "unknown", format!("{} unreachable; main declares {decl}", d.host), Some(format!("goatbuilder doctor --env {}", r.goatnet))),
                // A serving surface's ledger records what it serves (goat-get, goat-docs).
                ("served-drift", _) => push("served-payload", "drift", format!("{}: image {decl} as declared, but {}", d.host, d.running), deploy),
                // The ledger records no payload yet: the host's version.json answers below.
                ("served-unknown", _) => {}
                ("no-ledger", Ctr::Same(names)) => push("no-ledger", "unknown", format!("{}: {names} runs the declared {decl}, but the host has no {} ledger to verify or roll back from", d.host, d.component), Some(format!("the ledger appears at {}'s next deploy", d.component))),
                ("no-ledger", Ctr::Differs(names)) => push("undeclared-drift", "drift", format!("{}: {names}; main declares {decl}; no {} ledger", d.host, d.component), deploy),
                ("no-ledger", Ctr::None) => push("not-running", "drift", format!("{}: main declares {decl}, and no ledger and no container from that image are on the host", d.host), deploy),
                ("no-ledger", Ctr::Unread) => push("unknown", "unknown", format!("{} has no {} ledger and its containers could not be read; main declares {decl}", d.host, d.component), None),
                ("other-image", Ctr::Same(_)) => {}
                ("other-image", Ctr::Differs(names)) => push("undeclared-drift", "drift", format!("{}: {names}; main declares {decl}", d.host), deploy),
                ("other-image", _) => push("unknown", "unknown", format!("{}: the shared ledger's newest row records another image ({}) and no container answers for {decl}", d.host, inspect::short(&d.running)), None),
                _ => {}
            }
        }
        for (h, comps) in unread {
            out.push(f("intra", "unknown", "unknown", Some(&gn), None, format!("{h} could not be read: {} component(s) unverified ({})", comps.len(), comps.join(", ")), Some(format!("goatbuilder doctor --env {}   (the transport reached no such host)", r.goatnet))));
        }
        // Declared, never built: the inventory itself says so. Named, never counted
        // as consistent and never as unread.
        for (g, (evidence, comps)) in unprovisioned {
            out.push(f(
                "intra",
                "not-provisioned",
                "declared",
                Some(&gn),
                None,
                format!("{g} is a member group with no provisioned host ({evidence}); {} declared component(s) wait on it", comps.len()),
                Some(format!("provision the hosts and fill the inventory, or drop {g} from the goatnet's member groups")),
            ));
        }
        for (h, comps) in unledgered {
            out.push(f(
                "intra",
                "no-ledger",
                "unknown",
                Some(&gn),
                None,
                format!("{h}: the containers of {} run the declared digest, but the host has no ledger for them to verify or roll back from", comps.join(", ")),
                Some("each ledger appears at that component's next deploy".into()),
            ));
        }
        // Hosts of one goatnet that run different refs of one component.
        let mut by_comp: BTreeMap<&str, BTreeMap<String, Vec<&str>>> = BTreeMap::new();
        for d in r.rows.iter().filter(|d| matches!(d.state.as_str(), "same" | "drift") && !matches!(container_view(r.hosts.get(&d.host), &d.declared), Ctr::None)) {
            by_comp.entry(&d.component).or_default().entry(inspect::digest(&d.running).to_string()).or_default().push(&d.host);
        }
        for (comp, refs) in by_comp {
            if refs.len() > 1 {
                let parts: Vec<String> = refs
                    .iter()
                    .map(|(dg, hosts)| {
                        let full = r.rows.iter().find(|d| d.component == comp && inspect::digest(&d.running) == dg).map(|d| inspect::short(&d.running)).unwrap_or_else(|| dg.clone());
                        format!("{} on {}", full, hosts.join(", "))
                    })
                    .collect();
                out.push(f("intra", "hosts-disagree", "drift", Some(&gn), Some(comp), format!("hosts disagree: {}", parts.join("; ")), Some(format!("goatbuilder deploy --component {comp} --env {}", r.goatnet))));
            }
        }
        // What goat-get hosts serve, against what main declares for their group,
        // read from the host's version.json where its ledger records no payload
        // yet (a ledger that does is judged above).
        let ledger_answered: BTreeSet<&str> = r.rows.iter().filter(|d| d.component == "goat-get" && matches!(d.state.as_str(), "same" | "served-drift")).map(|d| d.host.as_str()).collect();
        let mut served_cli: Option<String> = None;
        for (h, hf) in &r.hosts {
            let want = if ledger_answered.contains(h.as_str()) { None } else { r.declared_served.get(&hf.group) };
            if let Some(want) = want {
                for (what, tag) in want {
                    match hf.served.get(what) {
                        Some(have) if have == tag => {}
                        Some(have) => out.push(f(
                            "intra",
                            "served-payload",
                            "drift",
                            Some(&gn),
                            Some(&format!("goat-get/{what}")),
                            format!("{h} serves {have} (its version.json; the ledger records no payload yet); main declares {tag}"),
                            Some(format!("goatbuilder deploy --component goat-get --env {}", r.goatnet)),
                        )),
                        None if hf.goat_get => out.push(f("intra", "unknown", "unknown", Some(&gn), Some(&format!("goat-get/{what}")), format!("{h} runs goat-get but its {what} version.json could not be read; main declares {tag}"), None)),
                        None => {}
                    }
                }
            }
            if let Some(t) = hf.served.get("goat-cli") {
                served_cli.get_or_insert_with(|| t.clone());
            }
        }
        // The goat CLI on each host against the one this goatnet serves (or,
        // when no host of it serves one, the one main declares).
        let reference = served_cli.clone().or_else(|| r.declared_served.values().find_map(|m| m.get("goat-cli").cloned()));
        if let Some(reference) = reference {
            let want = version_of(&reference);
            for (h, hf) in &r.hosts {
                if let Some(have) = &hf.goat_cli {
                    if version_of(have) != want {
                        let src = if served_cli.is_some() { format!("{gn} serves") } else { "main declares".to_string() };
                        out.push(f(
                            "intra",
                            "goat-cli-host",
                            "drift",
                            Some(&gn),
                            Some("goat-cli"),
                            format!("{h} runs goat {}; {src} {want}", version_of(have)),
                            Some(format!("goat update on {h}, or redeploy the role that installs it (onboard-runner)")),
                        ));
                    }
                }
            }
        }
    }

    // Across goatnets: the ladder's own grade for each declared difference.
    let (dev, prod) = (rungs.first().map(|r| r.0.clone()), rungs.last().map(|r| r.0.clone()));
    let mut in_flight = 0;
    let mut prod_differs = 0;
    for l in ladder {
        for (pair, grade) in &l.pairs {
            if grade == "UNDECLARED-SKEW" {
                let (lo, hi) = pair.split_once("->").unwrap_or((pair, ""));
                let v = |r: &str| l.declared.get(r).cloned().flatten().map(|x| inspect::short(&x)).unwrap_or_else(|| "absent".into());
                out.push(f(
                    "inter",
                    "undeclared-skew",
                    "drift",
                    None,
                    Some(&l.component),
                    format!("{lo} declares {}, {hi} declares {}; no open ladder record covers the difference", v(lo), v(hi)),
                    Some(format!("goatbuilder promote status   (open a candidate for {}, or fold the value)", l.component)),
                ));
            }
        }
        if let (Some(dv), Some(pv)) = (dev.as_ref().and_then(|d| l.declared.get(d)).cloned().flatten(), prod.as_ref().and_then(|p| l.declared.get(p)).cloned().flatten()) {
            if inspect::digest(&dv) != inspect::digest(&pv) {
                prod_differs += 1;
                let declared = l.pairs.values().all(|g| g != "UNDECLARED-SKEW");
                if declared {
                    in_flight += 1;
                    out.push(f(
                        "inter",
                        "prod-differs",
                        "declared",
                        None,
                        Some(&l.component),
                        format!("prod declares {}, dev declares {} — {}", inspect::short(&pv), inspect::short(&dv), plain_grade(&l.grade)),
                        Some(format!("goatbuilder promote review   ({} moves when its record graduates)", l.component)),
                    ));
                }
            }
        }
    }

    // The matrix: one row per component, one cell per goatnet.
    let mut comps: BTreeSet<String> = reads.iter().flat_map(|r| r.rows.iter().map(|d| d.component.clone())).collect();
    comps.extend(ladder.iter().map(|l| l.component.clone()));
    let mut matrix = Vec::new();
    for c in &comps {
        let lr = ladder.iter().find(|l| &l.component == c);
        let mut cells = BTreeMap::new();
        for r in reads {
            let rows: Vec<&Drift> = r.rows.iter().filter(|d| &d.component == c).collect();
            let cell = if r.error.is_some() {
                Cell { declared: String::new(), state: "unknown".into(), hosts: 0, read: 0 }
            } else if rows.is_empty() {
                let declared = rung_of(&r.goatnet).and_then(|rg| lr.and_then(|l| l.declared.get(&rg).cloned().flatten()));
                Cell { declared: declared.map(|d| inspect::short(&d)).unwrap_or_default(), state: "absent".into(), hosts: 0, read: 0 }
            } else {
                // A host's state for the matrix: the ledger, corrected by its containers.
                let host_state = |d: &&Drift| -> &'static str {
                    match (d.state.as_str(), container_view(r.hosts.get(&d.host), &d.declared)) {
                        ("same", Ctr::Differs(_)) => "drift",
                        ("same", _) => "same",
                        ("drift" | "served-drift", _) => "drift",
                        ("served-unknown", _) => "unknown",
                        ("no-ledger" | "other-image", Ctr::Same(_)) => "same",
                        ("no-ledger" | "other-image", Ctr::Differs(_)) | ("no-ledger", Ctr::None) => "drift",
                        ("no-coverage", _) => "no-coverage",
                        _ => "unknown",
                    }
                };
                let states: Vec<&str> = rows.iter().map(host_state).collect();
                let read = states.iter().filter(|s| matches!(**s, "same" | "drift")).count();
                let state = if states.iter().all(|s| *s == "same") {
                    "same"
                } else if states.iter().all(|s| *s == "no-coverage") {
                    "no-coverage"
                } else if states.contains(&"drift") && states.contains(&"same") {
                    "mixed"
                } else if states.contains(&"drift") {
                    "drift"
                } else if read > 0 {
                    "partial"
                } else {
                    "unknown"
                };
                Cell { declared: inspect::short(&rows[0].declared), state: state.into(), hosts: rows.len(), read }
            };
            cells.insert(name(&r.goatnet), cell);
        }
        matrix.push(MatrixRow { component: c.clone(), ladder_grade: lr.map(|l| l.grade.clone()).unwrap_or_default(), cells });
    }

    let prod_g = rungs.last().map(|r| r.1.clone());
    let summary = Summary {
        goatnets: reads.len(),
        components: comps.len(),
        drift: out.iter().filter(|x| x.severity == "drift").count(),
        unknown: out.iter().filter(|x| x.severity == "unknown").count(),
        declared_in_flight: in_flight,
        prod_differs_from_dev: prod_differs,
        prod_running_off_declared: reads.iter().filter(|r| Some(&r.goatnet) == prod_g.as_ref()).flat_map(|r| r.rows.iter()).filter(|d| d.state == "drift").count(),
    };
    (out, matrix, summary)
}

// ---------------------------------------------------------------------------
// reading: the real goatnets, or a fixture
// ---------------------------------------------------------------------------

fn yaml(path: &Path) -> Result<serde_yaml::Value> {
    let text = std::fs::read_to_string(path).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", path.display())))?;
    serde_yaml::from_str(&text).map_err(|e| goat_errors::coded("GB-326", format!("{} is not YAML: {e}", path.display())).into())
}

/// A top-level `key: value` line of a vars file, unquoted.
fn var_line(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().find(|l| l.starts_with(&format!("{key}:")))?;
    let v = line[key.len() + 1..].split(" #").next().unwrap_or("").trim().trim_matches('"').trim_matches('\'').to_string();
    if v.is_empty() { None } else { Some(v) }
}

/// A role variable as a member group sees it: its group_vars, then the role default.
fn group_var(repo: &Path, group: &str, role: &str, key: &str) -> Option<String> {
    let gv = repo.join("ops/ansible/group_vars").join(group).join("vars.yml");
    var_line(&gv, key).or_else(|| var_line(&repo.join("ops/ansible/roles").join(role).join("defaults/main.yml"), key))
}

/// What main declares goat-get serves, for each member group of a goatnet.
fn declared_served(repo: &Path, members: &[String]) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for g in members {
        let mut m = BTreeMap::new();
        if let Some(t) = group_var(repo, g, "goat-get", "goat_get_release_tag") {
            m.insert("goat-cli".to_string(), t);
        }
        if group_var(repo, g, "goat-get", "goat_get_goatbuilder_enabled").as_deref() == Some("true") {
            if let Some(t) = group_var(repo, g, "goat-get", "goat_get_goatbuilder_release_tag") {
                m.insert("goatbuilder".to_string(), t);
            }
        }
        out.insert(g.clone(), m);
    }
    out
}

/// The `goatnets:` maps of fleet-pins.yaml and, when it exists, the lite sidecar
/// ops/pins/lite-pins.yaml (ADR 1213) — the union `goatbuilder pins` works on.
fn goatnet_tables(repo: &Path) -> Vec<serde_yaml::Value> {
    ["ops/pins/fleet-pins.yaml", "ops/pins/lite-pins.yaml"].iter().filter_map(|rel| yaml(&repo.join(rel)).ok()).collect()
}

fn members_of(repo: &Path, goatnet: &str) -> Vec<String> {
    goatnet_tables(repo)
        .iter()
        .find_map(|p| p.get("goatnets").and_then(|g| g.get(goatnet)).and_then(|g| g.get("members")).and_then(|m| m.as_sequence()).cloned())
        .map(|s| s.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn all_goatnets(repo: &Path) -> Vec<String> {
    goatnet_tables(repo)
        .iter()
        .flat_map(|p| p.get("goatnets").and_then(|g| g.as_mapping()).map(|m| m.keys().filter_map(|k| k.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default())
        .collect()
}

/// The ladder's rungs (name, goatnet) in order, and its grade per component.
/// The ladder's rungs as (rung name, goatnet), in order.
type Rungs = Vec<(String, String)>;

fn ladder(repo: &Path) -> Result<(Rungs, Vec<LadderRow>)> {
    let t = promote::Tree::new(repo);
    let st = promote::build_status(&t)?;
    let rungs: Vec<(String, String)> = st
        .live
        .iter()
        .map(|r| (r.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(), r.get("goatnet").and_then(|v| v.as_str()).unwrap_or("").to_string()))
        .collect();
    let rows = st
        .rows
        .iter()
        .map(|row| {
            let mut declared = BTreeMap::new();
            if let Some(Value::Object(rg)) = row.get("rungs") {
                for (k, v) in rg {
                    let val = match v.get("state").and_then(|s| s.as_str()) {
                        Some("value") => v.get("value").and_then(|x| x.as_str()).map(String::from),
                        _ => None,
                    };
                    declared.insert(k.clone(), val);
                }
            }
            let pairs = match row.get("pairs") {
                Some(Value::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect(),
                _ => BTreeMap::new(),
            };
            LadderRow {
                component: row.get("component").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                grade: row.get("grade").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                declared,
                pairs,
            }
        })
        .collect();
    Ok((rungs, rows))
}

fn read_goatnet(repo: &Path, goatnet: &str, rung: Option<String>) -> GoatnetRead {
    let t0 = Instant::now();
    let members = members_of(repo, goatnet);
    let mut r = GoatnetRead { goatnet: goatnet.to_string(), rung, declared_served: declared_served(repo, &members), ..Default::default() };
    match inspect::collect_full(repo, goatnet, None) {
        Ok((rows, transport, hosts)) => {
            r.no_play = rows.iter().map(|d| d.component.clone()).collect::<BTreeSet<_>>().into_iter().filter(|c| crate::push::resolve(repo, c, goatnet, None).is_err()).collect();
            r.rows = rows;
            r.transport = transport;
            r.hosts = hosts;
        }
        Err(e) => r.error = Some(format!("{e:#}")),
    }
    r.elapsed_secs = (t0.elapsed().as_millis() as f64) / 1000.0;
    r
}

/// The goatnets a run reads: the named ones, every declared one (`--all`), or
/// by default the ladder's rungs.
fn chosen(repo: &Path, o: &Options, rungs: &[(String, String)]) -> Vec<String> {
    if !o.envs.is_empty() {
        return o.envs.iter().map(|e| e.replace('-', "_")).collect();
    }
    if o.all {
        let mut v: Vec<String> = rungs.iter().map(|r| r.1.clone()).collect();
        for g in all_goatnets(repo) {
            if !v.contains(&g) {
                v.push(g);
            }
        }
        return v;
    }
    rungs.iter().map(|r| r.1.clone()).collect()
}

pub fn run(repo: &Path, o: &Options) -> Result<Report> {
    let t0 = Instant::now();
    let (rungs, ladder_rows) = ladder(repo)?;
    let goatnets = chosen(repo, o, &rungs);
    let reads: Vec<GoatnetRead> = std::thread::scope(|s| {
        let hs: Vec<_> = goatnets
            .iter()
            .map(|g| {
                let rung = rungs.iter().find(|r| &r.1 == g).map(|r| r.0.clone());
                s.spawn(move || read_goatnet(repo, g, rung))
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    finish(reads, ladder_rows, rungs, t0, o)
}

/// The same survey over a fixture: its environments in promotion order, each
/// machine's newest record against the fixture's declaration.
pub fn run_mock(f: &verbs::Fixture, o: &Options) -> Result<Report> {
    let t0 = Instant::now();
    let mut path: Vec<(String, String)> = f.config.promotion.path.iter().filter_map(|s| s.environment.clone().map(|e| (s.name.clone(), e))).collect();
    if path.is_empty() {
        path = f.config.environments.iter().map(|e| (e.name.clone(), e.name.clone())).collect();
    }
    let envs: Vec<String> = if o.envs.is_empty() { path.iter().map(|p| p.1.clone()).collect() } else { o.envs.to_vec() };
    let scope = crate::standup::class_scope(&f.root);
    let mut reads = Vec::new();
    for env in &envs {
        let mut r = GoatnetRead { goatnet: env.clone(), rung: path.iter().find(|p| &p.1 == env).map(|p| p.0.clone()), transport: "fixture".into(), ..Default::default() };
        for comp in f.config.components.keys() {
            let Ok(declared) = f.declared_ref(env, comp) else { continue };
            let machines = scope.as_ref().and_then(|s| s.get(comp)).cloned().unwrap_or_else(|| f.converger.machines(env));
            for m in machines {
                let (running, state, ts, action) = match f.converger.read_record(env, &m, comp) {
                    Ok(Some(rec)) => {
                        let st = if inspect::digest(&rec.reference) == inspect::digest(&declared) { "same" } else { "drift" };
                        (rec.reference.clone(), st.to_string(), rec.ts.clone(), rec.action.clone())
                    }
                    Ok(None) => (String::new(), "no-ledger".to_string(), String::new(), String::new()),
                    Err(_) => (String::new(), "unreachable".to_string(), String::new(), String::new()),
                };
                r.rows.push(Drift { component: comp.clone(), group: env.clone(), host: m.clone(), declared: declared.clone(), running, state, row_ts: ts, action, read_failed: false });
            }
        }
        reads.push(r);
    }
    // The fixture's ladder: open candidate refs in `declared/ladder-records.yaml`
    // (component → [ref]) declare a difference; any other difference is undeclared.
    let open: BTreeMap<String, Vec<String>> = std::fs::read_to_string(f.root.join("declared/ladder-records.yaml"))
        .ok()
        .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        .and_then(|v| v.get("open").cloned())
        .and_then(|v| serde_yaml::from_value(v).ok())
        .unwrap_or_default();
    let mut ladder_rows = Vec::new();
    for comp in f.config.components.keys() {
        let mut declared = BTreeMap::new();
        for (rung, env) in &path {
            declared.insert(rung.clone(), f.declared_ref(env, comp).ok());
        }
        let vals: Vec<(String, String)> = path.iter().filter_map(|(rung, _)| declared.get(rung).cloned().flatten().map(|v| (rung.clone(), v))).collect();
        let mut pairs = BTreeMap::new();
        for w in vals.windows(2) {
            let g = if w[0].1 == w[1].1 {
                "aligned"
            } else if open.get(comp).map(|refs| refs.iter().any(|x| x == &w[0].1 || x == &w[1].1)).unwrap_or(false) {
                "candidate"
            } else {
                "UNDECLARED-SKEW"
            };
            pairs.insert(format!("{}->{}", w[0].0, w[1].0), g.to_string());
        }
        let grade = ["UNDECLARED-SKEW", "candidate"].iter().find(|g| pairs.values().any(|v| v == *g)).map(|g| g.to_string()).unwrap_or_else(|| if vals.len() < 2 { "single-rung".into() } else { "aligned".into() });
        ladder_rows.push(LadderRow { component: comp.clone(), grade, declared, pairs });
    }
    finish(reads, ladder_rows, path, t0, o)
}

fn finish(reads: Vec<GoatnetRead>, ladder_rows: Vec<LadderRow>, rungs: Vec<(String, String)>, t0: Instant, o: &Options) -> Result<Report> {
    if !reads.is_empty() && reads.iter().all(|r| r.error.is_some()) {
        let why: Vec<String> = reads.iter().map(|r| format!("{}: {}", name(&r.goatnet), r.error.clone().unwrap_or_default())).collect();
        return Err(goat_errors::coded("GB-070", format!("no goatnet could be read — {}", why.join("; "))).into());
    }
    let (mut findings, matrix, summary) = analyze(&reads, &ladder_rows, &rungs);
    findings.retain(|x| (o.intra && x.scope == "intra") || (o.inter && x.scope == "inter"));
    let elapsed = (t0.elapsed().as_millis() as f64) / 1000.0;
    let order: Vec<String> = reads.iter().map(|r| name(&r.goatnet)).collect();
    let survey = Survey { schema: SCHEMA, generated_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(), budget_secs: BUDGET_SECS, elapsed_secs: elapsed, order, summary, findings, matrix, goatnets: reads };
    if let Some(p) = o.html {
        std::fs::write(p, html(&survey)).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", p.display())))?;
    }
    match o.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&survey).map_err(|e| goat_errors::coded("GB-322", format!("could not serialize: {e}")))?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&survey).map_err(|e| goat_errors::coded("GB-322", format!("could not serialize: {e}")))?),
        // One trailing newline, so the next prompt is on its own line.
        OutputFormat::Table => println!("{}", table(&survey, o.intra, o.inter, o.html).trim_end()),
    }
    let failing = survey.findings.iter().any(|x| x.severity != "declared");
    let exit = if failing { EXIT_NOT_CONVERGED } else { EXIT_OK };
    let mut r = Report::new("survey", vec![], exit);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    Ok(r)
}

// ---------------------------------------------------------------------------
// presentation
// ---------------------------------------------------------------------------

fn pad(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - n)) }
}

/// The declared value as a reader compares it: the tag, or the digest's first 12.
fn tag_of(short: &str) -> String {
    if short.is_empty() {
        return "—".into();
    }
    let (name, dg) = short.split_once('@').unwrap_or((short, ""));
    match name.split_once(':') {
        Some((_, tag)) => tag.to_string(),
        None if !dg.is_empty() => dg.trim_start_matches("sha256:").chars().take(12).collect(),
        None => name.to_string(),
    }
}

fn mark(c: &Cell) -> String {
    match c.state.as_str() {
        "same" => String::new(),
        "drift" => " DRIFT".into(),
        "mixed" => " MIXED".into(),
        "partial" => format!(" ({}/{} read)", c.read, c.hosts),
        "unknown" => " ?".into(),
        "no-coverage" => " (no ledger kept)".into(),
        "absent" => " absent".into(),
        _ => String::new(),
    }
}

pub fn table(s: &Survey, intra: bool, inter: bool, html: Option<&Path>) -> String {
    table_for(s, intra, inter, html, std::io::IsTerminal::is_terminal(&std::io::stdout()))
}

/// What running on a machine differs from what main declares, as table cells.
fn drift_row(x: &Finding) -> Option<Vec<String>> {
    let (machine, rest) = x.detail.split_once(" runs ")?;
    let (running, declared) = rest.split_once("; main declares ")?;
    if machine.contains(' ') || running.contains(' ') || declared.contains(' ') {
        return None;
    }
    let state = x.severity.clone();
    let cells = [x.goatnet.clone().unwrap_or_default(), x.component.clone().unwrap_or_default(), machine.to_string()];
    Some([vec![state], cells.to_vec(), vec![crate::verbs::version_of(running), crate::verbs::version_of(declared)]].concat())
}

/// Rows for a machine that couldn't be read: one per component it left unverified.
fn unread_rows(x: &Finding) -> Option<Vec<Vec<String>>> {
    let (machine, rest) = x.detail.split_once(" could not be read: ")?;
    let inside = rest.rsplit_once('(')?.1.strip_suffix(')')?;
    Some(inside.split(", ").map(|c| vec!["unknown".to_string(), x.goatnet.clone().unwrap_or_default(), c.to_string(), machine.to_string(), "unreadable".to_string(), "-".to_string()]).collect())
}

/// A finding between goatnets in plain words.
fn across_line(x: &Finding) -> String {
    let re = regex::Regex::new(r"^(\S+) declares (\S+), (\S+) declares (\S+); no open ladder record covers the difference$").expect("regex");
    let v = crate::verbs::version_of;
    let component = x.component.clone().unwrap_or_default();
    match re.captures(&x.detail) {
        Some(m) => format!("{component}: {} declares {}, {} declares {}. No open release covers the difference.", &m[1], v(&m[2]), &m[3], v(&m[4])),
        None => format!("{component}: {}", x.detail.replace(" \u{2014} ", ": ")),
    }
}

/// The commands a real goatnet would need, each once, then the other way out.
fn fix_lines(fs: &[&Finding]) -> (Vec<String>, Vec<String>) {
    let adopt = regex::Regex::new(r"goatbuilder adopt --env \S+?(?:\)|$)").expect("regex");
    let (mut commands, mut adopts) = (Vec::new(), Vec::new());
    for fix in fs.iter().filter_map(|x| x.fix.as_ref()) {
        let command = fix.split(" (").next().unwrap_or(fix).trim().to_string();
        if !commands.contains(&command) {
            commands.push(command);
        }
        if let Some(m) = adopt.find(fix) {
            let a = m.as_str().trim_end_matches(')').to_string();
            if !adopts.contains(&a) {
                adopts.push(a);
            }
        }
    }
    (commands, adopts)
}

/// What a person reads: it says first that this only reads, then one table per
/// question, and marks the fixes as what a real goatnet would need.
fn survey_page(s: &Survey, intra: bool, inter: bool) -> String {
    use crate::tutor::fill;
    let color = crate::output::want_color(crate::output::ColorChoice::Auto);
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let m = &s.summary;
    let mut o = format!("{} {}\n", bold("Survey:"), s.order.join(", "));
    o.push_str(&format!("{}\n", fill("This only reads: nothing here changed anything. What you could do about it is at the end.", "", "")));
    let machines = if m.unknown == 1 { "1 machine could not be read".to_string() } else { format!("{} machines could not be read", m.unknown) };
    let line = format!("{} components read in {:.1} s. {} versions differ from main, and {machines}.", m.components, s.elapsed_secs, m.drift);
    o.push_str(&fill(&line, "", ""));
    let inside: Vec<&Finding> = s.findings.iter().filter(|x| x.scope == "intra").collect();
    let across: Vec<&Finding> = s.findings.iter().filter(|x| x.scope == "inter").collect();
    if intra && !inside.is_empty() {
        let mut rows: Vec<Vec<String>> = inside.iter().filter_map(|x| drift_row(x)).collect();
        rows.extend(inside.iter().filter_map(|x| unread_rows(x)).flatten());
        o.push_str(&format!("\n{}\n", bold(&format!("Machines to look at ({}):", rows.len()))));
        o.push_str(&crate::output::columns(&["STATE", "GOATNET", "COMPONENT", "MACHINE", "RUNNING", "DECLARED"], &rows, color));
        o.push('\n');
        for x in inside.iter().filter(|x| drift_row(x).is_none() && unread_rows(x).is_none()) {
            let at = [x.goatnet.clone(), x.component.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
            o.push_str(&fill(&format!("{}: {at}: {}", x.severity, x.detail.replace(" \u{2014} ", ": ")), "", "  "));
        }
    }
    if inter {
        if !across.is_empty() {
            o.push_str(&format!("\n{}\n", bold(&format!("Differences between goatnets ({}):", across.len()))));
            for x in &across {
                o.push_str(&fill(&across_line(x), "", "  "));
            }
        }
        let rows: Vec<Vec<String>> = s
            .matrix
            .iter()
            .map(|r| {
                let mut row = vec![r.component.clone()];
                for g in &s.order {
                    let c = r.cells.get(g).cloned().unwrap_or_default();
                    let cell = if c.state == "absent" && c.declared.is_empty() { "-".to_string() } else { format!("{}{}", tag_of(&c.declared), mark(&c).to_lowercase()) };
                    row.push(cell);
                }
                row.push(plain_grade(&r.ladder_grade));
                row
            })
            .collect();
        let upper: Vec<String> = s.order.iter().map(|g| name(g).to_uppercase()).collect();
        let mut titles: Vec<&str> = vec!["COMPONENT"];
        titles.extend(upper.iter().map(String::as_str));
        titles.push("RELEASE");
        o.push_str(&format!("\n{}\n", crate::output::columns(&titles, &rows, color)));
    }
    let all: Vec<&Finding> = s.findings.iter().collect();
    let (commands, adopts) = fix_lines(&all);
    if !commands.is_empty() {
        o.push_str(&format!("\n{}\n", bold("To fix these on a real goatnet, you would run:")));
        for c in &commands {
            o.push_str(&format!("  {}\n", bold(c)));
        }
        if !adopts.is_empty() {
            o.push_str("\nOr, if what is running is right, make main declare it:\n");
            for a in &adopts {
                o.push_str(&format!("  {}\n", bold(a)));
            }
        }
    }
    o.trim_end().to_string()
}

/// A ladder grade as a person reads it. The grades themselves are the words the
/// ladder uses, and `-o json` keeps them; this column is for the reader.
fn plain_grade(grade: &str) -> String {
    match grade {
        "UNDECLARED-SKEW" => "difference not declared",
        "candidate" => "release in progress",
        "aligned" => "same everywhere",
        "single-rung" => "one stage only",
        other => other,
    }
    .to_string()
}

fn table_for(s: &Survey, intra: bool, inter: bool, html: Option<&Path>, terminal: bool) -> String {
    if terminal && html.is_none() {
        return survey_page(s, intra, inter);
    }
    let mut o = String::new();
    let m = &s.summary;
    o.push_str(&format!(
        "survey of {} goatnet(s) — {} component(s), read in {:.1} s (budget {} s)\n",
        s.order.join(", "),
        m.components,
        s.elapsed_secs,
        s.budget_secs
    ));
    o.push_str(&format!(
        "  {} drift · {} unknown · {} declared and in flight · prod declares a different ref from dev for {} component(s) · {} prod host-component(s) off what prod declares\n\n",
        m.drift, m.unknown, m.declared_in_flight, m.prod_differs_from_dev, m.prod_running_off_declared
    ));
    for (scope, title, on) in [("intra", "Inside each goatnet", intra), ("inter", "Across goatnets", inter)] {
        if !on {
            continue;
        }
        let fs: Vec<&Finding> = s.findings.iter().filter(|x| x.scope == scope).collect();
        o.push_str(&format!("{title}: {} finding(s)\n", fs.len()));
        for x in fs {
            let tag = match x.severity.as_str() {
                "drift" => "DRIFT  ",
                "unknown" => "UNKNOWN",
                _ => "DECLARED",
            };
            let at = [x.goatnet.clone(), x.component.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
            o.push_str(&format!("  {tag} {at}: {}\n", x.detail));
            if let Some(fix) = &x.fix {
                o.push_str(&format!("          → {fix}\n"));
            }
        }
        o.push('\n');
    }
    if inter {
        let w0 = s.matrix.iter().map(|r| r.component.len()).max().unwrap_or(9).max(9);
        let cw = 30;
        o.push_str(&format!("{}  {}  ladder\n", pad("component", w0), s.order.iter().map(|g| pad(g, cw)).collect::<Vec<_>>().join("")));
        for r in &s.matrix {
            let cells: Vec<String> = s
                .order
                .iter()
                .map(|g| {
                    let c = r.cells.get(g).cloned().unwrap_or_default();
                    let v = if c.state == "absent" && c.declared.is_empty() { "—".to_string() } else { format!("{}{}", tag_of(&c.declared), mark(&c)) };
                    pad(&v, cw)
                })
                .collect();
            o.push_str(&format!("{}  {}  {}\n", pad(&r.component, w0), cells.join(""), r.ladder_grade));
        }
        o.push('\n');
    }
    for g in &s.goatnets {
        let cli: BTreeSet<String> = g.hosts.values().filter_map(|h| h.goat_cli.clone()).map(|v| version_of(&v)).collect();
        let served: BTreeSet<String> = g.hosts.values().filter_map(|h| h.served.get("goat-cli").cloned()).collect();
        o.push_str(&format!(
            "{}: read via {} in {:.1} s; goat CLI on hosts: {}; goat-get serves: {}\n",
            name(&g.goatnet),
            if g.error.is_some() { "— (unread)".to_string() } else { g.transport.clone() },
            g.elapsed_secs,
            if cli.is_empty() { "—".into() } else { cli.into_iter().collect::<Vec<_>>().join(", ") },
            if served.is_empty() { "—".into() } else { served.into_iter().collect::<Vec<_>>().join(", ") }
        ));
    }
    if let Some(p) = html {
        o.push_str(&format!("\nreport: {}\n", p.display()));
    }
    o
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A self-contained, theme-aware page: the findings first, then the matrix.
pub fn html(s: &Survey) -> String {
    let m = &s.summary;
    let mut rows_f = String::new();
    for x in &s.findings {
        rows_f.push_str(&format!(
            "<tr class=\"{}\"><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>\n",
            esc(&x.severity),
            esc(&x.severity),
            esc(&x.scope),
            esc(x.goatnet.as_deref().unwrap_or("all")),
            esc(x.component.as_deref().unwrap_or("")),
            esc(&x.detail),
            esc(x.fix.as_deref().unwrap_or(""))
        ));
    }
    let head: String = s.order.iter().map(|g| format!("<th>{}</th>", esc(g))).collect();
    let mut rows_m = String::new();
    for r in &s.matrix {
        let cells: String = s
            .order
            .iter()
            .map(|g| {
                let c = r.cells.get(g).cloned().unwrap_or_default();
                format!("<td class=\"{}\" title=\"{}\">{}<small>{}</small></td>", esc(&c.state), esc(&c.declared), esc(&tag_of(&c.declared)), esc(&mark(&c)))
            })
            .collect();
        rows_m.push_str(&format!("<tr><th scope=\"row\">{}</th>{cells}<td>{}</td></tr>\n", esc(&r.component), esc(&r.ladder_grade)));
    }
    let mut rows_g = String::new();
    for g in &s.goatnets {
        let cli: BTreeSet<String> = g.hosts.values().filter_map(|h| h.goat_cli.clone()).map(|v| version_of(&v)).collect();
        let served: BTreeSet<String> = g.hosts.values().filter_map(|h| h.served.get("goat-cli").cloned()).collect();
        rows_g.push_str(&format!(
            "<tr><th scope=\"row\">{}</th><td>{}</td><td>{}</td><td>{:.1} s</td><td>{}</td><td>{}</td></tr>\n",
            esc(&name(&g.goatnet)),
            esc(g.rung.as_deref().unwrap_or("—")),
            esc(g.error.as_deref().map(|e| format!("unread: {e}")).as_deref().unwrap_or(&g.transport)),
            g.elapsed_secs,
            esc(&cli.into_iter().collect::<Vec<_>>().join(", ")),
            esc(&served.into_iter().collect::<Vec<_>>().join(", "))
        ));
    }
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Software Consistency Survey</title>
<style>
:root {{ --bg:#fbfaf7; --fg:#15191c; --muted:#4a5257; --rule:#cfd3d1; --panel:#eef0ee; --drift:#a3260f; --drift-bg:#fbe3dc; --unknown:#7a5a00; --unknown-bg:#fdf0c8; --declared:#1d4f8a; --declared-bg:#dde8f6; --same:#1d6b45; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --bg:#111416; --fg:#eceeec; --muted:#b4bbb7; --rule:#353b3e; --panel:#1b2023; --drift:#ff9a85; --drift-bg:#3a1a12; --unknown:#f0cc68; --unknown-bg:#33290b; --declared:#9cc4f5; --declared-bg:#14263b; --same:#7fd8a8; color-scheme:dark }} }}
:root[data-theme="dark"] {{ --bg:#111416; --fg:#eceeec; --muted:#b4bbb7; --rule:#353b3e; --panel:#1b2023; --drift:#ff9a85; --drift-bg:#3a1a12; --unknown:#f0cc68; --unknown-bg:#33290b; --declared:#9cc4f5; --declared-bg:#14263b; --same:#7fd8a8; color-scheme:dark }}
body {{ background:var(--bg); color:var(--fg); font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif; margin:0; padding-block:28px 48px; padding-left:max(16px,env(safe-area-inset-left,0px)); padding-right:max(16px,env(safe-area-inset-right,0px)) }}
main {{ max-width:1100px; margin:0 auto }}
h1 {{ font-size:1.6rem; margin:0 0 4px }} h2 {{ font-size:1.05rem; margin:28px 0 8px }}
.sub {{ color:var(--muted); margin:0 0 18px }}
.tiles {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(150px,1fr)); gap:10px }}
.tile {{ background:var(--panel); padding:12px 14px; border-left:4px solid var(--rule) }}
.tile b {{ display:block; font-size:1.6rem; font-variant-numeric:tabular-nums }}
.tile.d {{ border-color:var(--drift) }} .tile.u {{ border-color:var(--unknown) }} .tile.i {{ border-color:var(--declared) }}
.wrap {{ overflow-x:auto }}
table {{ border-collapse:collapse; width:100%; font-size:.9rem }}
th,td {{ border-bottom:1px solid var(--rule); padding:6px 8px; text-align:left; vertical-align:top }}
thead th {{ background:var(--panel) }}
code {{ font-family:ui-monospace,Menlo,monospace; font-size:.85em }}
tr.drift td:first-child, td.drift, td.mixed {{ color:var(--drift); background:var(--drift-bg); font-weight:600 }}
tr.unknown td:first-child, td.unknown {{ color:var(--unknown); background:var(--unknown-bg); font-weight:600 }}
tr.declared td:first-child {{ color:var(--declared); background:var(--declared-bg); font-weight:600 }}
td.same {{ color:var(--same) }} td.absent {{ color:var(--muted) }}
small {{ margin-left:4px }}
</style></head><body><main>
<h1>Software consistency: {order}</h1>
<p class="sub">Generated {gen} by <code>goatbuilder survey</code> in {el:.1} s (budget {bud} s). Read only: each host's newest deploy-ledger row, its containers (running and stopped) with their images' registry digests, its goat CLI, and what goat-get serves; the ladder's grade from <code>promote status</code>.</p>
<div class="tiles">
<div class="tile d"><b>{drift}</b>drift nothing declares</div>
<div class="tile u"><b>{unknown}</b>unknown (could not be read)</div>
<div class="tile i"><b>{flight}</b>declared differences in flight</div>
<div class="tile"><b>{pdd}</b>components where prod declares a different ref from dev</div>
<div class="tile d"><b>{poff}</b>prod host-components not running what prod declares</div>
</div>
<h2>Findings</h2>
<div class="wrap"><table><thead><tr><th>Severity</th><th>Scope</th><th>Goatnet</th><th>Component</th><th>What differs</th><th>Fix</th></tr></thead><tbody>
{rows_f}</tbody></table></div>
<h2>Declared ref per goatnet, and whether its hosts run it</h2>
<div class="wrap"><table><thead><tr><th>Component</th>{head}<th>Ladder grade</th></tr></thead><tbody>
{rows_m}</tbody></table></div>
<h2>How each goatnet was read</h2>
<div class="wrap"><table><thead><tr><th>Goatnet</th><th>Rung</th><th>Transport</th><th>Read in</th><th>goat CLI on hosts</th><th>goat-get serves</th></tr></thead><tbody>
{rows_g}</tbody></table></div>
</main></body></html>
"#,
        order = esc(&s.order.join(", ")),
        gen = esc(&s.generated_at),
        el = s.elapsed_secs,
        bud = s.budget_secs,
        drift = m.drift,
        unknown = m.unknown,
        flight = m.declared_in_flight,
        pdd = m.prod_differs_from_dev,
        poff = m.prod_running_off_declared,
    )
}

/// Where `--html` writes when given a directory.
pub fn html_path(p: &Path) -> PathBuf {
    if p.is_dir() { p.join("survey.html") } else { p.to_path_buf() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(c: &str, host: &str, declared: &str, running: &str, state: &str) -> Drift {
        Drift { component: c.into(), group: "g_hosts".into(), host: host.into(), declared: declared.into(), running: running.into(), state: state.into(), row_ts: String::new(), action: "deploy".into(), read_failed: false }
    }
    const A: &str = "ghcr.io/o/portal:v2@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "ghcr.io/o/portal:v1@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn rungs() -> Vec<(String, String)> {
        vec![("dev".into(), "dev_g".into()), ("prod".into(), "prod_g".into())]
    }

    #[test]
    fn a_host_off_its_declaration_and_hosts_that_disagree_are_drift() {
        let r = GoatnetRead { goatnet: "dev_g".into(), rows: vec![d("portal", "a", A, A, "same"), d("portal", "b", A, B, "drift")], ..Default::default() };
        let (f, m, s) = analyze(&[r], &[], &rungs());
        assert!(f.iter().any(|x| x.kind == "undeclared-drift" && x.detail.starts_with("b runs portal:v1")));
        assert!(f.iter().any(|x| x.kind == "hosts-disagree"));
        assert_eq!(m[0].cells["dev-g"].state, "mixed");
        assert_eq!(s.drift, 2);
    }

    fn ctr(name: &str, image: &str, state: &str) -> inspect::Container {
        inspect::Container { name: name.into(), image: image.into(), repo_digests: vec![image.into()], state: state.into() }
    }

    fn host(cs: Vec<inspect::Container>) -> BTreeMap<String, HostFacts> {
        [("a".to_string(), HostFacts { group: "g_hosts".into(), containers: Some(cs), ..Default::default() })].into()
    }

    #[test]
    fn a_ledger_the_containers_contradict_is_drift() {
        let r = GoatnetRead { goatnet: "dev_g".into(), rows: vec![d("portal", "a", A, A, "same")], hosts: host(vec![ctr("portal", B, "running")]), ..Default::default() };
        let (f, m, _) = analyze(&[r], &[], &rungs());
        let x = f.iter().find(|x| x.kind == "ledger-vs-container").expect("ledger-vs-container");
        assert!(x.detail.contains("container portal runs sha256:bbbbbbbbbbbb"), "{}", x.detail);
        assert_eq!(m[0].cells["dev-g"].state, "drift");
    }

    #[test]
    fn a_stopped_standby_on_the_declared_digest_is_consistent_and_one_off_it_is_drift() {
        let ok = GoatnetRead { goatnet: "prod_g".into(), rows: vec![d("portal", "a", A, "", "no-ledger")], hosts: host(vec![ctr("portal-b", A, "exited")]), ..Default::default() };
        let (f, m, _) = analyze(&[ok], &[], &rungs());
        assert!(f.iter().all(|x| x.kind == "no-ledger"), "{f:?}");
        assert!(f[0].detail.contains("portal"));
        assert_eq!(m[0].cells["prod-g"].state, "same");
        let off = GoatnetRead { goatnet: "prod_g".into(), rows: vec![d("portal", "a", A, "", "no-ledger")], hosts: host(vec![ctr("portal-b", B, "exited")]), ..Default::default() };
        let (f, _, _) = analyze(&[off], &[], &rungs());
        let x = f.iter().find(|x| x.kind == "undeclared-drift").expect("drift");
        assert!(x.detail.contains("container portal-b (exited) runs"), "{}", x.detail);
    }

    #[test]
    fn stopped_leftovers_beside_a_running_container_are_not_the_component() {
        let r = GoatnetRead {
            goatnet: "dev_g".into(),
            rows: vec![d("portal", "a", A, A, "same")],
            hosts: host(vec![ctr("portal", A, "running"), ctr("rig-portal", B, "exited")]),
            ..Default::default()
        };
        let (f, m, _) = analyze(&[r], &[], &rungs());
        assert!(f.is_empty(), "{f:?}");
        assert_eq!(m[0].cells["dev-g"].state, "same");
    }

    #[test]
    fn a_tag_only_declaration_is_met_by_a_container_from_that_tag() {
        let tagged = "ghcr.io/o/grant-keyd:sha-0751afc";
        let c = inspect::Container { name: "grant-keyd".into(), image: tagged.into(), repo_digests: vec![format!("ghcr.io/o/grant-keyd@sha256:{}", "5".repeat(64))], state: "running".into() };
        let r = GoatnetRead { goatnet: "dev_g".into(), rows: vec![d("grant-keyd", "a", tagged, tagged, "same")], hosts: host(vec![c]), ..Default::default() };
        let (f, m, _) = analyze(&[r], &[], &rungs());
        assert!(f.is_empty(), "{f:?}");
        assert_eq!(m[0].cells["dev-g"].state, "same");
    }

    #[test]
    fn declared_with_no_ledger_and_no_container_is_not_running() {
        let r = GoatnetRead { goatnet: "dev_g".into(), rows: vec![d("portal", "a", A, "", "no-ledger")], hosts: host(vec![]), ..Default::default() };
        let (f, _, _) = analyze(&[r], &[], &rungs());
        assert!(f.iter().any(|x| x.kind == "not-running" && x.severity == "drift"));
    }

    #[test]
    fn an_unreachable_host_is_one_finding_and_its_cells_say_how_many_were_read() {
        let r = GoatnetRead {
            goatnet: "dev_g".into(),
            rows: vec![d("portal", "a", A, A, "same"), d("portal", "b", A, "", "unreachable"), d("router", "b", A, "", "unreachable")],
            ..Default::default()
        };
        let (f, m, _) = analyze(&[r], &[], &rungs());
        let u: Vec<_> = f.iter().filter(|x| x.severity == "unknown").collect();
        assert_eq!(u.len(), 1);
        assert!(u[0].detail.starts_with("b could not be read: 2 component(s) unverified"));
        let portal = m.iter().find(|r| r.component == "portal").unwrap();
        assert_eq!(portal.cells["dev-g"].state, "partial");
        assert_eq!(mark(&portal.cells["dev-g"]), " (1/2 read)");
    }

    #[test]
    fn an_unread_host_or_goatnet_is_unknown_never_consistent() {
        let a = GoatnetRead { goatnet: "dev_g".into(), rows: vec![d("portal", "a", A, "", "unreachable")], ..Default::default() };
        let b = GoatnetRead { goatnet: "prod_g".into(), error: Some("door to prod_g did not open".into()), ..Default::default() };
        let (f, m, s) = analyze(&[a, b], &[], &rungs());
        assert_eq!(s.unknown, 2);
        assert_eq!(m[0].cells["dev-g"].state, "unknown");
        assert_eq!(m[0].cells["prod-g"].state, "unknown");
        assert!(f.iter().all(|x| x.severity == "unknown"));
    }

    #[test]
    fn served_payload_and_host_goat_cli_are_compared_with_what_is_declared() {
        let mut hosts = BTreeMap::new();
        hosts.insert("m1".to_string(), HostFacts { group: "g_hosts".into(), goat_cli: Some("0.5.6".into()), served: [("goat-cli".to_string(), "goat-cli-v0.5.6".to_string())].into(), goat_get: true, ..Default::default() });
        let r = GoatnetRead {
            goatnet: "prod_g".into(),
            hosts,
            declared_served: [("g_hosts".to_string(), [("goat-cli".to_string(), "goat-cli-v0.5.14".to_string())].into())].into(),
            ..Default::default()
        };
        let (f, _, _) = analyze(&[r], &[], &rungs());
        let sp = f.iter().find(|x| x.kind == "served-payload").expect("served drift");
        assert_eq!(sp.detail, "m1 serves goat-cli-v0.5.6 (its version.json; the ledger records no payload yet); main declares goat-cli-v0.5.14");
        // The host's CLI matches what its own goatnet serves: no goat-cli-host finding.
        assert!(!f.iter().any(|x| x.kind == "goat-cli-host"));
    }

    #[test]
    fn a_serving_ledger_that_records_its_payload_is_the_verdict_not_version_json() {
        let mut hosts = BTreeMap::new();
        hosts.insert("m1".to_string(), HostFacts { group: "g_hosts".into(), served: [("goat-cli".to_string(), "goat-cli-v0.5.6".to_string())].into(), goat_get: true, ..Default::default() });
        let r = GoatnetRead {
            goatnet: "prod_g".into(),
            rows: vec![Drift { running: "served payload DRIFT: goat-cli goat-cli-v0.5.6, declared goat-cli-v0.5.14".into(), ..d("goat-get", "m1", A, A, "served-drift") }],
            hosts,
            declared_served: [("g_hosts".to_string(), [("goat-cli".to_string(), "goat-cli-v0.5.14".to_string())].into())].into(),
            ..Default::default()
        };
        let (f, m, _) = analyze(&[r], &[], &rungs());
        let sp: Vec<_> = f.iter().filter(|x| x.kind == "served-payload").collect();
        assert_eq!(sp.len(), 1, "one finding, from the ledger: {f:?}");
        assert!(sp[0].detail.contains("served payload DRIFT"));
        assert_eq!(m[0].cells["prod-g"].state, "drift");
    }

    #[test]
    fn a_host_goat_cli_behind_what_its_goatnet_serves_is_drift() {
        let mut hosts = BTreeMap::new();
        hosts.insert("m1".to_string(), HostFacts { group: "g".into(), goat_cli: Some("0.5.14".into()), served: [("goat-cli".to_string(), "goat-cli-v0.5.14".to_string())].into(), goat_get: true, ..Default::default() });
        hosts.insert("m2".to_string(), HostFacts { group: "g".into(), goat_cli: Some("0.4.41".into()), ..Default::default() });
        let r = GoatnetRead { goatnet: "dev_g".into(), hosts, ..Default::default() };
        let (f, _, _) = analyze(&[r], &[], &rungs());
        let x = f.iter().find(|x| x.kind == "goat-cli-host").expect("goat-cli-host");
        assert_eq!(x.detail, "m2 runs goat 0.4.41; dev-g serves 0.5.14");
    }

    #[test]
    fn prod_behind_dev_is_declared_when_the_ladder_carries_it_and_drift_when_not() {
        let mk = |c: &str, grade: &str| LadderRow {
            component: c.into(),
            grade: grade.into(),
            declared: [("dev".to_string(), Some(A.to_string())), ("prod".to_string(), Some(B.to_string()))].into(),
            pairs: [("dev->prod".to_string(), grade.to_string())].into(),
        };
        let (f, _, s) = analyze(&[], &[mk("portal", "candidate"), mk("router", "UNDECLARED-SKEW")], &rungs());
        assert!(f.iter().any(|x| x.kind == "prod-differs" && x.severity == "declared" && x.component.as_deref() == Some("portal")));
        assert!(f.iter().any(|x| x.kind == "undeclared-skew" && x.component.as_deref() == Some("router")));
        assert_eq!(s.prod_differs_from_dev, 2);
        assert_eq!(s.declared_in_flight, 1);
        assert_eq!(s.drift, 1);
    }

    #[test]
    fn versions_compare_across_the_three_spellings() {
        assert_eq!(version_of("goat-cli-v0.5.14"), "0.5.14");
        assert_eq!(version_of("goat 0.5.14"), "0.5.14");
        assert_eq!(version_of("0.5.14 (abc123)"), "0.5.14");
        assert_eq!(tag_of("portal:v1.4.0@sha256:0b2a6c1d3e4f"), "v1.4.0");
        assert_eq!(tag_of("zenoh-pqc-fips@sha256:a99d8aa2be23"), "a99d8aa2be23");
    }

    #[test]
    fn the_html_report_is_self_contained_and_escapes() {
        let r = GoatnetRead { goatnet: "dev_g".into(), rows: vec![d("por<tal", "a", A, B, "drift")], ..Default::default() };
        let (findings, matrix, summary) = analyze(std::slice::from_ref(&r), &[], &rungs());
        let s = Survey { schema: SCHEMA, generated_at: "t".into(), budget_secs: 90, elapsed_secs: 1.0, order: vec!["dev-g".into()], summary, findings, matrix, goatnets: vec![r] };
        let h = html(&s);
        assert!(h.contains("por&lt;tal") && !h.contains("por<tal"));
        assert!(!h.contains("<script") && !h.contains("http://") && !h.contains("https://"));
        assert!(h.contains("prefers-color-scheme: dark"));
    }

    #[test]
    fn a_ledger_row_with_no_container_is_stale_not_running() {
        let r = GoatnetRead { goatnet: "prod_g".into(), rows: vec![d("portal", "b", A, B, "drift"), d("portal", "a", A, A, "same")], hosts: [("a".to_string(), HostFacts { group: "g_hosts".into(), containers: Some(vec![ctr("portal", A, "running")]), ..Default::default() }), ("b".to_string(), HostFacts { group: "g_hosts".into(), containers: Some(vec![]), ..Default::default() })].into(), ..Default::default() };
        let (f, _, _) = analyze(&[r], &[], &rungs());
        let x = f.iter().find(|x| x.kind == "stale-ledger").expect("stale-ledger");
        assert!(x.detail.contains("no container from that image"), "{}", x.detail);
        assert!(!f.iter().any(|x| x.kind == "undeclared-drift" || x.kind == "hosts-disagree"), "{f:?}");
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;

    fn finding(scope: &str, severity: &str, component: Option<&str>, detail: &str, fix: Option<&str>) -> Finding {
        f(scope, "kind", severity, Some("dev"), component, detail.to_string(), fix.map(String::from))
    }

    #[test]
    fn a_page_says_what_each_finding_means_and_marks_the_fixes_as_real_ones() {
        let unread = finding("intra", "unknown", None, "dev-edge-a could not be read: 2 component(s) unverified (db-schema, portal)", None);
        let rows = unread_rows(&unread).unwrap();
        assert_eq!(rows[0], ["unknown", "dev", "db-schema", "dev-edge-a", "unreadable", "-"]);
        assert_eq!(rows[1][2], "portal");
        let across = finding("inter", "drift", Some("portal"), "dev declares portal:v1.4.0@sha256:3c, preprod declares portal:v1.3.2@sha256:9f; no open ladder record covers the difference", None);
        assert_eq!(across_line(&across), "portal: dev declares v1.4.0, preprod declares v1.3.2. No open release covers the difference.");
        let a = finding("intra", "drift", Some("portal"), "x", Some("goatbuilder deploy --component portal --env dev   (or declare what runs: goatbuilder adopt --env dev)"));
        let b = finding("intra", "drift", Some("portal"), "y", Some("goatbuilder deploy --component portal --env dev   (or declare what runs: goatbuilder adopt --env dev)"));
        let (commands, adopts) = fix_lines(&[&a, &b]);
        assert_eq!(commands, ["goatbuilder deploy --component portal --env dev"]);
        assert_eq!(adopts, ["goatbuilder adopt --env dev"]);
    }
}
