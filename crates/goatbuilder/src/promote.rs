//! `promote`: guide and observe release graduation across the goatnet ladder
//! — the verb that replaces ops/release/graduate.py behind the parity gate
//! (ops/goatbuilder/parity-promote.sh). Canon: docs/design/release-graduation.md
//! (ADR 1120), graduation sets (ADR 1125), daily review (ADR 1177).
//!
//! It reads the ladder (ops/release/ladder.yaml), the pin table
//! (ops/pins/fleet-pins.yaml) and the promotion records (ops/release/records/).
//! It never rewrites fleet-pins.yaml — that file's comments are load-bearing
//! review context — so a pin edit is PRINTED for the PR that owns it, and
//! `check` refuses trees where records and pins disagree.
//!
//! Subcommands, the script's: status, open, graduate, graduate-set, hotfix,
//! review, readiness, abandon, check. Every line a person reads is the script's
//! line; every refusal is the script's refusal with its exit code (1). Nothing
//! graduates by itself: a mutation runs only when invoked, prints what it did,
//! and without a terminal needs --yes (C10).
//!
//! Typed edits for the other verbs: `release` and `fastlane` call `open`,
//! `graduate`, `hotfix`, `abandon` and `open_set_for` here instead of
//! shelling out to the script.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use clap::Subcommand;
use serde_json::{json, Map, Value};

use crate::pins::{is_digest, py_str, read_yaml};
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

const KNOWN_GATES: [&str; 6] = ["candidate-record", "soak", "convergence-evidence", "validation-evidence", "operator-ack", "runtime-health"];
/// runtime-health: the peak-memory ceiling, as a fraction of the container
/// limit, when the gate entry in ladder.yaml sets no max_memory_fraction.
const RUNTIME_HEALTH_MAX_MEMORY_FRACTION: f64 = 0.8;
const RECORD_STATES: [&str; 3] = ["candidate", "graduated", "abandoned"];
const HOTFIX_BACKFILL_MAX_DAYS: i64 = 3;
const HOTFIX_BACKFILL_CUTOFF: (i32, u32, u32) = (2026, 9, 9);
const ABANDON_REASON_CUTOFF: &str = "2026-08-13";
const SET_STATES: [&str; 3] = ["open", "graduated", "abandoned"];
const CLASSES: [&str; 3] = ["ladder", "env-local", "exempt"];
const GRADE_ORDER: [&str; 5] = ["UNDECLARED-SKEW", "hotfix-backfill-owed", "candidate", "legacy-declared", "aligned"];
const READINESS_SCHEMA: &str = "release-readiness-v1";

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// Cross-rung matrix: per component, the effective value on each live rung, graded.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Start a candidate: write the record, print the preprod deviation edit.
    Open {
        component: String,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long)]
        reason: String,
    },
    /// Verify the hop gates, mark the record graduated, print the fold-to-fleet edit.
    Graduate {
        component: String,
        #[arg(long, value_parser = crate::parse_ack)]
        ack: Option<String>,
        /// Graduate although the runtime-health gate fails; REASON and the --ack name are written into the record.
        #[arg(long = "override-runtime-health", value_name = "REASON")]
        override_runtime_health: Option<String>,
    },
    /// Graduate every member of a set in one act, or refuse and name every member that is not ready.
    GraduateSet {
        name: String,
        #[arg(long, value_parser = crate::parse_ack)]
        ack: Option<String>,
    },
    /// Expedited prod path: record + prod deviation edit; owes a backfill.
    Hotfix {
        component: String,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long)]
        reason: String,
        #[arg(long, value_parser = crate::parse_ack)]
        ack: Option<String>,
        /// The ref being moved away from; required when fleet: has already been rewritten.
        #[arg(long = "previous-ref")]
        previous_ref: Option<String>,
    },
    /// The start-of-day / end-of-day review: every open record sorted by what a person must DO. Read-only.
    Review {
        #[arg(long)]
        json: bool,
        /// Your name, filled into the printed graduate commands.
        #[arg(long)]
        ack: Option<String>,
        /// The narrative per item: what it is, if left, if acted, command.
        #[arg(long)]
        detail: bool,
        /// Omit the REVIEW line (a wrapper prints its own).
        #[arg(long = "no-header")]
        no_header: bool,
        /// Limit to one component.
        component: Option<String>,
    },
    /// The `release-readiness-v1` document. Read-only, derived, no verdict of its own.
    Readiness {
        /// Write the document here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Mark a record abandoned WITH a machine-readable reason (C8).
    Abandon {
        record: String,
        #[arg(long)]
        reason: String,
        #[arg(long = "superseded-by")]
        superseded_by: Option<String>,
    },
    /// CI invariants C1–C10 (fail-closed, current-tree). Exit 0/1.
    Check,
}

// --- output: the script's stdout / stderr, as report rows -------------------------------

pub struct Out {
    pub lines: Vec<Check>,
    pub raw: Option<String>,
}

impl Out {
    fn new() -> Self {
        Out { lines: Vec::new(), raw: None }
    }
    fn out(&mut self, s: impl Into<String>) {
        self.lines.push(Check { name: "note".into(), ok: true, detail: s.into(), fix: None });
    }
    fn err(&mut self, s: impl Into<String>) {
        self.lines.push(Check { name: "refused".into(), ok: false, detail: s.into(), fix: None });
    }
}

fn die(msg: impl Into<String>) -> anyhow::Error {
    goat_errors::coded("GB-033", format!("graduate: {}", msg.into())).into()
}

// --- the tree -----------------------------------------------------------------------------

pub struct Tree {
    pub repo: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub path: PathBuf,
    pub doc: Map<String, Value>,
    pub error: Option<String>,
}

impl Record {
    fn get(&self, k: &str) -> Option<&Value> {
        self.doc.get(k)
    }
    fn s(&self, k: &str) -> Option<String> {
        self.doc.get(k).filter(|v| !v.is_null()).map(py_str)
    }
    fn class(&self) -> String {
        self.s("class").unwrap_or_else(|| "ladder".into())
    }
    fn evidence(&self, k: &str) -> bool {
        self.doc.get("evidence").and_then(|e| e.get(k)).map(truthy).unwrap_or(false)
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn s_of(v: Option<&Value>) -> Option<String> {
    v.filter(|x| !x.is_null()).map(py_str)
}

/// Python's `str(x)` of a `.get()` that may be None.
fn py_opt(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(x) => py_str(x),
    }
}

pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

impl Tree {
    pub fn new(repo: &Path) -> Self {
        Tree { repo: repo.to_path_buf() }
    }
    fn ladder_path(&self) -> PathBuf {
        self.repo.join("ops/release/ladder.yaml")
    }
    fn pins_path(&self) -> PathBuf {
        self.repo.join("ops/pins/fleet-pins.yaml")
    }
    fn records_dir(&self) -> PathBuf {
        self.repo.join("ops/release/records")
    }
    fn sets_dir(&self) -> PathBuf {
        self.repo.join("ops/release/sets")
    }
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.repo).unwrap_or(p).display().to_string()
    }

    /// (ladder, rungs, live rungs)
    pub fn load_ladder(&self) -> Result<(Value, Vec<Value>, Vec<Value>)> {
        let ladder = read_yaml(&self.ladder_path())?;
        let rungs: Vec<Value> = get(&ladder, "rungs").and_then(|r| r.as_array()).cloned().unwrap_or_default();
        let live: Vec<Value> = rungs.iter().filter(|r| get(r, "status").and_then(|s| s.as_str()) == Some("live")).cloned().collect();
        if live.is_empty() {
            return Err(die("ladder has no live rungs"));
        }
        Ok((ladder, rungs, live))
    }

    pub fn load_pins(&self) -> Result<Value> {
        read_yaml(&self.pins_path())
    }

    pub fn load_records(&self) -> Vec<Record> {
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.records_dir()) {
            for d in rd.flatten() {
                if d.path().is_dir() {
                    if let Ok(files) = std::fs::read_dir(d.path()) {
                        for f in files.flatten() {
                            if f.path().extension().map(|e| e == "yaml").unwrap_or(false) {
                                paths.push(f.path());
                            }
                        }
                    }
                }
            }
        }
        paths.sort();
        paths.into_iter().map(|p| self.read_record(&p)).collect()
    }

    fn read_record(&self, p: &Path) -> Record {
        match read_yaml(p) {
            Ok(Value::Object(doc)) => Record { path: p.to_path_buf(), doc, error: None },
            Ok(_) => Record { path: p.to_path_buf(), doc: Map::new(), error: None },
            Err(e) => Record { path: p.to_path_buf(), doc: Map::new(), error: Some(e.to_string()) },
        }
    }

    pub fn load_sets(&self) -> Vec<Record> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(self.sets_dir())
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "yaml").unwrap_or(false)).collect())
            .unwrap_or_default();
        paths.sort();
        paths.into_iter().map(|p| self.read_record(&p)).collect()
    }
}

fn latest_record<'a>(records: &'a [Record], comp: &str, state: Option<&str>, reference: Option<&str>, cls: Option<&str>) -> Option<&'a Record> {
    records.iter().rfind(|r| {
        r.error.is_none()
            && r.s("component").as_deref() == Some(comp)
            && state.map(|s| r.s("state").as_deref() == Some(s)).unwrap_or(true)
            && reference.map(|x| r.s("candidate_ref").as_deref() == Some(x)).unwrap_or(true)
            && cls.map(|c| r.class() == c).unwrap_or(true)
    })
}

/// Resolution for a goatnet: goatnet deviation -> fleet. Returns (state, value).
fn effective(comp_def: &Value, goatnet: &str) -> (&'static str, Option<Value>) {
    let dev = get(comp_def, "deviations").and_then(|d| d.get(goatnet));
    if let Some(dev) = dev.filter(|d| truthy(d)) {
        if get(dev, "absent").map(truthy).unwrap_or(false) {
            return ("absent", None);
        }
        if let Some(v) = get(dev, "value") {
            return ("value", Some(v.clone()));
        }
    }
    match get(comp_def, "fleet") {
        None | Some(Value::Null) => ("unresolved", None),
        Some(f) => ("value", Some(f.clone())),
    }
}

fn set_members(doc: &Map<String, Value>) -> Vec<Map<String, Value>> {
    doc.get("members").and_then(|m| m.as_array()).map(|a| a.iter().filter_map(|m| m.as_object().cloned()).collect()).unwrap_or_default()
}

/// The OPEN set this component belongs to, or None.
pub fn open_set_for<'a>(sets: &'a [Record], component: &str) -> Option<&'a Record> {
    sets.iter().find(|d| d.error.is_none() && d.s("state").as_deref() == Some("open") && set_members(&d.doc).iter().any(|m| s_of(m.get("component")).as_deref() == Some(component)))
}

fn gate_id(g: &Value) -> String {
    match g {
        Value::Object(o) => s_of(o.get("id")).unwrap_or_default(),
        other => py_str(other),
    }
}

fn gates(hop: &Value) -> Vec<Value> {
    get(hop, "gates").and_then(|g| g.as_array()).cloned().unwrap_or_default()
}

/// (max_memory_fraction, min_days, require_memory_limit) for one runtime-health
/// gate entry. min_days defaults to the hop's own soak gate, so the capture is
/// taken no earlier than the soak's end; a hop with no soak gate waits one day.
fn runtime_health_settings(hop: &Value, gate: &Value) -> (Value, i64, bool) {
    let soak_days = gates(hop).into_iter().find(|x| gate_id(x) == "soak").map(|s| get(&s, "min_days").and_then(|m| m.as_i64()).unwrap_or(1)).unwrap_or(1);
    let max = get(gate, "max_memory_fraction").cloned().unwrap_or_else(|| json!(RUNTIME_HEALTH_MAX_MEMORY_FRACTION));
    let min_days = get(gate, "min_days").and_then(|m| m.as_i64()).unwrap_or(soak_days);
    let require = get(gate, "require_memory_limit") != Some(&Value::Bool(false));
    (max, min_days, require)
}

/// The record's override for this rung when it names who AND why.
fn runtime_health_override(rec: &Record, rung: &str) -> Option<Map<String, Value>> {
    let o = rec.get("runtime_health_override").and_then(|o| o.as_object())?.get(rung)?.as_object()?;
    let filled = |k: &str| o.get(k).filter(|v| truthy(v)).map(|v| !py_str(v).trim().is_empty()).unwrap_or(false);
    if filled("reason") && filled("by") {
        Some(o.clone())
    } else {
        None
    }
}

/// An ISO-8601 time from a record, as UTC; None when unreadable.
fn utc_time(v: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = v.filter(|v| !v.is_null()).map(py_str)?;
    let s = s.trim();
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&chrono::Utc));
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(t) = chrono::NaiveDateTime::parse_from_str(s, f) {
            return Some(t.and_utc());
        }
    }
    date(s).and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| t.and_utc())
}

/// Every reason one captured runtime_health block fails the gate — the
/// script's grade_runtime_health, line for line.
fn grade_runtime_health(block: Option<&Value>, rung: &str, candidate_ref: Option<&str>, max_fraction: &Value, min_days: i64, require_limit: bool) -> Vec<String> {
    let block = match block {
        None | Some(Value::Null) => return vec![format!("runtime-health awaiting: no runtime_health.{rung} block (ops/release/rung-health.py --write-record, at the soak's end)")],
        Some(b) => b,
    };
    if !block.is_object() {
        return vec![format!("runtime-health: runtime_health.{rung} is not a mapping")];
    }
    let mut fails = Vec::new();
    match (utc_time(get(block, "captured")), utc_time(get(block, "deployed"))) {
        (Some(c), Some(d)) => {
            let hours = (c - d).num_seconds().div_euclid(3600);
            if hours < min_days * 24 {
                fails.push(format!("runtime-health awaiting: runtime_health.{rung} was captured {hours} h after deploy, needs >= {} h (the soak)", min_days * 24));
            }
        }
        _ => fails.push(format!("runtime-health: runtime_health.{rung} has no readable captured/deployed time")),
    }
    if let Some(m) = regex::Regex::new(r"@sha256:[0-9a-f]{64}").unwrap().find(candidate_ref.unwrap_or("")) {
        let digest = &m.as_str()[1..];
        let have: Vec<String> = get(block, "digests").and_then(|d| d.as_array()).map(|a| a.iter().map(py_str).collect()).unwrap_or_default();
        if !have.iter().any(|d| d == digest) {
            fails.push(format!("runtime-health: runtime_health.{rung} was not captured on the candidate digest {}", &digest[7..19]));
        }
    }
    for (key, what) in [("restarts", "restarted"), ("oom_kills", "OOM-killed")] {
        match get(block, key).and_then(|n| n.as_i64()) {
            None => fails.push(format!("runtime-health: {rung} {key} unknown — the capture could not read it")),
            Some(n) if n > 0 => fails.push(format!("runtime-health: {rung} {what} {n} time(s) since deploy")),
            _ => {}
        }
    }
    match get(block, "peak_memory_fraction").filter(|f| f.is_number()) {
        None => {
            if require_limit {
                fails.push(format!("runtime-health: {rung} peak memory has no limit to grade against (set a memory limit, or require_memory_limit: false on the gate)"));
            }
        }
        Some(f) => {
            if f.as_f64().unwrap_or(0.0) > max_fraction.as_f64().unwrap_or(RUNTIME_HEALTH_MAX_MEMORY_FRACTION) {
                fails.push(format!("runtime-health: {rung} peak memory reached {} of its limit, ceiling {}", py_str(f), py_str(max_fraction)));
            }
        }
    }
    fails
}

/// The runtime-health gate for this hop: graded on the LOWER rung (the hop's
/// `from`), satisfied by a recorded override for that rung.
fn runtime_health_failures(rec: &Record, hop: &Value, gate: &Value) -> Vec<String> {
    let rung = s_of(get(hop, "from")).unwrap_or_default();
    if runtime_health_override(rec, &rung).is_some() {
        return Vec::new();
    }
    let (max, min_days, require) = runtime_health_settings(hop, gate);
    let block = rec.get("runtime_health").and_then(|b| b.as_object()).and_then(|m| m.get(&rung));
    grade_runtime_health(block, &rung, rec.s("candidate_ref").as_deref(), &max, min_days, require)
}

fn gate_failures(rec: &Record, hop: &Value, today: NaiveDate) -> Result<Vec<String>> {
    let mut fails = Vec::new();
    for g in gates(hop) {
        match gate_id(&g).as_str() {
            "candidate-record" => {
                if rec.s("state").as_deref() != Some("candidate") {
                    fails.push("record is not in state candidate".to_string());
                }
            }
            "soak" => {
                let mind = get(&g, "min_days").and_then(|m| m.as_i64()).unwrap_or(1);
                let opened_s = py_opt(rec.get("opened"));
                let opened = date(&opened_s).ok_or_else(|| anyhow!("Invalid isoformat string: '{opened_s}'"))?;
                if (today - opened).num_days() < mind {
                    fails.push(format!("soak: opened {opened}, needs >= {mind} day(s)"));
                }
            }
            "convergence-evidence" => {
                if !rec.evidence("preprod_convergence") {
                    fails.push("evidence.preprod_convergence is empty (name the goat health consistency band / console render / M-doc)".to_string());
                }
            }
            "validation-evidence" if !rec.evidence("validation") => {
                fails.push("evidence.validation is empty".to_string());
            }
            "runtime-health" => fails.extend(runtime_health_failures(rec, hop, &g)),
            _ => {}
        }
    }
    Ok(fails)
}

fn set_blockers(doc: &Map<String, Value>, records: &[Record], hop: &Value, today: NaiveDate) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for m in set_members(doc) {
        let comp = s_of(m.get("component")).unwrap_or_default();
        let reference = s_of(m.get("ref")).unwrap_or_default();
        match latest_record(records, &comp, Some("candidate"), Some(&reference), None) {
            None => out.push(format!("{comp}: no OPEN candidate record for the set's exact ref")),
            Some(rec) => {
                for f in gate_failures(rec, hop, today)? {
                    out.push(format!("{comp}: {f}"));
                }
            }
        }
    }
    Ok(out)
}

fn grade_pair(name: &str, lo_ref: &str, hi_ref: &str, records: &[Record], legacy: bool, top: bool) -> &'static str {
    if lo_ref == hi_ref {
        if let Some(rec) = latest_record(records, name, Some("candidate"), Some(hi_ref), None) {
            if rec.class() == "hotfix" {
                return "hotfix-backfill-owed";
            }
        }
        return "aligned";
    }
    if legacy {
        return "legacy-declared";
    }
    if let Some(rec) = latest_record(records, name, Some("candidate"), Some(lo_ref), None) {
        if rec.class() == "hotfix" {
            return "hotfix-backfill-owed";
        }
        return "candidate";
    }
    if let Some(rec) = latest_record(records, name, Some("candidate"), Some(hi_ref), None) {
        if top || rec.class() == "hotfix" {
            return "hotfix-backfill-owed";
        }
        return "candidate";
    }
    "UNDECLARED-SKEW"
}

fn rung_name(r: &Value) -> String {
    s_of(get(r, "name")).unwrap_or_default()
}
fn rung_goatnet(r: &Value) -> String {
    s_of(get(r, "goatnet")).unwrap_or_default()
}

fn grade_component(name: &str, comp_def: &Value, live: &[Value], records: &[Record], overrides: &Value, allow: &Value) -> Map<String, Value> {
    let cls = get(overrides, name).and_then(|o| get(o, "class")).map(py_str).unwrap_or_else(|| "ladder".into());
    let mut row = Map::new();
    row.insert("component".into(), json!(name));
    row.insert("class".into(), json!(cls));
    if cls == "exempt" {
        row.insert("grade".into(), json!("exempt"));
        row.insert("rungs".into(), json!({}));
        row.insert("pairs".into(), json!({}));
        return row;
    }
    let mut per_rung: Map<String, Value> = Map::new();
    let mut vals: Vec<(String, String)> = Vec::new();
    for r in live {
        let (st, val) = effective(comp_def, &rung_goatnet(r));
        per_rung.insert(rung_name(r), json!({"state": st, "value": val.clone().unwrap_or(Value::Null)}));
        if st == "value" {
            vals.push((rung_name(r), val.map(|v| py_str(&v)).unwrap_or_default()));
        }
    }
    if vals.len() < 2 {
        let grade = if per_rung.values().any(|v| get(v, "state").and_then(|s| s.as_str()) == Some("absent")) { "env-local" } else { "single-rung" };
        row.insert("grade".into(), json!(grade));
        row.insert("rungs".into(), Value::Object(per_rung));
        row.insert("pairs".into(), json!({}));
        return row;
    }
    let top_hop = (rung_name(&live[live.len() - 2]), rung_name(&live[live.len() - 1]));
    let mut pairs: Map<String, Value> = Map::new();
    let in_allow = get(allow, name).is_some();
    for w in vals.windows(2) {
        let (lo, hi) = (&w[0], &w[1]);
        let legacy = in_allow && (lo.0.clone(), hi.0.clone()) == top_hop;
        let g = grade_pair(name, &lo.1, &hi.1, records, legacy, hi.0 == top_hop.1);
        pairs.insert(format!("{}->{}", lo.0, hi.0), json!(g));
    }
    let grade = GRADE_ORDER.iter().find(|g| pairs.values().any(|v| v.as_str() == Some(g))).copied().unwrap_or("aligned");
    row.insert("grade".into(), json!(grade));
    row.insert("rungs".into(), Value::Object(per_rung));
    row.insert("pairs".into(), Value::Object(pairs));
    row
}

pub struct Status {
    pub ladder: Value,
    pub live: Vec<Value>,
    pub records: Vec<Record>,
    pub rows: Vec<Map<String, Value>>,
}

pub fn build_status(t: &Tree) -> Result<Status> {
    let (ladder, _rungs, live) = t.load_ladder()?;
    let pins = t.load_pins()?;
    let records = t.load_records();
    let overrides = get(&ladder, "class_overrides").cloned().filter(|v| v.is_object()).unwrap_or(json!({}));
    let allow = get(&ladder, "legacy_skew_allowlist").cloned().filter(|v| v.is_object()).unwrap_or(json!({}));
    let mut rows = Vec::new();
    if let Some(comps) = get(&pins, "components").and_then(|c| c.as_object()) {
        for (name, comp_def) in comps {
            rows.push(grade_component(name, comp_def, &live, &records, &overrides, &allow));
        }
    }
    Ok(Status { ladder, live, records, rows })
}

/// Python's `json.dumps(indent=2)` with its default `ensure_ascii=True`.
pub fn json_dumps(v: &Value) -> String {
    let text = serde_json::to_string_pretty(v).unwrap_or_default();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if (c as u32) < 0x80 {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{:04x}", u));
            }
        }
    }
    out
}

fn short_ref(cell: &Value) -> String {
    let state = get(cell, "state").and_then(|s| s.as_str()).unwrap_or("");
    let v = get(cell, "value").filter(|v| !v.is_null()).map(py_str);
    if state == "absent" {
        return "absent".into();
    }
    match v {
        None => "-".into(),
        Some(v) => {
            // Ten hexadecimal characters: enough to tell two digests apart by
            // eye, and the three columns then fit an 80-column terminal beside
            // the component and its grade. `-o json` carries every digest whole.
            if v.contains("@sha256:") {
                v.rsplit("@sha256:").next().unwrap_or("").chars().take(10).collect()
            } else {
                // No digest: the tag is what distinguishes it. The last ten
                // characters of the whole ref cut a tag in half
                // (`grant-keyd:sha-0751afc` read as `ha-0751afc`), and a
                // `sha-` prefix is a convention, not part of the identifier.
                let tag = v.rsplit(':').next().unwrap_or(&v);
                tag.strip_prefix("sha-").unwrap_or(tag).chars().take(10).collect()
            }
        }
    }
}

fn cmd_status(t: &Tree, json: bool, o: &mut Out) -> Result<i32> {
    let st = build_status(t)?;
    if json {
        o.raw = Some(json_dumps(&Value::Array(st.rows.iter().map(|r| Value::Object(r.clone())).collect())));
        return Ok(0);
    }
    let rung_names: Vec<String> = st.live.iter().map(rung_name).collect();
    let w = st.rows.iter().map(|r| s_of(r.get("component")).unwrap_or_default().chars().count()).max().unwrap_or(12);
    // The grade column is as wide as the grades this repository actually has,
    // not as wide as the longest grade there could ever be.
    let g = st.rows.iter().map(|r| s_of(r.get("grade")).unwrap_or_default().chars().count()).max().unwrap_or(5).max(5);
    let mut header = format!("{:<w$}  {:<g$}", "component", "grade", w = w, g = g);
    for n in &rung_names {
        header.push_str(&format!("  {:<10}", n));
    }
    let len = header.chars().count();
    o.out(header);
    o.out("-".repeat(len));
    let mut rows = st.rows.clone();
    rows.sort_by_key(|r| s_of(r.get("component")).unwrap_or_default());
    for r in &rows {
        let mut line = format!("{:<w$}  {:<g$}", s_of(r.get("component")).unwrap_or_default(), s_of(r.get("grade")).unwrap_or_default(), w = w, g = g);
        for n in &rung_names {
            let cell = r.get("rungs").and_then(|x| x.get(n)).cloned().unwrap_or(json!({}));
            line.push_str(&format!("  {:<10}", short_ref(&cell)));
        }
        o.out(line);
    }
    let open: Vec<&Record> = st.records.iter().filter(|r| r.error.is_none() && r.s("state").as_deref() == Some("candidate")).collect();
    if !open.is_empty() {
        o.out(String::new());
        o.out(format!("open records ({}):", open.len()));
        for r in open {
            let cref = py_opt(r.get("candidate_ref"));
            let n = cref.chars().count();
            let tail: String = cref.chars().skip(n.saturating_sub(19)).collect();
            o.out(format!("  {}: {} opened {} ref …{tail}  ({})", r.s("component").unwrap_or_default(), r.class(), py_opt(r.get("opened")), t.rel(&r.path)));
        }
    }
    Ok(0)
}

fn preprod_prod(live: &[Value]) -> Result<(Value, Value)> {
    let pre = live.iter().find(|r| rung_name(r) == "preprod");
    let prod = live.iter().find(|r| rung_name(r) == "prod");
    match (pre, prod) {
        (Some(a), Some(b)) => Ok((a.clone(), b.clone())),
        _ => Err(die("ladder must have live preprod and prod rungs")),
    }
}

fn hop_preprod_prod(ladder: &Value) -> Result<Value> {
    get(ladder, "hops")
        .and_then(|h| h.as_array())
        .and_then(|hs| hs.iter().find(|h| get(h, "from").and_then(|f| f.as_str()) == Some("preprod") && get(h, "to").and_then(|f| f.as_str()) == Some("prod")).cloned())
        .ok_or_else(|| die("no preprod->prod hop in ladder"))
}

fn needs_ack(hop: &Value) -> bool {
    gates(hop).iter().any(|g| gate_id(g) == "operator-ack")
}

// --- writing records ---------------------------------------------------------------------------

fn ymap(pairs: Vec<(&str, Value)>) -> serde_yaml::Mapping {
    let mut m = serde_yaml::Mapping::new();
    for (k, v) in pairs {
        m.insert(serde_yaml::Value::String(k.into()), serde_yaml::to_value(v).unwrap_or(serde_yaml::Value::Null));
    }
    m
}

/// The record as YAML, in field order, dates and timestamps quoted as the
/// script's dumper writes them (`opened: ''`, `captured: '…T…Z'`) at any
/// depth — unquoted, the script's loader would read them back as dates.
pub fn dump_record(doc: &Map<String, Value>) -> String {
    let m = ymap(doc.iter().map(|(k, v)| (k.as_str(), v.clone())).collect());
    let text = serde_yaml::to_string(&m).unwrap_or_default();
    let re = regex::Regex::new(r"^(\s*[A-Za-z_][A-Za-z0-9_]*): (\d{4}-\d{2}-\d{2}(?:[Tt ]\d{1,2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)?)$").unwrap();
    text.lines().map(|l| match re.captures(l) {
        Some(c) => format!("{}: '{}'", &c[1], &c[2]),
        None => l.to_string(),
    }).collect::<Vec<_>>().join("\n") + "\n"
}

fn write_record(path: &Path, doc: &Map<String, Value>) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, dump_record(doc)).map_err(|e| anyhow!("{}: {e}", path.display()))
}

fn ref_short(reference: &str) -> String {
    reference.rsplit(':').next().unwrap_or("").chars().take(12).collect()
}

fn component_def<'a>(pins: &'a Value, component: &str) -> Result<&'a Value> {
    get(pins, "components").and_then(|c| c.get(component)).filter(|c| truthy(c)).ok_or_else(|| die(format!("unknown component {}", py_repr(Some(&json!(component))))))
}

fn cmd_open(t: &Tree, component: &str, reference: &str, reason: &str, o: &mut Out) -> Result<i32> {
    let (_ladder, _, live) = t.load_ladder()?;
    let pins = t.load_pins()?;
    let comp = component_def(&pins, component)?;
    if !is_digest(reference) {
        return Err(die("candidate ref must be digest-pinned (…@sha256:…) — same rule as fleet-pins"));
    }
    let (pre, _prod) = preprod_prod(&live)?;
    let today = today().to_string();
    let path = t.records_dir().join(component).join(format!("{}-{}.yaml", today.replace('-', ""), ref_short(reference)));
    let mut rec = Map::new();
    rec.insert("component".into(), json!(component));
    rec.insert("class".into(), json!("ladder"));
    rec.insert("candidate_ref".into(), json!(reference));
    rec.insert("previous_fleet_ref".into(), get(comp, "fleet").cloned().unwrap_or(Value::Null));
    rec.insert("opened".into(), json!(today));
    rec.insert("reason".into(), json!(reason));
    rec.insert("evidence".into(), json!({"validation": "", "preprod_convergence": ""}));
    rec.insert("state".into(), json!("candidate"));
    rec.insert("graduated".into(), Value::Null);
    rec.insert("operator_ack".into(), Value::Null);
    if rec["previous_fleet_ref"] == rec["candidate_ref"] {
        return Err(die("previous_fleet_ref would equal candidate_ref — the record's rollback pointer would name the ref being deployed. Open the record BEFORE moving fleet:."));
    }
    write_record(&path, &rec)?;
    o.out(format!("record written: {}", t.rel(&path)));
    o.out(format!("\nnow make this edit in ops/pins/fleet-pins.yaml under components.{component}:"));
    o.out(format!("    deviations:\n      {}:\n        value: {reference}\n        reason: \"RC ({today}): {reason} — record {}\"", rung_goatnet(&pre), path.file_name().unwrap_or_default().to_string_lossy()));
    o.out("\nthen: goatbuilder pins && deploy to preprod && fill evidence fields in the record as it soaks. `promote check` gates the PR.");
    Ok(0)
}

/// `graduate <component> --ack NAME`. `from_set` is the set path's internal call.
/// `override_runtime_health` is `--override-runtime-health REASON`.
pub fn cmd_graduate(t: &Tree, component: &str, ack: Option<&str>, override_runtime_health: Option<&str>, from_set: bool, o: &mut Out) -> Result<i32> {
    let (ladder, _, live) = t.load_ladder()?;
    let records = t.load_records();
    let mut rec = match latest_record(&records, component, Some("candidate"), None, Some("ladder")) {
        Some(r) => r.clone(),
        None => {
            let r = latest_record(&records, component, Some("candidate"), None, Some("hotfix")).ok_or_else(|| die(format!("no open ladder candidate or hotfix record for {component}")))?.clone();
            o.out(format!("graduate: closing the fastlane backfill on {} (hotfix record; the same preprod->prod gates apply)", t.rel(&r.path)));
            r
        }
    };
    let (pre, prod) = preprod_prod(&live)?;
    let hop = hop_preprod_prod(&ladder)?;
    let sets = t.load_sets();
    if let Some(owning) = open_set_for(&sets, component) {
        if !from_set {
            o.err(format!("graduate: REFUSING — {component} is a member of the OPEN graduation set '{}' ({}).", owning.s("name").unwrap_or_default(), t.rel(&owning.path)));
            o.err("  The set moves together or not at all; graduating one member is how a validated train arrives in pieces.");
            o.err(format!("  Members: {}", set_members(&owning.doc).iter().map(|m| s_of(m.get("component")).unwrap_or_else(|| "?".into())).collect::<Vec<_>>().join(", ")));
            o.err(format!("\n  graduate the set:  goatbuilder promote graduate-set {} --ack '<name>'", owning.s("name").unwrap_or_default()));
            o.err("  or take this component out of the set (edit the file, in a reviewed PR) if it genuinely moves alone.");
            return Ok(1);
        }
    }
    if let Some(reason) = override_runtime_health {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(die("--override-runtime-health needs a reason"));
        }
        let by = match ack.filter(|a| !a.is_empty()) {
            Some(a) => a,
            None => return Err(die("--override-runtime-health needs --ack '<name>' — an override names who decided")),
        };
        let rt: Vec<String> = gates(&hop).iter().filter(|g| gate_id(g) == "runtime-health").flat_map(|g| runtime_health_failures(&rec, &hop, g)).collect();
        if !rt.is_empty() {
            let rung = s_of(get(&hop, "from")).unwrap_or_default();
            let mut ov = rec.doc.get("runtime_health_override").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            ov.insert(rung.clone(), json!({"by": by, "reason": reason, "recorded": today().to_string(), "failures": rt}));
            rec.doc.insert("runtime_health_override".into(), Value::Object(ov));
            o.out(format!("graduate: runtime-health overridden for {rung} by {by}: {reason}"));
        }
    }
    let mut fails = gate_failures(&rec, &hop, today())?;
    if needs_ack(&hop) && ack.map(|a| a.is_empty()).unwrap_or(true) {
        fails.push("operator-ack gate: pass --ack '<name>'".into());
    }
    if !fails.is_empty() {
        o.err(format!("graduate: {component} is NOT ready:"));
        for f in fails {
            o.err(format!("  - {f}"));
        }
        return Ok(1);
    }
    let mut doc = rec.doc.clone();
    doc.insert("state".into(), json!("graduated"));
    doc.insert("graduated".into(), json!(today().to_string()));
    doc.insert("operator_ack".into(), ack.map(|a| json!(a)).unwrap_or(Value::Null));
    write_record(&rec.path, &doc)?;
    o.out(format!("record graduated: {}", t.rel(&rec.path)));
    o.out(format!("\nnow make this edit in ops/pins/fleet-pins.yaml under components.{component}:"));
    o.out(format!("    fleet: {}", py_opt(rec.get("candidate_ref"))));
    o.out(format!("    # and DELETE the {} RC deviation added by `open` (and any {} hotfix deviation for this ref)", rung_goatnet(&pre), rung_goatnet(&prod)));
    o.out("\nthen: goatbuilder pins && PR && deploy prod && verify with the consistency surface. `check` refuses a tree where the graduated record and fleet value disagree (C5).");
    Ok(0)
}

fn cmd_graduate_set(t: &Tree, name: &str, ack: Option<&str>, o: &mut Out) -> Result<i32> {
    let (ladder, _, live) = t.load_ladder()?;
    let records = t.load_records();
    let sets = t.load_sets();
    let doc = sets.iter().find(|d| d.error.is_none() && d.s("name").as_deref() == Some(name)).ok_or_else(|| die(format!("no graduation set named {} in ops/release/sets/", py_repr(Some(&json!(name))))))?.clone();
    if doc.s("state").as_deref() != Some("open") {
        return Err(die(format!("set {} is {}, not open", py_repr(Some(&json!(name))), py_repr(doc.get("state")))));
    }
    let (pre, _prod) = preprod_prod(&live)?;
    let hop = hop_preprod_prod(&ladder)?;
    let td = today();
    let mut blockers = set_blockers(&doc.doc, &records, &hop, td)?;
    if needs_ack(&hop) && ack.map(|a| a.is_empty()).unwrap_or(true) {
        blockers.push("operator-ack gate: pass --ack '<name>' (one ack covers the set)".into());
    }
    if !blockers.is_empty() {
        o.err(format!("graduate-set: {name} is NOT ready ({} blocker(s)):", blockers.len()));
        for b in blockers {
            o.err(format!("  - {b}"));
        }
        return Ok(1);
    }
    for m in set_members(&doc.doc) {
        let comp = s_of(m.get("component")).unwrap_or_default();
        let reference = s_of(m.get("ref")).unwrap_or_default();
        let rec = latest_record(&records, &comp, Some("candidate"), Some(&reference), None).ok_or_else(|| die(format!("{comp}: no OPEN candidate record for the set's exact ref")))?;
        let mut d = rec.doc.clone();
        d.insert("state".into(), json!("graduated"));
        d.insert("graduated".into(), json!(td.to_string()));
        d.insert("operator_ack".into(), ack.map(|a| json!(a)).unwrap_or(Value::Null));
        d.insert("graduated_with_set".into(), json!(name));
        write_record(&rec.path, &d)?;
        o.out(format!("record graduated: {}", t.rel(&rec.path)));
    }
    let mut d = doc.doc.clone();
    d.insert("state".into(), json!("graduated"));
    d.insert("graduated".into(), json!(td.to_string()));
    d.insert("operator_ack".into(), ack.map(|a| json!(a)).unwrap_or(Value::Null));
    write_record(&doc.path, &d)?;
    o.out(format!("set graduated: {}", t.rel(&doc.path)));
    o.out("\nnow make these edits in ops/pins/fleet-pins.yaml, IN ONE COMMIT — splitting them re-creates the partial promotion this set prevents:");
    for m in set_members(&doc.doc) {
        o.out(format!("    components.{}.fleet: {}", s_of(m.get("component")).unwrap_or_default(), py_opt(m.get("ref"))));
    }
    o.out(format!("    # and DELETE each member's {} RC deviation", rung_goatnet(&pre)));
    o.out("\nthen: goatbuilder pins && PR && deploy prod && verify.");
    Ok(0)
}

pub fn cmd_hotfix(t: &Tree, component: &str, reference: &str, reason: &str, ack: Option<&str>, previous_ref: Option<&str>, o: &mut Out) -> Result<i32> {
    let (_ladder, _, live) = t.load_ladder()?;
    let pins = t.load_pins()?;
    let comp = component_def(&pins, component)?;
    if !is_digest(reference) {
        return Err(die("hotfix ref must be digest-pinned (…@sha256:…)"));
    }
    let ack = match ack {
        Some(a) if !a.is_empty() => a,
        _ => return Err(die("hotfix requires --ack '<name>' — it is a prod change")),
    };
    let (_pre, prod) = preprod_prod(&live)?;
    let today = today().to_string();
    let path = t.records_dir().join(component).join(format!("{}-hotfix-{}.yaml", today.replace('-', ""), ref_short(reference)));
    let mut rec = Map::new();
    rec.insert("component".into(), json!(component));
    rec.insert("class".into(), json!("hotfix"));
    rec.insert("candidate_ref".into(), json!(reference));
    rec.insert("previous_fleet_ref".into(), previous_ref.filter(|p| !p.is_empty()).map(|p| json!(p)).unwrap_or_else(|| get(comp, "fleet").cloned().unwrap_or(Value::Null)));
    rec.insert("opened".into(), json!(today));
    rec.insert("reason".into(), json!(reason));
    rec.insert("evidence".into(), json!({"validation": "", "preprod_convergence": ""}));
    rec.insert("state".into(), json!("candidate"));
    rec.insert("graduated".into(), Value::Null);
    rec.insert("operator_ack".into(), json!(ack));
    if rec["previous_fleet_ref"] == rec["candidate_ref"] {
        return Err(die("previous_fleet_ref would equal candidate_ref — the record's rollback pointer would name the ref being deployed. Pass --previous-ref with the ref you moved away from."));
    }
    write_record(&path, &rec)?;
    o.out(format!("hotfix record written: {} (backfill owed — status shows hotfix-backfill-owed until this ref graduates normally)", t.rel(&path)));
    o.out(format!("\nnow make this edit in ops/pins/fleet-pins.yaml under components.{component}:"));
    o.out(format!("    deviations:\n      {}:\n        value: {reference}\n        reason: \"HOTFIX ({today}, ack {ack}): {reason} — record {}\"", rung_goatnet(&prod), path.file_name().unwrap_or_default().to_string_lossy()));
    o.out("\nbackfill: deploy the same ref to preprod, fill evidence, then `promote graduate` folds it to fleet and clears both deviations.");
    Ok(0)
}

pub fn cmd_abandon(t: &Tree, record: &str, reason: &str, superseded_by: Option<&str>, o: &mut Out) -> Result<i32> {
    let mut path = PathBuf::from(record);
    if !path.is_absolute() {
        path = t.repo.join(record);
    }
    if !path.exists() {
        return Err(die(format!("no such record: {record}")));
    }
    let rec = t.read_record(&path);
    if let Some(e) = &rec.error {
        return Err(die(format!("{} does not parse: {e}", path.display())));
    }
    if rec.s("state").as_deref() == Some("graduated") {
        return Err(die("refusing to abandon a record that already graduated — that is history, not a candidate"));
    }
    let mut doc = rec.doc.clone();
    doc.insert("state".into(), json!("abandoned"));
    doc.insert("abandoned_reason".into(), json!(reason));
    if let Some(sb) = superseded_by.filter(|s| !s.is_empty()) {
        doc.insert("superseded_by".into(), json!(sb));
    }
    write_record(&path, &doc)?;
    o.out(format!("abandoned: {}", t.rel(&path)));
    o.out(format!("  abandoned_reason: {reason}"));
    if let Some(sb) = superseded_by.filter(|s| !s.is_empty()) {
        o.out(format!("  superseded_by:    {sb}"));
    }
    Ok(0)
}

// --- check (C1–C10) -------------------------------------------------------------------------------

/// Every failure line, or an Err for the C1 case the script reports as its own line.
pub fn check_failures(t: &Tree) -> Result<(Vec<String>, usize, usize, usize, usize)> {
    let mut failures: Vec<String> = Vec::new();
    let (ladder, _rungs, live) = match t.load_ladder() {
        Ok(x) => x,
        Err(e) => return Err(goat_errors::coded("GB-037", format!("C1 FAIL: ladder.yaml unreadable: {}", e.to_string().trim_start_matches("graduate: "))).into()),
    };
    let pins = t.load_pins()?;
    let goatnets: Vec<String> = get(&pins, "goatnets").and_then(|g| g.as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for r in &live {
        if !goatnets.contains(&rung_goatnet(r)) {
            failures.push(format!("C1: live rung {} names goatnet {} not present in fleet-pins.yaml", py_repr(get(r, "name")), py_repr(get(r, "goatnet"))));
        }
    }
    for h in get(&ladder, "hops").and_then(|h| h.as_array()).cloned().unwrap_or_default() {
        for g in gates(&h) {
            let gid = gate_id(&g);
            if !KNOWN_GATES.contains(&gid.as_str()) {
                failures.push(format!("C1: unknown gate id {}", py_repr(Some(&json!(gid)))));
            }
            if gid == "runtime-health" && g.is_object() {
                let hop_name = format!("{}->{}", py_opt(get(&h, "from")), py_opt(get(&h, "to")));
                let frac = get(&g, "max_memory_fraction").cloned().unwrap_or_else(|| json!(RUNTIME_HEALTH_MAX_MEMORY_FRACTION));
                if !frac.as_f64().map(|f| f > 0.0 && f <= 1.0).unwrap_or(false) {
                    failures.push(format!("C1: hop {hop_name} runtime-health max_memory_fraction must be a number in (0, 1]"));
                }
                let md = get(&g, "min_days").cloned().unwrap_or_else(|| json!(1));
                if !md.as_i64().map(|d| d >= 1).unwrap_or(false) {
                    failures.push(format!("C1: hop {hop_name} runtime-health min_days must be a whole number of days >= 1"));
                }
            }
        }
    }
    if let Some(Value::Object(ov)) = get(&ladder, "class_overrides") {
        for (name, o) in ov {
            if !CLASSES.contains(&s_of(get(o, "class")).unwrap_or_default().as_str()) {
                failures.push(format!("C1: class_overrides.{name} has unknown class"));
            }
            if !get(o, "reason").map(truthy).unwrap_or(false) {
                failures.push(format!("C1: class_overrides.{name} missing reason"));
            }
        }
    }
    if let Some(Value::Object(al)) = get(&ladder, "legacy_skew_allowlist") {
        for (name, row) in al {
            if !get(row, "reason").map(truthy).unwrap_or(false) {
                failures.push(format!("C1: legacy_skew_allowlist.{name} missing reason"));
            }
        }
    }
    // C2 — records
    let records = t.load_records();
    let comp_names: Vec<String> = get(&pins, "components").and_then(|c| c.as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for rec in &records {
        let rp = t.rel(&rec.path);
        if let Some(e) = &rec.error {
            failures.push(format!("C2: {rp} unparseable: {e}"));
            continue;
        }
        if !comp_names.contains(&rec.s("component").unwrap_or_default()) {
            failures.push(format!("C2: {rp} names unknown component {}", py_repr(rec.get("component"))));
        }
        if !RECORD_STATES.contains(&rec.s("state").unwrap_or_default().as_str()) {
            failures.push(format!("C2: {rp} has unknown state {}", py_repr(rec.get("state"))));
        }
        if !is_digest_anywhere(&py_opt(rec.get("candidate_ref"))) {
            failures.push(format!("C2: {rp} candidate_ref is not digest-pinned"));
        }
        if rec.class() == "hotfix" && !rec.get("operator_ack").map(truthy).unwrap_or(false) {
            failures.push(format!("C2: {rp} hotfix record without operator_ack"));
        }
        if let Some(w) = rec.get("soak_waiver").filter(|w| !w.is_null()) {
            let ok_reason = w.as_object().map(|m| !s_of(m.get("reason")).unwrap_or_default().trim().is_empty()).unwrap_or(false);
            if !ok_reason {
                failures.push(format!("C2: {rp} soak_waiver must be a mapping carrying a non-empty reason — an unexplained waiver is worse than none"));
            } else if !w.as_object().map(|m| m.contains_key("by")).unwrap_or(false) {
                failures.push(format!("C2: {rp} soak_waiver names no `by:` — name the operator, or write `by: null` to state that the record cannot attribute it"));
            }
        }
        if let Some(ov) = rec.get("runtime_health_override").filter(|v| !v.is_null()) {
            match ov.as_object() {
                None => failures.push(format!("C2: {rp} runtime_health_override must map a rung to {{by, reason}}")),
                Some(m) => {
                    for rung in m.keys() {
                        if runtime_health_override(rec, rung).is_none() {
                            failures.push(format!("C2: {rp} runtime_health_override.{rung} must name a non-empty by and reason — an override nobody can attribute is worse than none"));
                        }
                    }
                }
            }
        }
    }
    // C3/C4 — skew must be declared
    let st = build_status(t)?;
    for row in &st.rows {
        if s_of(row.get("grade")).as_deref() == Some("UNDECLARED-SKEW") {
            let hops: Vec<String> = row.get("pairs").and_then(|p| p.as_object()).map(|m| m.iter().filter(|(_, g)| g.as_str() == Some("UNDECLARED-SKEW")).map(|(p, _)| p.clone()).collect()).unwrap_or_default();
            let cells: Vec<String> = row.get("rungs").and_then(|r| r.as_object()).map(|m| m.iter().map(|(n, c)| {
                let v = get(c, "value").filter(|v| truthy(v)).map(py_str).or_else(|| s_of(get(c, "state"))).unwrap_or_default();
                format!("'{n}': '{v}'")
            }).collect()).unwrap_or_default();
            failures.push(format!("C3/C4: {} cross-rung skew on {} has no record and no allowlist row: {{{}}}", s_of(row.get("component")).unwrap_or_default(), hops.join(", "), cells.join(", ")));
        }
    }
    // C5 — graduated records agree with fleet
    for name in &comp_names {
        let rec = match latest_record(&records, name, Some("graduated"), None, None) {
            Some(r) => r,
            None => continue,
        };
        let newer = records.iter().any(|r| r.error.is_none() && r.s("component").as_deref() == Some(name) && r.path.display().to_string() > rec.path.display().to_string());
        if newer {
            continue;
        }
        let fleet = get(&pins, "components").and_then(|c| c.get(name)).and_then(|c| get(c, "fleet")).filter(|f| !f.is_null()).map(py_str);
        if let Some(f) = fleet {
            if Some(f) != rec.s("candidate_ref") {
                failures.push(format!("C5: {name} latest graduated record ref differs from fleet value — fold not executed or superseded silently"));
            }
        }
    }
    // C6 — stale candidates
    let (pre, _) = preprod_prod(&live)?;
    let dev = live.iter().find(|r| rung_name(r) == "dev").cloned();
    let prod_goatnet = live.iter().find(|r| rung_name(r) == "prod").map(rung_goatnet).unwrap_or_default();
    for rec in &records {
        if rec.s("state").as_deref() != Some("candidate") || rec.error.is_some() {
            continue;
        }
        let comp_def = match get(&pins, "components").and_then(|c| c.get(rec.s("component").unwrap_or_default())) {
            Some(c) => c.clone(),
            None => continue,
        };
        let target_goatnet = if rec.class() != "hotfix" { rung_goatnet(&pre) } else { prod_goatnet.clone() };
        let (stt, val) = effective(&comp_def, &target_goatnet);
        if stt == "value" && val.map(|v| py_str(&v)) != rec.s("candidate_ref") {
            if rec.class() != "hotfix" {
                if let Some(d) = &dev {
                    let (dst, dval) = effective(&comp_def, &rung_goatnet(d));
                    if dst == "value" && dval.map(|v| py_str(&v)) == rec.s("candidate_ref") {
                        continue;
                    }
                }
            }
            failures.push(format!("C6: {} is open but {target_goatnet} now runs a different ref — abandon (state: abandoned) or refresh the record", t.rel(&rec.path)));
        }
    }
    // C9 — a hotfix's backfill is owed, not optional
    let td = today();
    let cutoff = NaiveDate::from_ymd_opt(HOTFIX_BACKFILL_CUTOFF.0, HOTFIX_BACKFILL_CUTOFF.1, HOTFIX_BACKFILL_CUTOFF.2).unwrap();
    for rec in &records {
        if rec.s("state").as_deref() != Some("candidate") || rec.class() != "hotfix" || rec.error.is_some() {
            continue;
        }
        let opened = match date(&py_opt(rec.get("opened"))) {
            Some(d) => d,
            None => continue,
        };
        if opened < cutoff {
            continue;
        }
        let age = (td - opened).num_days();
        if age > HOTFIX_BACKFILL_MAX_DAYS && !rec.evidence("validation") {
            failures.push(format!("C9: {} has owed its backfill for {age} days with evidence.validation empty — run the affected live rows, fill the evidence and `graduate`, or abandon it with a reason", t.rel(&rec.path)));
        }
    }
    // C10 — runtime health on a hop `graduate` does not run (the dev->preprod
    // move is a pin edit, so the PR that makes it is where the hop is held).
    for h in get(&ladder, "hops").and_then(|h| h.as_array()).cloned().unwrap_or_default() {
        if s_of(get(&h, "status")).as_deref() != Some("live") {
            continue;
        }
        let to = s_of(get(&h, "to")).unwrap_or_default();
        let upper = match live.iter().find(|r| rung_name(r) == to) {
            Some(u) => rung_goatnet(u),
            None => continue,
        };
        for g in gates(&h) {
            if gate_id(&g) != "runtime-health" {
                continue;
            }
            let since = get(&g, "records_opened_from").filter(|v| truthy(v)).map(py_str).unwrap_or_default();
            for rec in &records {
                if rec.error.is_some() || rec.s("state").as_deref() != Some("candidate") || rec.class() != "ladder" {
                    continue;
                }
                if !since.is_empty() && rec.s("opened").unwrap_or_default() < since {
                    continue;
                }
                let comp_def = match get(&pins, "components").and_then(|c| c.get(rec.s("component").unwrap_or_default())) {
                    Some(c) => c.clone(),
                    None => continue,
                };
                let (stt, val) = effective(&comp_def, &upper);
                if stt != "value" || val.map(|v| py_str(&v)) != rec.s("candidate_ref") {
                    continue;
                }
                for f in runtime_health_failures(rec, &h, &g) {
                    failures.push(format!("C10: {} runs on {upper} but its {}->{to} gate fails: {f}", t.rel(&rec.path), py_opt(get(&h, "from"))));
                }
            }
        }
    }
    // C7 — graduation sets
    let sets = t.load_sets();
    let mut seen_open: BTreeMap<String, String> = BTreeMap::new();
    for doc in &sets {
        let rel = t.rel(&doc.path);
        if let Some(e) = &doc.error {
            failures.push(format!("C7: {rel} does not parse: {e}"));
            continue;
        }
        if !doc.get("name").map(truthy).unwrap_or(false) {
            failures.push(format!("C7: {rel} has no name:"));
        }
        if !SET_STATES.contains(&doc.s("state").unwrap_or_default().as_str()) {
            failures.push(format!("C7: {rel} state {} is not one of [{}]", py_repr(doc.get("state")), { let mut s = SET_STATES.to_vec(); s.sort(); s.iter().map(|x| format!("'{x}'")).collect::<Vec<_>>().join(", ") }));
        }
        let members = set_members(&doc.doc);
        if members.is_empty() {
            failures.push(format!("C7: {rel} names no members — an empty set promotes nothing and hides its own emptiness"));
        }
        for m in &members {
            let comp = s_of(m.get("component")).unwrap_or_default();
            let reference = s_of(m.get("ref"));
            if !comp_names.contains(&comp) {
                failures.push(format!("C7: {rel} names unknown component {}", py_repr(m.get("component"))));
                continue;
            }
            if !reference.as_deref().map(is_digest_anywhere).unwrap_or(false) {
                failures.push(format!("C7: {rel} member {comp} ref is not digest-pinned — build once, promote the digest"));
            }
            if latest_record(&records, &comp, None, reference.as_deref(), None).is_none() {
                failures.push(format!("C7: {rel} member {comp} has no record for its exact ref — the set moves refs, the record carries the evidence, and a member with neither is unreviewable"));
            }
            if doc.s("state").as_deref() == Some("open") {
                if let Some(prev) = seen_open.get(&comp) {
                    failures.push(format!("C7: {comp} is in TWO open sets ({prev} and {rel}) — which one moves it is undecidable"));
                }
                seen_open.insert(comp.clone(), rel.clone());
            }
        }
    }
    // C8 — an abandoned record must say WHY
    for rec in &records {
        if rec.error.is_some() || rec.s("state").as_deref() != Some("abandoned") {
            continue;
        }
        let opened = rec.s("opened").unwrap_or_default();
        if opened.as_str() < ABANDON_REASON_CUTOFF {
            continue;
        }
        let rp = t.rel(&rec.path);
        if rec.s("abandoned_reason").unwrap_or_default().trim().is_empty() {
            failures.push(format!("C8: {rp} is abandoned but has no `abandoned_reason:` — a YAML comment does not count, because nothing that reads records as data can see it. Set it with `promote abandon`."));
        }
        let sb = rec.s("superseded_by").unwrap_or_default().trim().to_string();
        if !sb.is_empty() && !t.records_dir().join(rec.s("component").unwrap_or_default()).join(&sb).exists() && !sb.contains("@sha256:") {
            failures.push(format!("C8: {rp} names superseded_by {}, which is neither a record file beside it nor a digest-pinned ref", py_repr(Some(&json!(sb)))));
        }
    }
    let n_records = records.iter().filter(|r| r.error.is_none()).count();
    let n_sets = sets.iter().filter(|d| d.error.is_none()).count();
    Ok((failures, st.rows.len(), n_records, live.len(), n_sets))
}

fn is_digest_anywhere(s: &str) -> bool {
    regex::Regex::new(r"@sha256:[0-9a-f]{64}").unwrap().is_match(s)
}

/// Python's `repr()` of a str-or-None value as the script interpolates it with `!r`.
fn py_repr(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(s)) => {
            if s.contains('\'') && !s.contains('"') {
                format!("\"{s}\"")
            } else {
                format!("'{}'", s.replace('\'', "\\'"))
            }
        }
        Some(other) => py_str(other),
    }
}

fn cmd_check(t: &Tree, o: &mut Out) -> Result<i32> {
    let (failures, n_rows, n_records, n_live, n_sets) = match check_failures(t) {
        Ok(x) => x,
        Err(e) => {
            o.err(e.to_string());
            return Ok(1);
        }
    };
    if !failures.is_empty() {
        o.err(format!("graduation check: {} failure(s)", failures.len()));
        for f in failures {
            o.err(format!("  {f}"));
        }
        return Ok(1);
    }
    o.out(format!("graduation check: OK ({n_rows} components, {n_records} records, {n_live} live rungs, {n_sets} graduation set(s))"));
    Ok(0)
}

// --- readiness (release-graduation.md §8) ----------------------------------------------------------

fn live_hop(ladder: &Value) -> Option<Value> {
    get(ladder, "hops").and_then(|h| h.as_array()).and_then(|hs| {
        hs.iter().find(|h| get(h, "from").and_then(|f| f.as_str()) == Some("preprod") && get(h, "to").and_then(|f| f.as_str()) == Some("prod") && get(h, "status").and_then(|f| f.as_str()) == Some("live")).cloned()
    })
}

fn git_out(repo: &Path, args: &[&str]) -> (bool, String) {
    match Command::new("git").arg("-C").arg(repo).args(args).output() {
        Err(e) => (false, e.to_string()),
        Ok(o) if !o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            (false, if err.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { err })
        }
        Ok(o) => (true, String::from_utf8_lossy(&o.stdout).trim().to_string()),
    }
}

fn image_workflow(t: &Tree, comp_def: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let set = |out: &mut Map<String, Value>, wf: Value, paths: Value, why: Value| {
        out.insert("workflow".into(), wf);
        out.insert("paths".into(), paths);
        out.insert("why".into(), why);
    };
    match get(comp_def, "image_workflow") {
        None => set(&mut out, Value::Null, Value::Null, json!("fleet-pins.yaml declares no `image_workflow:` for this component, so the tree names no build-input path set")),
        Some(Value::Null) => set(&mut out, Value::Null, Value::Null, json!("fleet-pins.yaml declares this component has no image workflow in this repo")),
        Some(v) => {
            let rel = py_str(v);
            let (paths, why) = workflow_push_paths(t, &rel);
            set(&mut out, json!(rel), paths.map(|p| json!(p)).unwrap_or(Value::Null), why.map(|w| json!(w)).unwrap_or(Value::Null));
        }
    }
    out
}

fn workflow_push_paths(t: &Tree, rel: &str) -> (Option<Vec<String>>, Option<String>) {
    let path = t.repo.join(rel);
    if !path.is_file() {
        return (None, Some(format!("declared image workflow {rel} is not in this checkout")));
    }
    let wf = match read_yaml(&path) {
        Ok(w) => w,
        Err(e) => return (None, Some(format!("{rel} did not parse as YAML: {}", e.to_string().split(": ").skip(1).collect::<Vec<_>>().join(": ")))),
    };
    let on = get(&wf, "on").or_else(|| get(&wf, "true"));
    let push = on.and_then(|o| o.as_object()).and_then(|o| o.get("push"));
    let push = match push.and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return (None, Some(format!("{rel} declares no `on.push` trigger to read a filter from"))),
    };
    let paths = match push.get("paths") {
        None | Some(Value::Null) => {
            if push.get("paths-ignore").map(truthy).unwrap_or(false) {
                return (None, Some(format!("{rel} filters with `paths-ignore` — an exclusion this tool does not evaluate")));
            }
            return (None, Some(format!("{rel} declares no `on.push.paths` filter, so every push to its branches builds and no subset of the tree is this component's build input")));
        }
        Some(p) => p.as_array().cloned().unwrap_or_default().iter().map(py_str).collect::<Vec<String>>(),
    };
    let negated: Vec<&String> = paths.iter().filter(|p| p.starts_with('!')).collect();
    if !negated.is_empty() {
        return (None, Some(format!("{rel} `paths:` carries negated pattern(s) {} — this tool evaluates no exclusions", negated.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))));
    }
    (Some(paths), None)
}

/// What a release candidate carries, for its pin PR body and its record: the
/// commits since the component's previous tag that touch the paths its image
/// workflow builds from (`on.push.paths`, `/**` dropped). Returns the one-line
/// summary and one `- <subject>` line per commit. rc.sh's `rc_changes` is the
/// same computation; keep them in step.
pub fn candidate_changes(repo: &Path, image_workflow: &str, image: &str, tag: &str, fallback_head: &str) -> (String, String) {
    let t = Tree { repo: repo.to_path_buf() };
    let paths: Vec<String> = workflow_push_paths(&t, image_workflow)
        .0
        .unwrap_or_default()
        .iter()
        .map(|p| p.trim_end_matches('*').trim_end_matches('/').to_string())
        .collect();
    let tag_commit = format!("refs/tags/{tag}^{{commit}}");
    let head = match git_out(repo, &["rev-parse", "-q", "--verify", &tag_commit]) {
        (true, h) if !h.is_empty() => h,
        _ => fallback_head.to_string(),
    };
    let pattern = format!("{image}/v*");
    let (_, tags) = git_out(repo, &["tag", "-l", &pattern, "--sort=-v:refname", "--merged", &head]);
    let Some(prev) = tags.lines().map(str::trim).find(|l| !l.is_empty() && *l != tag).map(String::from) else {
        return (format!("the first candidate for {image}"), format!("- the first release candidate tagged for {image}"));
    };
    let range = format!("{prev}..{head}");
    let mut args = vec!["log", "--no-merges", "--format=- %s", range.as_str(), "--"];
    args.extend(paths.iter().map(|p| p.as_str()));
    let (_, log) = git_out(repo, &args);
    let changes = if log.trim().is_empty() { format!("- no change to {image}'s source since {prev}") } else { log.trim().to_string() };
    let prs: Vec<String> = changes
        .lines()
        .filter_map(|l| l.rsplit_once("(#").and_then(|(_, n)| n.strip_suffix(')')).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|n| format!("#{n}")))
        .collect();
    let line = if prs.is_empty() { format!("0 change(s) since {prev}") } else { format!("{} change(s) since {prev}: {}", prs.len(), prs.join(", ")) };
    (line, changes)
}

fn component_commits(t: &Tree, commit: &str, head: &str, paths: &[String]) -> Map<String, Value> {
    let spec: Vec<String> = paths.iter().map(|p| format!(":(glob){p}")).collect();
    let mut args: Vec<&str> = vec!["rev-list", "--count"];
    let range = format!("{commit}..{head}");
    args.push(&range);
    args.push("--");
    args.extend(spec.iter().map(|s| s.as_str()));
    let (ok, count) = git_out(&t.repo, &args);
    let mut out = Map::new();
    if !ok || count.parse::<i64>().is_err() {
        out.insert("known".into(), json!(false));
        out.insert("why".into(), json!(format!("git could not count commits over the declared paths: {count}")));
        return out;
    }
    let n: i64 = count.parse().unwrap_or(0);
    let mut newest = Value::Null;
    if n > 0 {
        let mut a: Vec<&str> = vec!["log", "-1", "--format=%h%x1f%cI%x1f%s", &range, "--"];
        a.extend(spec.iter().map(|s| s.as_str()));
        let (ok2, line) = git_out(&t.repo, &a);
        let parts: Vec<&str> = if ok2 { line.split('\u{1f}').collect() } else { Vec::new() };
        if parts.len() == 3 {
            newest = json!({"sha": parts[0], "date": parts[1], "subject": parts[2]});
        }
    }
    out.insert("known".into(), json!(true));
    out.insert("count".into(), json!(n));
    out.insert("newest".into(), newest);
    out
}

fn component_commit_facts(t: &Tree, commit: Option<&str>, head: Option<&str>, wf: &Map<String, Value>) -> Value {
    let mut base = Map::new();
    base.insert("workflow".into(), wf.get("workflow").cloned().unwrap_or(Value::Null));
    let paths: Option<Vec<String>> = wf.get("paths").and_then(|p| p.as_array()).map(|a| a.iter().map(py_str).collect());
    base.insert("paths".into(), json!(paths.clone().unwrap_or_default()));
    let unknown = |mut b: Map<String, Value>, why: String| {
        b.insert("known".into(), json!(false));
        b.insert("why".into(), json!(why));
        Value::Object(b)
    };
    let paths = match paths {
        None => return unknown(base, wf.get("why").map(py_str).unwrap_or_default()),
        Some(p) => p,
    };
    let commit = match commit {
        None | Some("") => return unknown(base, "no build commit resolvable in this checkout to count from (see the build note for which way the ref failed)".into()),
        Some(c) => c,
    };
    let head = match head {
        None | Some("") => return unknown(base, "the generating checkout resolved no HEAD to count to".into()),
        Some(h) => h,
    };
    for (k, v) in component_commits(t, commit, head, &paths) {
        base.insert(k, v);
    }
    Value::Object(base)
}

fn build_facts(t: &Tree, reference: Option<&str>, head: Option<&str>, wf: &Map<String, Value>) -> Value {
    let finish = |mut facts: Map<String, Value>, commit: Option<&str>| {
        facts.insert("component_commits".into(), component_commit_facts(t, commit, head, wf));
        Value::Object(facts)
    };
    let reference = match reference {
        None | Some("") => {
            let mut f = Map::new();
            f.insert("tag".into(), Value::Null);
            f.insert("known".into(), json!(false));
            f.insert("why".into(), json!("no ref pinned"));
            return finish(f, None);
        }
        Some(r) => r,
    };
    let re = regex::Regex::new(r":sha-([0-9a-f]{7,40})@").unwrap();
    let m = match re.captures(reference) {
        None => {
            let mut f = Map::new();
            f.insert("tag".into(), Value::Null);
            f.insert("known".into(), json!(false));
            f.insert("why".into(), json!("ref carries no sha-<short> tag (digest-only pin)"));
            return finish(f, None);
        }
        Some(m) => m,
    };
    let tag = format!("sha-{}", &m[1]);
    let (ok, commit) = git_out(&t.repo, &["rev-parse", "--verify", &format!("{}^{{commit}}", &m[1])]);
    if !ok {
        let mut f = Map::new();
        f.insert("tag".into(), json!(tag));
        f.insert("known".into(), json!(false));
        f.insert("why".into(), json!("the build commit is not in this checkout"));
        return finish(f, None);
    }
    let (ok_when, when) = git_out(&t.repo, &["show", "-s", "--format=%cI", &commit]);
    let (ok2, ahead) = match head {
        Some(h) if !h.is_empty() => git_out(&t.repo, &["rev-list", "--count", &format!("{commit}..{h}")]),
        _ => (false, String::new()),
    };
    let mut f = Map::new();
    f.insert("tag".into(), json!(tag));
    f.insert("known".into(), json!(true));
    f.insert("commit".into(), json!(commit));
    f.insert("commit_date".into(), if ok_when { json!(when) } else { Value::Null });
    f.insert("commits_on_head_since".into(), if ok2 { ahead.parse::<i64>().map(|n| json!(n)).unwrap_or(Value::Null) } else { Value::Null });
    finish(f, Some(&commit))
}

/// `yaml.safe_load` turns a PLAIN timestamp into a datetime, and `str()` of
/// that is `YYYY-MM-DD HH:MM:SS+00:00`; a quoted one stays as written.
fn py_timestamp_str(raw: &str, quoted: bool) -> String {
    if quoted {
        return raw.to_string();
    }
    let re = regex::Regex::new(r"^(\d{4}-\d{2}-\d{2})[Tt ](\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:?\d{2})?$").unwrap();
    match re.captures(raw) {
        Some(c) => {
            let frac = c.get(3).map(|m| m.as_str().to_string()).unwrap_or_default();
            let frac = if frac.is_empty() || frac.trim_end_matches('0') == "." { String::new() } else { format!("{:0<7}", frac).chars().take(7).collect() };
            let tz = match c.get(4).map(|m| m.as_str()) {
                None => String::new(),
                Some("Z") => "+00:00".into(),
                Some(t) if t.len() == 5 => format!("{}:{}", &t[..3], &t[3..]),
                Some(t) => t.to_string(),
            };
            format!("{} {}{frac}{tz}", &c[1], &c[2])
        }
        None => raw.to_string(),
    }
}

/// (status, rows, path, whether each row's merged_at was a quoted scalar)
type LedgerRows = (&'static str, Vec<Map<String, Value>>, Option<String>, Vec<bool>);

fn ledger_rows(t: &Tree, name: &str, comp_def: &Value) -> LedgerRows {
    if matches!(get(comp_def, "ledger_component"), Some(Value::Null)) {
        return ("no-ledger-coverage", Vec::new(), None, Vec::new());
    }
    let ledger = get(comp_def, "ledger_component").filter(|v| truthy(v)).map(py_str).unwrap_or_else(|| name.to_string());
    let path = t.repo.join("ops/ansible/roles").join(&ledger).join("deploy-history.yml");
    let rel = t.rel(&path);
    if !path.is_file() {
        return ("ledger-file-missing", Vec::new(), Some(rel), Vec::new());
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let doc = match read_yaml(&path) {
        Ok(d) => d,
        Err(_) => return ("ledger-unreadable", Vec::new(), Some(rel), Vec::new()),
    };
    let rows: Vec<Map<String, Value>> = get(&doc, "history").and_then(|h| h.as_array()).map(|a| a.iter().filter_map(|r| r.as_object().cloned()).collect()).unwrap_or_default();
    if rows.is_empty() {
        return ("no-history-rows", Vec::new(), Some(rel), Vec::new());
    }
    let re = regex::Regex::new(r#"^\s*merged_at:\s*(["']?)"#).unwrap();
    let quoted: Vec<bool> = text.lines().filter(|l| !l.trim_start().starts_with('#')).filter_map(|l| re.captures(l).map(|c| !c[1].is_empty())).collect();
    ("rows", rows, Some(rel), quoted)
}

fn digest_of(reference: Option<&Value>) -> Option<String> {
    let s = reference.and_then(|r| r.as_str())?;
    let re = regex::Regex::new(r"@sha256:[0-9a-f]{64}").unwrap();
    re.find(s).map(|m| s[m.start() + 1..m.end()].to_string())
}

fn ledger_facts(t: &Tree, name: &str, comp_def: &Value, pinned: &Vec<(String, Option<String>)>) -> Value {
    let (status, rows, path, quoted) = ledger_rows(t, name, comp_def);
    let mut out = Map::new();
    out.insert("status".into(), json!(status));
    out.insert("path".into(), path.map(|p| json!(p)).unwrap_or(Value::Null));
    out.insert("newest".into(), Value::Null);
    let mut pd = Map::new();
    for (k, _) in pinned {
        pd.insert(k.clone(), Value::Null);
    }
    out.insert("pin_deployed".into(), Value::Object(pd));
    if status != "rows" {
        return Value::Object(out);
    }
    // merged_at as the script's yaml loader shows it: plain stamps become datetimes.
    let mut qi = quoted.iter();
    let stamped: Vec<(Map<String, Value>, Option<String>)> = rows
        .iter()
        .map(|r| {
            let m = r.get("merged_at");
            let q = if m.is_some() { *qi.next().unwrap_or(&false) } else { false };
            let s = m.filter(|v| !v.is_null()).map(|v| py_timestamp_str(&py_str(v), q));
            (r.clone(), s)
        })
        .collect();
    let dated: Vec<&(Map<String, Value>, Option<String>)> = stamped.iter().filter(|(_, s)| s.is_some()).collect();
    let newest = if !dated.is_empty() { dated.iter().max_by(|a, b| a.1.cmp(&b.1)).unwrap() } else { &stamped[0] };
    let mut n = Map::new();
    n.insert("sha".into(), newest.0.get("sha").cloned().unwrap_or(Value::Null));
    n.insert("image".into(), newest.0.get("image").cloned().unwrap_or(Value::Null));
    n.insert("digest".into(), digest_of(newest.0.get("image")).map(|d| json!(d)).unwrap_or(Value::Null));
    n.insert("pr".into(), newest.0.get("pr").cloned().unwrap_or(Value::Null));
    n.insert("merged_at".into(), newest.1.clone().map(|s| json!(s)).unwrap_or(Value::Null));
    n.insert("ordered_by".into(), json!(if !dated.is_empty() { "merged_at" } else { "file order (no merged_at on any row)" }));
    let newest_digest = digest_of(newest.0.get("image"));
    out.insert("newest".into(), Value::Object(n));
    let deployed: Vec<String> = rows.iter().filter_map(|r| digest_of(r.get("image"))).collect();
    let mut pd = Map::new();
    for (k, v) in pinned {
        let d = v.as_deref().and_then(|x| digest_of(Some(&json!(x))));
        pd.insert(k.clone(), d.map(|d| json!(deployed.contains(&d))).unwrap_or(Value::Null));
    }
    out.insert("pin_deployed".into(), Value::Object(pd));
    let pinned_digests: Vec<String> = pinned.iter().filter_map(|(_, v)| v.as_deref().and_then(|x| digest_of(Some(&json!(x))))).collect();
    out.insert("newest_is_pinned".into(), newest_digest.map(|d| json!(pinned_digests.contains(&d))).unwrap_or(Value::Null));
    Value::Object(out)
}

fn soak_waiver(rec: &Record) -> Value {
    let w = match rec.get("soak_waiver").and_then(|w| w.as_object()) {
        Some(w) => w,
        None => return Value::Null,
    };
    let by = w.get("by").filter(|b| truthy(b)).map(|b| py_str(b).trim().to_string());
    json!({
        "by": by,
        "attribution": if by.is_some() { "named" } else { "unknown" },
        "reason": w.get("reason").filter(|r| truthy(r)).map(|r| py_str(r).trim().to_string()),
    })
}

fn soak_facts(rec: &Record, hop: Option<&Value>, today: NaiveDate) -> Value {
    let gate = hop.map(gates).unwrap_or_default().into_iter().find(|g| gate_id(g) == "soak");
    let waiver = soak_waiver(rec);
    let gate = match gate {
        None => return json!({"state": "no-soak-gate", "required_days": null}),
        Some(g) => g,
    };
    let required = get(&gate, "min_days").and_then(|m| m.as_i64()).unwrap_or(1);
    let opened = match date(&py_opt(rec.get("opened"))) {
        Some(d) => d,
        None => {
            let why = format!("record `opened` is not a date: {}", py_repr(rec.get("opened")));
            if !waiver.is_null() {
                return json!({"state": "waived", "required_days": required, "why": why, "waiver": waiver});
            }
            return json!({"state": "unknown", "required_days": required, "why": why});
        }
    };
    let elapsed = (today - opened).num_days();
    let met = elapsed >= required;
    let mut facts = Map::new();
    facts.insert("state".into(), json!(if met { "met" } else if !waiver.is_null() { "waived" } else { "soaking" }));
    facts.insert("required_days".into(), json!(required));
    facts.insert("opened".into(), json!(opened.to_string()));
    facts.insert("elapsed_days".into(), json!(elapsed));
    facts.insert("remaining_days".into(), json!((required - elapsed).max(0)));
    facts.insert("clock".into(), json!("record `opened` date"));
    if !waiver.is_null() {
        facts.insert("waiver".into(), waiver);
    }
    Value::Object(facts)
}

fn candidate_readiness(rec: &Record, hop: Option<&Value>, today: NaiveDate) -> Result<Value> {
    let empty = json!({});
    let mut blockers = gate_failures(rec, hop.unwrap_or(&empty), today)?;
    let ack_required = hop.map(needs_ack).unwrap_or(false);
    if ack_required && !rec.get("operator_ack").map(truthy).unwrap_or(false) {
        blockers.push("operator-ack gate: the record names no operator_ack".into());
    }
    let waiver = soak_waiver(rec);
    let state = if blockers.is_empty() {
        "ready"
    } else if blockers.iter().all(|b| b.starts_with("soak") || b.starts_with("runtime-health awaiting")) {
        if !waiver.is_null() { "soak-waived" } else { "soaking" }
    } else {
        "blocked"
    };
    Ok(json!({
        "state": state,
        "blockers": blockers,
        "ack_required": ack_required,
        "operator_ack": rec.get("operator_ack").cloned().unwrap_or(Value::Null),
        "soak_waiver": waiver,
    }))
}

#[allow(clippy::too_many_arguments)]
fn component_readiness(t: &Tree, row: &Map<String, Value>, comp_def: &Value, records: &[Record], live: &[Value], hop: Option<&Value>, today: NaiveDate, head: Option<&str>, goatnets: &[String]) -> Result<Value> {
    let name = s_of(row.get("component")).unwrap_or_default();
    let devs = get(comp_def, "deviations").and_then(|d| d.as_object()).cloned().unwrap_or_default();
    let wf = image_workflow(t, comp_def);
    let mut rungs = Vec::new();
    let mut pinned: Vec<(String, Option<String>)> = vec![("fleet".into(), get(comp_def, "fleet").filter(|f| !f.is_null()).map(py_str))];
    for r in live {
        let cell = row.get("rungs").and_then(|x| x.get(rung_name(r))).cloned().unwrap_or(json!({}));
        let dev = devs.get(&rung_goatnet(r)).filter(|d| truthy(d)).cloned().unwrap_or(json!({}));
        let value = get(&cell, "value").filter(|v| !v.is_null()).map(py_str);
        pinned.push((rung_name(r), value.clone()));
        rungs.push(json!({
            "rung": rung_name(r),
            "goatnet": rung_goatnet(r),
            "state": get(&cell, "state").cloned().unwrap_or(json!("unresolved")),
            "value": value,
            "deviates_from_fleet": truthy(&dev),
            "deviation_reason": get(&dev, "reason").cloned().unwrap_or(Value::Null),
            "build": build_facts(t, value.as_deref(), head, &wf),
        }));
    }
    let member_devs: Vec<Value> = devs
        .iter()
        .filter(|(k, _)| !goatnets.contains(k))
        .map(|(k, v)| {
            let v = if v.is_object() { v.clone() } else { json!({}) };
            json!({"member": k, "reason": get(&v, "reason").cloned().unwrap_or(Value::Null), "absent": get(&v, "absent").map(truthy).unwrap_or(false), "value": get(&v, "value").cloned().unwrap_or(Value::Null)})
        })
        .collect();
    let rec = latest_record(records, &name, Some("candidate"), None, None);
    let candidate = match rec {
        None => Value::Null,
        Some(rec) => json!({
            "path": t.rel(&rec.path),
            "class": rec.class(),
            "candidate_ref": rec.get("candidate_ref").cloned().unwrap_or(Value::Null),
            "previous_fleet_ref": rec.get("previous_fleet_ref").cloned().unwrap_or(Value::Null),
            "opened": rec.get("opened").cloned().unwrap_or(Value::Null),
            "reason": rec.get("reason").cloned().unwrap_or(Value::Null),
            "evidence": {"validation": rec.evidence("validation"), "preprod_convergence": rec.evidence("preprod_convergence")},
            "note": rec.get("note").cloned().unwrap_or(Value::Null),
            "soak": soak_facts(rec, hop, today),
            "readiness": candidate_readiness(rec, hop, today)?,
        }),
    };
    let last = latest_record(records, &name, Some("graduated"), None, None);
    let readiness_state = match (&candidate, s_of(row.get("class")).as_deref()) {
        (Value::Object(c), _) => c["readiness"]["state"].clone(),
        (_, Some("exempt")) => json!("not-applicable"),
        _ => json!("no-candidate"),
    };
    Ok(json!({
        "component": name,
        "class": row.get("class").cloned().unwrap_or(Value::Null),
        "grade": row.get("grade").cloned().unwrap_or(Value::Null),
        "pairs": row.get("pairs").cloned().unwrap_or(json!({})),
        "var": get(comp_def, "var").cloned().unwrap_or(Value::Null),
        "artifact_type": get(comp_def, "artifact_type").cloned().unwrap_or(json!("image")),
        "fleet": get(comp_def, "fleet").cloned().unwrap_or(Value::Null),
        "image_workflow": Value::Object(wf),
        "rungs": rungs,
        "member_deviations": member_devs,
        "candidate": candidate,
        "readiness_state": readiness_state,
        "last_graduated": last.map(|l| json!({"ref": l.get("candidate_ref").cloned().unwrap_or(Value::Null), "on": l.get("graduated").cloned().unwrap_or(Value::Null), "ack": l.get("operator_ack").cloned().unwrap_or(Value::Null), "soak_waiver": soak_waiver(l), "path": t.rel(&l.path)})).unwrap_or(Value::Null),
        "ledger": ledger_facts(t, &name, comp_def, &pinned),
    }))
}

pub fn build_readiness(t: &Tree, today: NaiveDate) -> Result<Value> {
    let st = build_status(t)?;
    let pins = t.load_pins()?;
    let hop = live_hop(&st.ladder);
    let goatnets: Vec<String> = get(&pins, "goatnets").and_then(|g| g.as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let (ok, head) = git_out(&t.repo, &["rev-parse", "HEAD"]);
    let head = if ok { Some(head) } else { None };
    let comps = get(&pins, "components").and_then(|c| c.as_object()).cloned().unwrap_or_default();
    let mut rows = st.rows.clone();
    rows.sort_by_key(|r| s_of(r.get("component")).unwrap_or_default());
    let mut components = Vec::new();
    for row in &rows {
        let name = s_of(row.get("component")).unwrap_or_default();
        let def = comps.get(&name).cloned().unwrap_or(json!({}));
        components.push(component_readiness(t, row, &def, &st.records, &st.live, hop.as_ref(), today, head.as_deref(), &goatnets)?);
    }
    let ms = chrono::Utc::now().timestamp_millis();
    Ok(json!({
        "schema": READINESS_SCHEMA,
        "generated_unix_ms": ms,
        "generated_date": today.to_string(),
        "source": {
            "head_commit": head,
            "git": if head.is_some() { "available" } else { "unavailable — build ages are unknown" },
            "note": "generated from a checkout at deploy time; it does not refresh itself, so its age IS the age of its facts",
        },
        "ladder": {
            "rungs": st.live.iter().map(|r| json!({"name": rung_name(r), "goatnet": rung_goatnet(r)})).collect::<Vec<_>>(),
            "hop": hop.as_ref().map(|h| json!({"from": get(h, "from").cloned().unwrap_or(Value::Null), "to": get(h, "to").cloned().unwrap_or(Value::Null), "gates": gates(h).iter().map(|g| if g.is_object() { g.clone() } else { json!({"id": g}) }).collect::<Vec<_>>()})).unwrap_or(Value::Null),
        },
        "unknowable": [
            "Whether an image was actually built and pushed: this tool contacts no CI and no registry. Per component it reports whether commits since the pinned build touched the paths that component's image workflow builds on — a strong \"a newer image exists that this rung does not run\", not a confirmed one.",
            "Components whose build inputs the tree does not declare: no `image_workflow:` in fleet-pins.yaml, a workflow with no `on.push.paths` filter, or a filter this tool will not evaluate (exclusions, negated patterns). Those rows read unknown with the reason — never 0 commits.",
            "CI run state: no committed artifact carries it and this tool contacts nothing; the newest deploy-ledger row's PR is the last build with in-tree evidence, which is not the same fact.",
            "Whether a recorded soak waiver was justified: a `soak_waiver:` block is an operator's decision written down, verified by nothing. `graduate` still refuses on the unmet gate — the waiver records the decision, it does not override it.",
        ],
        "components": components,
    }))
}

fn short(text: &str) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= 110 { t } else { format!("{}…", t.chars().take(109).collect::<String>()) }
}

fn narrate(name: &str, r: &Value, rd: &Value, today: NaiveDate) -> Map<String, Value> {
    let cls = get(r, "class").filter(|c| truthy(c)).map(py_str).unwrap_or_else(|| "ladder".into());
    let cref = get(r, "candidate_ref").filter(|c| !c.is_null()).map(py_str).unwrap_or_default();
    let reference = { let a = cref.split('@').next().unwrap_or(""); let b = a.rsplit(':').next().unwrap_or(""); if b.is_empty() { "?".to_string() } else { b.to_string() } };
    let reason = short(&get(r, "reason").filter(|c| !c.is_null()).map(py_str).unwrap_or_default());
    let ev = get(r, "evidence").and_then(|e| e.as_object()).cloned().unwrap_or_default();
    let missing: Vec<String> = ev.iter().filter(|(_, v)| !truthy(v)).map(|(k, _)| k.clone()).collect();
    let opened_s = py_opt(get(r, "opened"));
    let age = date(&opened_s).map(|d| (today - d).num_days());
    let owner = get(rd, "operator_ack").filter(|o| truthy(o)).map(py_str).unwrap_or_else(|| "nobody named".into());
    let state = get(rd, "state").and_then(|s| s.as_str()).unwrap_or("");
    let opened_disp = py_opt(get(r, "opened"));
    let (what, left, acted);
    if cls == "hotfix" {
        let validated = ev.get("validation").map(truthy).unwrap_or(false);
        let cutoff = NaiveDate::from_ymd_opt(HOTFIX_BACKFILL_CUTOFF.0, HOTFIX_BACKFILL_CUTOFF.1, HOTFIX_BACKFILL_CUTOFF.2).unwrap();
        let (c9, grandfathered) = match date(&opened_s) {
            Some(o) if o >= cutoff => (Some((o + chrono::Duration::days(HOTFIX_BACKFILL_MAX_DAYS + 1)).to_string()), false),
            Some(_) => (None, true),
            None => (None, false),
        };
        what = format!("a fastlane debt: {name} {reference} went to EVERY goatnet on {opened_disp} with no soak ({reason}). Prod already runs it; {}", if validated { "its validation on dev and preprod is on record." } else { "nobody has validated it on record." });
        if state == "ready" {
            left = "the debt stays open and status keeps grading the component owed; nothing changes on any goatnet.".to_string();
            acted = "graduate closes the record. No deploy — the ref is already everywhere. Bookkeeping, with evidence.".to_string();
        } else if state == "soaking" || state == "soak-waived" {
            left = if validated {
                "nothing until the runnable date; the debt is only bookkeeping now.".to_string()
            } else {
                format!("the soak runs, but the record still owes its validation{}", match &c9 { Some(c) => format!(" — CI check C9 blocks every PR from {c}."), None => "; nothing will ever force it (grandfathered), only this review shows it.".to_string() })
            };
            acted = if validated {
                "on the runnable date, graduate closes the record. No deploy.".to_string()
            } else {
                "run the mapped live rows on dev and preprod now (read-only probes, minutes), write PASS into evidence.validation; on the runnable date graduate closes the record. No deploy.".to_string()
            };
        } else {
            left = format!("prod keeps running a ref with no validation on record{}.", match (&c9, validated, grandfathered) { (Some(c), false, _) => format!("; CI check C9 blocks every PR from {c}"), (_, _, true) => "; nothing will ever force it (grandfathered), so only this review shows it".to_string(), _ => String::new() });
            acted = format!("run the mapped live rows on dev and preprod (read-only probes, minutes), write PASS into evidence.{}; then graduate closes the record. No prod change.", if missing.is_empty() { "validation".to_string() } else { missing.join("/") });
        }
    } else {
        what = format!("a release candidate: {name} {reference}, opened {opened_disp}{}, owner {owner}. {reason}", age.map(|a| format!(" ({a} days ago)")).unwrap_or_default());
        if state == "ready" {
            left = "preprod keeps running a ref prod does not; the candidate ages and its evidence goes stale.".to_string();
            acted = "graduate marks the record graduated and moves fleet: to this ref — PROD DEPLOYS IT on the next converge. A real production change: read the record and the live state first.".to_string();
        } else if state == "soaking" || state == "soak-waived" {
            left = "the soak keeps running; nothing to do until the runnable date except fill the evidence.".to_string();
            acted = "fill the evidence it names now; on the runnable date graduate moves fleet: to this ref and PROD DEPLOYS IT.".to_string();
        } else {
            left = "preprod runs a ref prod does not, with no evidence and no owner; it drifts until check C6 abandons it when the rung moves — the candidate is quietly lost.".to_string();
            acted = "decide it: either run the rows on dev and preprod and name an ack, then graduate (a PROD DEPLOY), or abandon it with a reason so the ladder is honest. Either way the drift ends.".to_string();
        }
    }
    let mut out = Map::new();
    out.insert("what".into(), json!(what));
    out.insert("if_left".into(), json!(left));
    out.insert("if_acted".into(), json!(acted));
    out.insert("owner".into(), json!(owner));
    out.insert("age_days".into(), age.map(|a| json!(a)).unwrap_or(Value::Null));
    out
}

pub fn build_review(t: &Tree, today: NaiveDate, ack: &str) -> Result<Value> {
    let doc = build_readiness(t, today)?;
    let (mut act, mut soaking, mut blocked, mut attention): (Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>) = (vec![], vec![], vec![], vec![]);
    let cutoff = NaiveDate::from_ymd_opt(HOTFIX_BACKFILL_CUTOFF.0, HOTFIX_BACKFILL_CUTOFF.1, HOTFIX_BACKFILL_CUTOFF.2).unwrap();
    for c in doc["components"].as_array().cloned().unwrap_or_default() {
        let name = s_of(get(&c, "component")).unwrap_or_default();
        if get(&c, "grade").and_then(|g| g.as_str()) == Some("UNDECLARED-SKEW") {
            let hops = get(&c, "pairs").and_then(|p| p.as_object()).map(|m| m.iter().filter(|(_, g)| g.as_str() == Some("UNDECLARED-SKEW")).map(|(p, _)| p.replace("->", " and ")).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            attention.push(json!({"component": name, "what": format!("{name}: {hops} run different refs and no record says why."), "if_left": "the ladder cannot say which goatnet is right; rollback has no pointer.", "if_acted": "open a record for the intended ref, or fix the pin; nothing deploys.", "command": "goatbuilder promote status"}));
        }
        let cand = get(&c, "candidate").filter(|x| x.is_object()).cloned();
        if let Some(r) = cand {
            let rd = r["readiness"].clone();
            let sk = r["soak"].clone();
            let state = rd["state"].as_str().unwrap_or("").to_string();
            let cref = get(&r, "candidate_ref").filter(|x| !x.is_null()).map(py_str).unwrap_or_default();
            let n = cref.chars().count();
            let mut base = Map::new();
            base.insert("component".into(), json!(name));
            base.insert("record".into(), r["path"].clone());
            base.insert("class".into(), r["class"].clone());
            base.insert("opened".into(), r["opened"].clone());
            base.insert("ref".into(), json!(cref.chars().skip(n.saturating_sub(19)).collect::<String>()));
            base.insert("state".into(), json!(state));
            let mut grad = format!("goatbuilder promote graduate {name} --ack {ack}");
            let is_hotfix = r["class"].as_str() == Some("hotfix");
            if is_hotfix {
                grad.push_str("  # closes the fastlane's hotfix record");
            }
            let missing: Vec<String> = get(&r, "evidence").and_then(|e| e.as_object()).map(|m| m.iter().filter(|(_, v)| !truthy(v)).map(|(k, _)| k.clone()).collect()).unwrap_or_default();
            if state == "ready" {
                let mut item = base.clone();
                item.insert("command".into(), json!(grad));
                item.extend(narrate(&name, &r, &rd, today));
                act.push(Value::Object(item));
            } else if state == "soaking" || state == "soak-waived" {
                let runnable = date(&py_opt(get(&r, "opened"))).map(|d| (d + chrono::Duration::days(get(&sk, "required_days").and_then(|x| x.as_i64()).unwrap_or(1))).to_string());
                let mut item = base.clone();
                item.insert("runnable".into(), runnable.map(|x| json!(x)).unwrap_or(Value::Null));
                item.insert("remaining_days".into(), get(&sk, "remaining_days").cloned().unwrap_or(Value::Null));
                item.insert("fill_first".into(), json!(missing));
                item.insert("command".into(), json!(grad));
                item.extend(narrate(&name, &r, &rd, today));
                soaking.push(Value::Object(item));
            } else {
                let mut item = base.clone();
                item.insert("blockers".into(), rd["blockers"].clone());
                item.insert("command".into(), json!(grad));
                let mut r2 = r.clone();
                if is_hotfix && !get(&r, "evidence").and_then(|e| get(e, "validation")).map(truthy).unwrap_or(false) {
                    if let Some(o) = date(&py_opt(get(&r, "opened"))) {
                        if o >= cutoff {
                            item.insert("check_red_on".into(), json!((o + chrono::Duration::days(HOTFIX_BACKFILL_MAX_DAYS + 1)).to_string()));
                        } else {
                            item.insert("grandfathered".into(), json!(true));
                        }
                    }
                    item.insert("fill_first".into(), json!([format!("run the live rows on dev and preprod (ops/validation/affected-flows.py --component {name}; run-acceptance.py), write PASS into evidence.validation")]));
                }
                if let Some(m) = r2.as_object_mut() {
                    m.insert("check_red_on".into(), item.get("check_red_on").cloned().unwrap_or(Value::Null));
                }
                item.extend(narrate(&name, &r2, &rd, today));
                blocked.push(Value::Object(item));
            }
        }
    }
    let (check_ok, check_lines) = match check_failures(t) {
        Ok((f, _, _, _, _)) => (f.is_empty(), f.into_iter().filter(|l| l.trim().starts_with('C')).collect::<Vec<_>>()),
        Err(e) => (false, vec![e.to_string()]),
    };
    let prod_changes = act.iter().chain(soaking.iter()).filter(|i| get(i, "if_acted").and_then(|x| x.as_str()).map(|s| s.contains("PROD DEPLOYS")).unwrap_or(false)).count();
    Ok(json!({
        "schema": "release-review-v1",
        "date": today.to_string(),
        "head": doc["source"]["head_commit"].clone(),
        "summary": {"act_now": act.len(), "soaking": soaking.len(), "blocked": blocked.len(), "attention": attention.len() + check_lines.len(), "prod_changes_pending": prod_changes},
        "act_now": act, "soaking": soaking, "blocked": blocked, "attention": attention,
        "check": {"ok": check_ok, "failures": check_lines},
    }))
}

fn next_step(it: &Value) -> String {
    let st = get(it, "state").and_then(|s| s.as_str()).unwrap_or("");
    let cls = get(it, "class").and_then(|s| s.as_str()).unwrap_or("");
    let tail = if cls != "hotfix" { "PROD" } else { "no deploy" };
    if st == "ready" {
        return format!("graduate → {tail}");
    }
    if st == "soaking" || st == "soak-waived" {
        let fill = if get(it, "fill_first").map(truthy).unwrap_or(false) { "fill evidence → " } else { "" };
        return format!("{fill}graduate on {} → {tail}", get(it, "runnable").filter(|r| truthy(r)).map(py_str).unwrap_or_else(|| "?".into()));
    }
    let blockers: Vec<String> = get(it, "blockers").and_then(|b| b.as_array()).map(|a| a.iter().map(py_str).collect()).unwrap_or_default();
    let mut needs = Vec::new();
    if blockers.iter().any(|b| b.contains("evidence")) {
        needs.push("evidence");
    }
    if blockers.iter().any(|b| b.contains("operator-ack")) {
        needs.push("ack");
    }
    if blockers.iter().any(|b| b.starts_with("runtime-health")) {
        needs.push("runtime health");
    }
    let what = if needs.is_empty() { "gate".to_string() } else { needs.join(" + ") };
    if cls == "hotfix" {
        return format!("rows → {what} → graduate → no deploy");
    }
    format!("{what} → graduate → PROD")
}

fn due(it: &Value) -> String {
    if let Some(r) = get(it, "runnable").filter(|r| truthy(r)) {
        return py_str(r);
    }
    if let Some(c) = get(it, "check_red_on").filter(|r| truthy(r)) {
        return format!("CI {}", py_str(c).chars().skip(5).collect::<String>());
    }
    if get(it, "grandfathered").map(truthy).unwrap_or(false) {
        return "never".into();
    }
    "—".into()
}

fn age(it: &Value, today: NaiveDate) -> String {
    match date(&py_opt(get(it, "opened"))) {
        Some(d) => format!("{}d", (today - d).num_days()),
        None => "?".into(),
    }
}

fn terminal_width() -> usize {
    let cols = std::env::var("COLUMNS").ok().and_then(|c| c.parse::<usize>().ok()).or_else(|| {
        if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            Command::new("tput").arg("cols").output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<usize>().ok())
        } else {
            None
        }
    });
    cols.unwrap_or(100).max(80)
}

/// `text` wrapped to `width`, under the given indents. The page's own `fill`
/// takes a label; this is the same layout without one.
fn fill_plain(text: &str, width: usize, first: &str, rest: &str) -> Vec<String> {
    // Greedy to `width`, which is this page's own width (the script wraps to
    // the same one, ADR 1201). `crate::tutor::fill` is fixed at 80 and would
    // wrap a line the script leaves whole.
    let mut lines: Vec<String> = Vec::new();
    let mut line = first.to_string();
    let mut len = line.chars().count();
    for word in text.split_whitespace() {
        let w = word.chars().count();
        let sep = usize::from(len > rest.chars().count() || line.trim_end() != first.trim_end());
        if len + sep + w > width && line.trim() != first.trim() {
            lines.push(line.trim_end().to_string());
            line = format!("{rest}{word}");
            len = line.chars().count();
            continue;
        }
        if sep == 1 && !line.ends_with(' ') && !line.is_empty() {
            line.push(' ');
            len += 1;
        }
        line.push_str(word);
        len += w;
    }
    lines.push(line.trim_end().to_string());
    lines
}

fn pad(s: &str, w: usize) -> String {
    let n = s.chars().count();
    format!("{s}{}", " ".repeat(w.saturating_sub(n)))
}

fn cut(s: &str, w: usize) -> String {
    s.chars().take(w).collect()
}

/// `textwrap.fill(text, width - 1, initial_indent, subsequent_indent)`: the
/// text is chunked into words and the whitespace runs between them (a run is
/// kept inside a line and dropped at a break), greedy to the width.
fn fill(label: &str, text: &str, width: usize) -> Vec<String> {
    let first = format!("    {}", pad(label, 9));
    let sub = " ".repeat(first.chars().count());
    let limit = width - 1;
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_ws: Option<bool> = None;
    for c in text.chars() {
        let ws = c.is_whitespace();
        if cur_ws.is_some() && cur_ws != Some(ws) {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push(if ws { ' ' } else { c });
        cur_ws = Some(ws);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    let mut lines = Vec::new();
    let mut line = first.clone();
    let mut len = line.chars().count();
    let mut i = 0;
    while i < chunks.len() {
        let ch = &chunks[i];
        let cl = ch.chars().count();
        if len + cl <= limit {
            line.push_str(ch);
            len += cl;
            i += 1;
            continue;
        }
        if ch.trim().is_empty() {
            i += 1;
            continue;
        }
        if line.trim().is_empty() || line == first || line == sub {
            line.push_str(ch);
            i += 1;
        }
        let trimmed = line.trim_end().to_string();
        lines.push(trimmed);
        line = sub.clone();
        len = line.chars().count();
        while i < chunks.len() && chunks[i].trim().is_empty() {
            i += 1;
        }
    }
    if line != sub || lines.is_empty() {
        lines.push(line.trim_end().to_string());
    }
    lines
}

fn cmd_review(t: &Tree, json: bool, ack: Option<&str>, detail: bool, no_header: bool, component: Option<&str>, o: &mut Out) -> Result<i32> {
    let td = today();
    let doc = build_review(t, td, ack.unwrap_or("<you>"))?;
    if json {
        o.raw = Some(json_dumps(&doc));
        return Ok(0);
    }
    let width = terminal_width();
    let sm = &doc["summary"];
    let head: String = doc["head"].as_str().unwrap_or("").chars().take(9).collect();
    let mut rows: Vec<(&str, Value)> = Vec::new();
    for (st, key) in [("READY", "act_now"), ("SOAKING", "soaking"), ("BLOCKED", "blocked")] {
        for it in doc[key].as_array().cloned().unwrap_or_default() {
            rows.push((st, it));
        }
    }
    if let Some(only) = component {
        rows.retain(|(_, it)| get(it, "component").and_then(|c| c.as_str()) == Some(only));
    }
    if !no_header {
        o.out(format!("REVIEW  {}  main {}", doc["date"].as_str().unwrap_or(""), if head.is_empty() { "?" } else { &head }));
        o.out(String::new());
    }
    let pcp = sm["prod_changes_pending"].as_i64().unwrap_or(0);
    o.out(format!("LADDER  {} ready · {} soaking · {} blocked · {} attention{}", sm["act_now"], sm["soaking"], sm["blocked"], sm["attention"], if pcp > 0 { format!(" · {pcp} would deploy to PROD") } else { String::new() }));
    let attention = doc["attention"].as_array().cloned().unwrap_or_default();
    let failures: Vec<String> = doc["check"]["failures"].as_array().map(|a| a.iter().map(py_str).collect()).unwrap_or_default();
    if rows.is_empty() && attention.is_empty() && failures.is_empty() {
        o.out("  nothing on the ladder needs a person today");
        return Ok(0);
    }
    if !detail {
        let cw = rows.iter().map(|(_, it)| get(it, "component").and_then(|c| c.as_str()).map(|s| s.chars().count()).unwrap_or(0)).max().unwrap_or(9).min(22);
        let fmt = |a: &str, b: &str, c: &str, d: &str, e: &str, f: &str| format!("  {} {} {} {:>4}  {}  {}", pad(a, 8), pad(b, cw), pad(c, 10), d, pad(e, 10), f);
        o.out(fmt("STATE", "COMPONENT", "KIND", "AGE", "DUE", "NEXT"));
        for (st, it) in &rows {
            // The ladder's own words are `candidate` and `fastlane`; this column
            // is for a reader (output standard, rule 4).
            let kind = if get(it, "class").and_then(|c| c.as_str()) == Some("hotfix") { "hotfix" } else { "release" };
            let comp = get(it, "component").and_then(|c| c.as_str()).unwrap_or("");
            let step = next_step(it);
            let line = fmt(st, &cut(comp, cw), kind, &age(it, td), &due(it), &step);
            if line.chars().count() < width {
                o.out(line);
                continue;
            }
            // NEXT is the only place a row says what to do, so it moves to its
            // own line rather than being cut in half ("evidence + ack → gr").
            o.out(fmt(st, &cut(comp, cw), kind, &age(it, td), &due(it), "").trim_end().to_string());
            let under = " ".repeat(11);
            for l in fill_plain(&format!("next: {step}"), width.saturating_sub(1), &under, &format!("{under}      ")) {
                o.out(l);
            }
        }
        for it in &attention {
            let comp = get(it, "component").and_then(|c| c.as_str()).unwrap_or("");
            o.out(cut(&format!("  {} {} {} {:>4}  {}  fix the pin or open a record", pad("SKEW", 8), pad(&cut(comp, cw), cw), pad("tree", 10), "", pad("—", 10)), width - 1));
        }
        for f in &failures {
            o.out(cut(&format!("  {} {} {} {:>4}  {}  {f}", pad("CHECK", 8), pad("graduate.py", cw), pad("tree", 10), "", pad("now", 10)), width - 1));
        }
        for l in fill_plain("PROD = graduating deploys to production · never = grandfathered, only this review shows it", width.saturating_sub(1), "  ", "    ") {
            o.out(l);
        }
        // The command goes on its own line, in both avenues: its name is
        // longer here than in the script, and a width-driven wrap would then
        // break the two pages apart (ADR 1201).
        o.out("  a release nobody owns may instead be abandoned:".to_string());
        o.out("    goatbuilder promote abandon <record> --reason \"<why>\"".to_string());
        o.out("  detail: goatbuilder promote review --detail <component>".to_string());
        return Ok(0);
    }
    for (st, it) in &rows {
        let kind = if get(it, "class").and_then(|c| c.as_str()) == Some("hotfix") { "fastlane debt" } else { "release candidate" };
        let mut extra = get(it, "runnable").filter(|r| truthy(r)).map(|r| format!(" · runnable {}", py_str(r))).unwrap_or_default();
        extra.push_str(&match get(it, "check_red_on").filter(|r| truthy(r)) {
            Some(c) => format!(" · CI red {}", py_str(c)),
            None => if get(it, "grandfathered").map(truthy).unwrap_or(false) { " · grandfathered".to_string() } else { String::new() },
        });
        o.out(format!("\n  {st}  {}  ({kind}, opened {}, {}){extra}", get(it, "component").and_then(|c| c.as_str()).unwrap_or(""), py_opt(get(it, "opened")), age(it, td)));
        for l in fill("what", get(it, "what").and_then(|x| x.as_str()).unwrap_or(""), width) { o.out(l); }
        for l in fill("if left", get(it, "if_left").and_then(|x| x.as_str()).unwrap_or(""), width) { o.out(l); }
        for l in fill("if acted", get(it, "if_acted").and_then(|x| x.as_str()).unwrap_or(""), width) { o.out(l); }
        for b in get(it, "blockers").and_then(|b| b.as_array()).cloned().unwrap_or_default() {
            for l in fill("blocked", &py_str(&b), width) { o.out(l); }
        }
        for l in fill("command", get(it, "command").and_then(|x| x.as_str()).unwrap_or(""), width) { o.out(l); }
    }
    for it in &attention {
        o.out(format!("\n  SKEW  {}", get(it, "component").and_then(|c| c.as_str()).unwrap_or("")));
        for key in ["what", "if_left", "if_acted", "command"] {
            let label = key.replace('_', " ");
            for l in fill(&label, get(it, key).and_then(|x| x.as_str()).unwrap_or(""), width) { o.out(l); }
        }
    }
    for f in &failures {
        o.out(format!("\n  CHECK  {f}"));
    }
    o.out(String::new());
    Ok(0)
}

fn cmd_readiness(t: &Tree, out: Option<&Path>, o: &mut Out) -> Result<i32> {
    let doc = build_readiness(t, today())?;
    let text = json_dumps(&doc);
    match out {
        Some(p) => {
            std::fs::write(p, format!("{text}\n"))?;
            o.out(format!("release readiness written: {} ({} components)", p.display(), doc["components"].as_array().map(|a| a.len()).unwrap_or(0)));
        }
        // A terminal gets the page; a pipe, a redirect and `-o json` get the
        // document, so every program that reads this is untouched (output
        // standard, rule 1).
        None if std::io::IsTerminal::is_terminal(&std::io::stdout()) && !crate::output::machine_output() => {
            for line in readiness_page(&doc) {
                o.out(line);
            }
        }
        None => o.raw = Some(text),
    }
    Ok(0)
}

/// `promote readiness` as a person reads it: how many components are at the
/// same version everywhere, how many are mid-release, how many differ with
/// nothing declaring it, and where the whole document is.
fn readiness_page(doc: &Value) -> Vec<String> {
    let comps = doc["components"].as_array().cloned().unwrap_or_default();
    let grade = |c: &Value| get(c, "grade").and_then(|g| g.as_str().map(String::from)).unwrap_or_default();
    let count = |g: &str| comps.iter().filter(|c| grade(c) == g).count();
    let named = |g: &str| {
        let mut v: Vec<String> = comps.iter().filter(|c| grade(c) == g).filter_map(|c| get(c, "component").and_then(|x| x.as_str()).map(String::from)).collect();
        v.sort();
        v
    };
    let mut out = vec![
        format!("READINESS  {}  {} components", doc["generated_date"].as_str().unwrap_or(""), comps.len()),
        String::new(),
    ];
    // Every grade the ladder can give, so the numbers add up to the components.
    let known = [
        ("aligned", "at the same version on every stage"),
        ("candidate", "mid-release: a stage is ahead, and a record says so"),
        ("UNDECLARED-SKEW", "a stage is ahead and nothing declares it"),
        ("hotfix-backfill-owed", "a hotfix went out and the stages behind it still owe it"),
        ("env-local", "declared per goatnet, so no stage to compare"),
        ("single-rung", "declared on one stage only"),
    ];
    for (grade, says) in known {
        let n = count(grade);
        if n == 0 {
            continue;
        }
        out.push(format!("  {n:>3}  {says}"));
        if grade == "UNDECLARED-SKEW" {
            // The one state somebody has to do something about: name them.
            for l in fill("", &named(grade).join(", "), 78) {
                out.push(format!("  {}", l.trim_start()));
            }
        }
    }
    let accounted: usize = known.iter().map(|(g, _)| count(g)).sum();
    if accounted < comps.len() {
        out.push(format!("  {:>3}  graded in some other way: goatbuilder -o json promote readiness names them", comps.len() - accounted));
    }
    let unknowable = doc["unknowable"].as_array().map(|a| a.len()).unwrap_or(0);
    if unknowable > 0 {
        out.push(format!("  {unknowable:>3}  could not be read"));
    }
    out.push(String::new());
    out.push("  What to do about them: goatbuilder promote review".to_string());
    out.push("  The whole document, as a program reads it: goatbuilder -o json promote readiness".to_string());
    out
}

// --- the verb ------------------------------------------------------------------------------------------

pub fn run(repo: &Path, cmd: &Sub, yes: bool) -> Result<Report> {
    let t = Tree::new(repo);
    let mut o = Out::new();
    let mutation = matches!(cmd, Sub::Open { .. } | Sub::Graduate { .. } | Sub::GraduateSet { .. } | Sub::Hotfix { .. } | Sub::Abandon { .. });
    if mutation && !yes && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(goat_errors::coded("GB-008", "promote writes a record: pass --yes without a terminal (C10)").into());
    }
    let code = match cmd {
        Sub::Status { json } => cmd_status(&t, *json, &mut o),
        Sub::Open { component, reference, reason } => cmd_open(&t, component, reference, reason, &mut o),
        Sub::Graduate { component, ack, override_runtime_health } => cmd_graduate(&t, component, ack.as_deref(), override_runtime_health.as_deref(), false, &mut o),
        Sub::GraduateSet { name, ack } => cmd_graduate_set(&t, name, ack.as_deref(), &mut o),
        Sub::Hotfix { component, reference, reason, ack, previous_ref } => cmd_hotfix(&t, component, reference, reason, ack.as_deref(), previous_ref.as_deref(), &mut o),
        Sub::Review { json, ack, detail, no_header, component } => cmd_review(&t, *json, ack.as_deref(), *detail, *no_header, component.as_deref(), &mut o),
        Sub::Readiness { out } => cmd_readiness(&t, out.as_deref(), &mut o),
        Sub::Abandon { record, reason, superseded_by } => cmd_abandon(&t, record, reason, superseded_by.as_deref(), &mut o),
        Sub::Check => cmd_check(&t, &mut o),
    };
    let code = match code {
        Ok(c) => c,
        Err(e) => {
            // the script's `die`: one line on stderr, exit 1
            o.err(e.to_string());
            1
        }
    };
    if let Some(raw) = o.raw.take() {
        println!("{raw}");
    }
    let mut r = Report::new("promote", o.lines, EXIT_REFUSED);
    r.ok = code == 0;
    r.exit = if code == 0 { EXIT_OK } else { EXIT_REFUSED };
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rh_record(block: Value, extra: Value) -> Record {
        let mut doc = json!({
            "component": "thing", "class": "ladder", "state": "candidate", "opened": "2026-08-01",
            "candidate_ref": format!("ghcr.io/o/thing:sha-abc1234@sha256:{}", "a".repeat(64)),
            "runtime_health": {"preprod": block},
        });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            doc.as_object_mut().unwrap().insert(k, v);
        }
        Record { path: PathBuf::from("r.yaml"), doc: doc.as_object().cloned().unwrap(), error: None }
    }

    fn rh_block(over: Value) -> Value {
        let mut b = json!({"captured": "2026-08-12T10:00:00Z", "deployed": "2026-08-10T09:00:00Z",
                           "digests": [format!("sha256:{}", "a".repeat(64))], "restarts": 0, "oom_kills": 0,
                           "peak_memory_fraction": 0.42});
        for (k, v) in over.as_object().cloned().unwrap_or_default() {
            b.as_object_mut().unwrap().insert(k, v);
        }
        b
    }

    fn rh_hop() -> Value {
        json!({"from": "preprod", "to": "prod", "status": "live", "gates": [
            {"id": "soak", "min_days": 2}, {"id": "runtime-health", "max_memory_fraction": 0.8}]})
    }

    fn rh(rec: &Record) -> Vec<String> {
        gate_failures(rec, &rh_hop(), NaiveDate::from_ymd_opt(2026, 8, 12).unwrap()).unwrap().into_iter().filter(|f| f.starts_with("runtime-health")).collect()
    }

    #[test]
    fn runtime_health_gate_matches_the_script() {
        assert!(rh(&rh_record(rh_block(json!({})), json!({}))).is_empty());
        assert_eq!(rh(&rh_record(rh_block(json!({"restarts": 8})), json!({}))), vec!["runtime-health: preprod restarted 8 time(s) since deploy"]);
        assert_eq!(rh(&rh_record(rh_block(json!({"oom_kills": 25})), json!({}))), vec!["runtime-health: preprod OOM-killed 25 time(s) since deploy"]);
        assert_eq!(rh(&rh_record(rh_block(json!({"peak_memory_fraction": 0.93})), json!({}))), vec!["runtime-health: preprod peak memory reached 0.93 of its limit, ceiling 0.8"]);
        assert!(rh(&rh_record(rh_block(json!({"peak_memory_fraction": 0.8})), json!({}))).is_empty());
        assert_eq!(rh(&rh_record(rh_block(json!({"oom_kills": null})), json!({}))), vec!["runtime-health: preprod oom_kills unknown — the capture could not read it"]);
        assert_eq!(rh(&rh_record(rh_block(json!({"captured": "2026-08-11T08:00:00Z"})), json!({}))), vec!["runtime-health awaiting: runtime_health.preprod was captured 23 h after deploy, needs >= 48 h (the soak)"]);
        assert_eq!(rh(&rh_record(rh_block(json!({"digests": [format!("sha256:{}", "b".repeat(64))]})), json!({}))), vec!["runtime-health: runtime_health.preprod was not captured on the candidate digest aaaaaaaaaaaa"]);
        assert_eq!(rh(&rh_record(Value::Null, json!({}))), vec!["runtime-health awaiting: no runtime_health.preprod block (ops/release/rung-health.py --write-record, at the soak's end)"]);
        assert_eq!(rh(&rh_record(rh_block(json!({"peak_memory_fraction": null})), json!({}))).len(), 1);
    }

    #[test]
    fn runtime_health_override_needs_who_and_why() {
        let ok = rh_record(rh_block(json!({"restarts": 3})), json!({"runtime_health_override": {"preprod": {"by": "A Person", "reason": "host reboot"}}}));
        assert!(rh(&ok).is_empty());
        let nobody = rh_record(rh_block(json!({"restarts": 3})), json!({"runtime_health_override": {"preprod": {"by": "", "reason": "x"}}}));
        assert_eq!(rh(&nobody).len(), 1);
    }

    #[test]
    fn an_awaited_capture_reads_soaking_and_names_runtime_health() {
        let rec = rh_record(Value::Null, json!({"opened": "2026-08-11"}));
        let rd = candidate_readiness(&rec, Some(&rh_hop()), NaiveDate::from_ymd_opt(2026, 8, 12).unwrap()).unwrap();
        assert_eq!(rd["state"], json!("soaking"));
        let blocked = json!({"state": "blocked", "class": "ladder", "blockers": ["runtime-health: preprod OOM-killed 2 time(s) since deploy"]});
        assert_eq!(next_step(&blocked), "runtime health → graduate → PROD");
    }

    #[test]
    fn effective_is_goatnet_deviation_then_fleet() {
        let d = json!({"fleet": "f", "deviations": {"g": {"value": "v", "reason": "r"}, "h": {"absent": true, "reason": "r"}}});
        assert_eq!(effective(&d, "g"), ("value", Some(json!("v"))));
        assert_eq!(effective(&d, "h"), ("absent", None));
        assert_eq!(effective(&d, "x"), ("value", Some(json!("f"))));
        assert_eq!(effective(&json!({"fleet": null}), "x"), ("unresolved", None));
    }

    #[test]
    fn py_timestamp_str_matches_the_yaml_loader() {
        assert_eq!(py_timestamp_str("2026-08-11T10:00:00Z", false), "2026-08-11 10:00:00+00:00");
        assert_eq!(py_timestamp_str("2026-08-11T10:00:00Z", true), "2026-08-11T10:00:00Z");
        assert_eq!(py_timestamp_str("2026-08-11T10:00:00+02:00", false), "2026-08-11 10:00:00+02:00");
        assert_eq!(py_timestamp_str("not a stamp", false), "not a stamp");
    }

    #[test]
    fn json_dumps_escapes_non_ascii_as_the_script_does() {
        assert_eq!(json_dumps(&json!({"a": "—"})), "{\n  \"a\": \"\\u2014\"\n}");
    }

    #[test]
    fn dump_record_quotes_nested_timestamps() {
        let mut rec = Map::new();
        rec.insert("runtime_health".into(), json!({"preprod": {"captured": "2026-10-03T10:00:00Z", "restarts": 0}}));
        let text = dump_record(&rec);
        assert!(text.contains("captured: '2026-10-03T10:00:00Z'"), "{text}");
    }

    #[test]
    fn dump_record_quotes_dates_and_keeps_field_order() {
        let mut rec = Map::new();
        rec.insert("component".into(), json!("x"));
        rec.insert("opened".into(), json!("2026-09-06"));
        rec.insert("evidence".into(), json!({"validation": "", "preprod_convergence": ""}));
        rec.insert("graduated".into(), Value::Null);
        let text = dump_record(&rec);
        assert!(text.starts_with("component: x\nopened: '2026-09-06'\n"), "{text}");
        assert!(text.contains("evidence:\n  validation: ''\n  preprod_convergence: ''\n"), "{text}");
        assert!(text.contains("graduated: null\n"), "{text}");
    }

    #[test]
    fn next_step_names_the_prod_change() {
        assert_eq!(next_step(&json!({"state": "ready", "class": "ladder"})), "graduate → PROD");
        assert_eq!(next_step(&json!({"state": "ready", "class": "hotfix"})), "graduate → no deploy");
        assert_eq!(next_step(&json!({"state": "blocked", "class": "ladder", "blockers": ["evidence.validation is empty", "operator-ack gate: x"]})), "evidence + ack → graduate → PROD");
        assert_eq!(next_step(&json!({"state": "soaking", "class": "ladder", "runnable": "2026-10-30", "fill_first": ["validation"]})), "fill evidence → graduate on 2026-10-30 → PROD");
    }

    #[test]
    fn the_repository_tree_grades_as_the_script_grades_it() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !repo.join("ops/release/ladder.yaml").is_file() {
            return;
        }
        let t = Tree::new(&repo);
        let st = build_status(&t).expect("status");
        assert!(!st.rows.is_empty());
        for row in &st.rows {
            let g = row["grade"].as_str().unwrap();
            assert!(GRADE_ORDER.contains(&g) || ["exempt", "env-local", "single-rung"].contains(&g), "{g}");
        }
        let (failures, _, _, _, _) = check_failures(&t).expect("check");
        assert!(failures.is_empty(), "{failures:?}");
    }
}

// --- what the drivers (release, fastlane) call ----------------------------------------------------

/// One subcommand run as a library call: the script's printed lines, and its exit code.
pub fn call(repo: &Path, cmd: &Sub) -> (Vec<String>, i32) {
    let t = Tree::new(repo);
    let mut o = Out::new();
    let code = match cmd {
        Sub::Open { component, reference, reason } => cmd_open(&t, component, reference, reason, &mut o),
        Sub::Graduate { component, ack, override_runtime_health } => cmd_graduate(&t, component, ack.as_deref(), override_runtime_health.as_deref(), false, &mut o),
        Sub::Hotfix { component, reference, reason, ack, previous_ref } => cmd_hotfix(&t, component, reference, reason, ack.as_deref(), previous_ref.as_deref(), &mut o),
        Sub::Abandon { record, reason, superseded_by } => cmd_abandon(&t, record, reason, superseded_by.as_deref(), &mut o),
        Sub::Check => cmd_check(&t, &mut o),
        _ => Err(anyhow!("promote::call: {cmd:?} is not a driver step")),
    };
    let code = match code {
        Ok(c) => c,
        Err(e) => {
            o.err(e.to_string());
            1
        }
    };
    (o.lines.into_iter().map(|c| c.detail).collect(), code)
}

/// rc.sh's evidence edit: the record's empty `<field>: ''` becomes a folded scalar.
pub fn record_set_evidence(path: &Path, field: &str, text: &str) -> Result<()> {
    let s = std::fs::read_to_string(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let from = format!("  {field}: ''\n");
    let to = format!("  {field}: >-\n    {text}\n");
    std::fs::write(path, s.replacen(&from, &to, 1)).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Every other OPEN ladder candidate for the component (rc.sh's preprod step
/// closes them as superseded once the rung runs the new candidate; C6).
pub fn stale_ladder_candidates(repo: &Path, component: &str, except_file: &str) -> Vec<String> {
    let t = Tree::new(repo);
    t.load_records()
        .iter()
        .filter(|r| r.error.is_none() && r.s("component").as_deref() == Some(component))
        .filter(|r| r.path.file_name().map(|n| n != except_file).unwrap_or(true))
        .filter(|r| r.s("state").as_deref() == Some("candidate") && r.class() != "hotfix")
        .map(|r| t.rel(&r.path))
        .collect()
}

/// OPEN hotfix records whose ref is `reference` (the fleet value a graduation replaces).
pub fn hotfix_candidates_with_ref(repo: &Path, component: &str, reference: &str) -> Vec<String> {
    let t = Tree::new(repo);
    t.load_records()
        .iter()
        .filter(|r| r.error.is_none() && r.s("component").as_deref() == Some(component))
        .filter(|r| r.class() == "hotfix" && r.s("state").as_deref() == Some("candidate") && r.s("candidate_ref").as_deref() == Some(reference))
        .map(|r| t.rel(&r.path))
        .collect()
}

/// The refs of every OPEN hotfix record for the component.
pub fn open_hotfix_refs(repo: &Path, component: &str) -> Vec<String> {
    let t = Tree::new(repo);
    t.load_records()
        .iter()
        .filter(|r| r.error.is_none() && r.s("component").as_deref() == Some(component))
        .filter(|r| r.class() == "hotfix" && r.s("state").as_deref() == Some("candidate"))
        .filter_map(|r| r.s("candidate_ref"))
        .collect()
}

/// OPEN candidates for the component whose ref is NOT `new_ref` (what a fastlane overtakes).
pub fn candidates_other_than(repo: &Path, component: &str, new_ref: &str) -> Vec<String> {
    let t = Tree::new(repo);
    t.load_records()
        .iter()
        .filter(|r| r.error.is_none() && r.s("component").as_deref() == Some(component))
        .filter(|r| r.s("state").as_deref() == Some("candidate") && r.s("candidate_ref").as_deref() != Some(new_ref))
        .map(|r| t.rel(&r.path))
        .collect()
}

/// The open LADDER candidates for a component that check C6 would refuse: the
/// preprod rung runs a different ref and the dev rung does not run theirs
/// either — overtaken, usually by a hotfix. Each comes with the record that
/// overtook it when one is open (the latest open hotfix record's file name);
/// otherwise the caller names the new candidate's ref. Returned as
/// (relative path, hotfix file name or None), oldest first.
pub fn stale_candidates_c6(repo: &Path, component: &str) -> Vec<(String, Option<String>)> {
    let t = Tree::new(repo);
    let (_, _, live) = match t.load_ladder() {
        Ok(x) => x,
        Err(_) => return Vec::new(),
    };
    let pins = match t.load_pins() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let comp_def = match get(&pins, "components").and_then(|c| c.get(component)) {
        Some(c) => c.clone(),
        None => return Vec::new(),
    };
    let pre = match live.iter().find(|r| rung_name(r) == "preprod") {
        Some(p) => rung_goatnet(p),
        None => return Vec::new(),
    };
    let dev = live.iter().find(|r| rung_name(r) == "dev").map(rung_goatnet);
    let records = t.load_records();
    let hotfix = latest_record(&records, component, Some("candidate"), None, Some("hotfix")).and_then(|h| h.path.file_name().map(|n| n.to_string_lossy().to_string()));
    let mut out = Vec::new();
    for rec in records.iter().filter(|r| r.error.is_none() && r.s("component").as_deref() == Some(component) && r.s("state").as_deref() == Some("candidate") && r.class() != "hotfix") {
        let (st, val) = effective(&comp_def, &pre);
        if st != "value" || val.map(|v| py_str(&v)) == rec.s("candidate_ref") {
            continue;
        }
        if let Some(d) = &dev {
            let (dst, dval) = effective(&comp_def, d);
            if dst == "value" && dval.map(|v| py_str(&v)) == rec.s("candidate_ref") {
                continue;
            }
        }
        out.push((t.rel(&rec.path), hotfix.clone()));
    }
    out
}

/// The OPEN records whose file name ends in `-<digest12>.yaml` (rc.sh's
/// `--from preprod-rows` lookup: the one record preprod's pinned tag carries).
pub fn open_records_by_digest(repo: &Path, component: &str, d12: &str) -> Vec<String> {
    let t = Tree::new(repo);
    t.load_records()
        .iter()
        .filter(|r| r.error.is_none() && r.s("component").as_deref() == Some(component))
        .filter(|r| r.path.file_name().map(|n| n.to_string_lossy().ends_with(&format!("-{d12}.yaml"))).unwrap_or(false))
        .filter(|r| r.s("state").as_deref() == Some("candidate"))
        .map(|r| t.rel(&r.path))
        .collect()
}

/// The open set's name for a component, or "" (fastlane's step 2).
pub fn open_set_name(repo: &Path, component: &str) -> String {
    let t = Tree::new(repo);
    open_set_for(&t.load_sets(), component).and_then(|d| d.s("name")).unwrap_or_default()
}

/// The soak gate's min_days on the preprod->prod hop (1 when unstated).
pub fn soak_min_days(repo: &Path) -> i64 {
    let t = Tree::new(repo);
    t.load_ladder()
        .ok()
        .and_then(|(ladder, _, _)| hop_preprod_prod(&ladder).ok())
        .and_then(|h| gates(&h).into_iter().find(|g| gate_id(g) == "soak"))
        .and_then(|g| get(&g, "min_days").and_then(|m| m.as_i64()))
        .unwrap_or(1)
}

#[cfg(test)]
mod driver_tests {
    use super::*;

    fn tree(name: &str) -> crate::testdir::Held<PathBuf> {
        let d = crate::testdir::path(&format!("goatbuilder-promote-{name}-"));
        let recs = d.join("ops/release/records/comp");
        std::fs::create_dir_all(&recs).unwrap();
        let w = |f: &str, body: &str| std::fs::write(recs.join(f), body).unwrap();
        w("20260901-aaaaaaaaaaaa.yaml", "component: comp\nclass: ladder\ncandidate_ref: 'r@sha256:a'\nstate: candidate\nopened: '2026-09-01'\n");
        w("20260902-hotfix-bbbbbbbbbbbb.yaml", "component: comp\nclass: hotfix\ncandidate_ref: 'r@sha256:b'\nstate: candidate\nopened: '2026-09-02'\n");
        w("20260903-cccccccccccc.yaml", "component: comp\nclass: ladder\ncandidate_ref: 'r@sha256:c'\nstate: candidate\nopened: '2026-09-03'\n");
        w("20260804-dddddddddddd.yaml", "component: comp\nclass: ladder\ncandidate_ref: 'r@sha256:d'\nstate: graduated\nopened: '2026-08-04'\n");
        d
    }

    #[test]
    fn stale_ladder_candidates_are_the_other_open_ladder_records() {
        let d = tree("stale");
        assert_eq!(stale_ladder_candidates(&d, "comp", "20260903-cccccccccccc.yaml"), vec!["ops/release/records/comp/20260901-aaaaaaaaaaaa.yaml".to_string()]);
        assert!(stale_ladder_candidates(&d, "other", "x").is_empty());
    }

    #[test]
    fn hotfix_candidates_with_ref_and_candidates_other_than() {
        let d = tree("hot");
        assert_eq!(hotfix_candidates_with_ref(&d, "comp", "r@sha256:b"), vec!["ops/release/records/comp/20260902-hotfix-bbbbbbbbbbbb.yaml".to_string()]);
        assert!(hotfix_candidates_with_ref(&d, "comp", "r@sha256:a").is_empty());
        assert_eq!(candidates_other_than(&d, "comp", "r@sha256:c"), vec!["ops/release/records/comp/20260901-aaaaaaaaaaaa.yaml".to_string(), "ops/release/records/comp/20260902-hotfix-bbbbbbbbbbbb.yaml".to_string()]);
    }

    /// The dev-rung defect of: a candidate overtaken on preprod by
    /// a hotfix left an open ladder record that check C6 refused, so no new
    /// candidate could start at dev. The finder names it, and what overtook it.
    #[test]
    fn stale_candidates_c6_names_the_overtaken_record_and_the_hotfix_that_overtook_it() {
        let d = crate::testdir::path("goatbuilder-promote-c6-");
        std::fs::create_dir_all(d.join("ops/release/records/comp")).unwrap();
        std::fs::create_dir_all(d.join("ops/pins")).unwrap();
        std::fs::write(d.join("ops/release/ladder.yaml"), "rungs:\n  - {name: dev, goatnet: zagros_dev, status: live}\n  - {name: preprod, goatnet: zagros, status: live}\n  - {name: prod, goatnet: zagros_prod, status: live}\n").unwrap();
        let a = "r@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let h = "r@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let c = "r@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        // the hotfix moved fleet: (every rung) to h; the old candidate a is on no rung; c is the dev candidate
        std::fs::write(d.join("ops/pins/fleet-pins.yaml"), format!("goatnets:\n  zagros_dev: {{members: [m1]}}\n  zagros: {{members: [m2]}}\n  zagros_prod: {{members: [m3]}}\ncomponents:\n  comp:\n    var: comp_image\n    fleet: \"{h}\"\n    deviations:\n      zagros_dev:\n        value: \"{c}\"\n        reason: rc\n")).unwrap();
        let recs = d.join("ops/release/records/comp");
        std::fs::write(recs.join("20260911-aaaaaaaaaaaa.yaml"), format!("component: comp\nclass: ladder\ncandidate_ref: '{a}'\nstate: candidate\nopened: '2026-09-11'\n")).unwrap();
        std::fs::write(recs.join("20260920-hotfix-bbbbbbbbbbbb.yaml"), format!("component: comp\nclass: hotfix\ncandidate_ref: '{h}'\nstate: candidate\nopened: '2026-09-20'\noperator_ack: A\n")).unwrap();
        std::fs::write(recs.join("20261028-cccccccccccc.yaml"), format!("component: comp\nclass: ladder\ncandidate_ref: '{c}'\nstate: candidate\nopened: '2026-10-28'\n")).unwrap();
        let stale = stale_candidates_c6(&d, "comp");
        assert_eq!(stale, vec![("ops/release/records/comp/20260911-aaaaaaaaaaaa.yaml".to_string(), Some("20260920-hotfix-bbbbbbbbbbbb.yaml".to_string()))]);
        // and check C6 itself refuses the same record, in its words
        let (failures, _, _, _, _) = check_failures(&Tree::new(&d)).unwrap();
        assert!(failures.iter().any(|f| f.starts_with("C6: ops/release/records/comp/20260911-aaaaaaaaaaaa.yaml is open but zagros now runs a different ref")), "{failures:?}");
        // once abandoned with the hotfix named, nothing is stale and C6 is quiet
        let (lines, code) = call(&d, &Sub::Abandon { record: "ops/release/records/comp/20260911-aaaaaaaaaaaa.yaml".into(), reason: "Superseded".into(), superseded_by: Some("20260920-hotfix-bbbbbbbbbbbb.yaml".into()) });
        assert_eq!(code, 0, "{lines:?}");
        assert!(stale_candidates_c6(&d, "comp").is_empty());
        let (failures, _, _, _, _) = check_failures(&Tree::new(&d)).unwrap();
        assert!(!failures.iter().any(|f| f.starts_with("C6:")), "{failures:?}");
    }

    /// `--from preprod-rows` resumes at the one open record whose name carries
    /// the pinned tag's digest-12; two, or none, is a refusal upstream.
    #[test]
    fn open_records_by_digest_finds_the_one_open_record_for_a_digest() {
        let d = tree("d12");
        assert_eq!(open_records_by_digest(&d, "comp", "cccccccccccc"), vec!["ops/release/records/comp/20260903-cccccccccccc.yaml".to_string()]);
        // the graduated one is not open
        assert!(open_records_by_digest(&d, "comp", "dddddddddddd").is_empty());
        assert!(open_records_by_digest(&d, "comp", "000000000000").is_empty());
    }

    #[test]
    fn record_set_evidence_folds_the_empty_field_once() {
        let d = tree("ev");
        let p = d.join("ops/release/records/comp/rec.yaml");
        std::fs::write(&p, "evidence:\n  validation: ''\n  preprod_convergence: ''\n").unwrap();
        record_set_evidence(&p, "validation", "dev, 2026-10-28: PASS").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "evidence:\n  validation: >-\n    dev, 2026-10-28: PASS\n  preprod_convergence: ''\n");
    }
}

#[cfg(test)]
mod candidate_changes_tests {
    use super::candidate_changes;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
    fn commit(dir: &std::path::Path, file: &str, msg: &str) {
        let p = dir.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, msg).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
    }

    #[test]
    fn lists_only_the_components_own_commits_since_its_previous_tag() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        git(r, &["init", "-q"]);
        std::fs::create_dir_all(r.join(".github/workflows")).unwrap();
        std::fs::write(r.join(".github/workflows/svc-image.yml"), "on:\n  push:\n    paths:\n      - 'svc/**'\n").unwrap();
        commit(r, "svc/a", "feat(svc): first (#1)");
        git(r, &["tag", "svc/v0.1.0-rc.1"]);
        commit(r, "svc/b", "fix(svc): second (#2)");
        commit(r, "other/c", "fix(other): not ours (#3)");
        commit(r, "svc/d", "fix(svc): third (#4)");
        git(r, &["tag", "svc/v0.1.0-rc.2"]);

        let (line, changes) = candidate_changes(r, ".github/workflows/svc-image.yml", "svc", "svc/v0.1.0-rc.2", "HEAD");
        assert_eq!(line, "2 change(s) since svc/v0.1.0-rc.1: #4, #2");
        assert_eq!(changes, "- fix(svc): third (#4)\n- fix(svc): second (#2)");

        let (first, _) = candidate_changes(r, ".github/workflows/svc-image.yml", "svc", "svc/v0.1.0-rc.1", "HEAD");
        assert_eq!(first, "the first candidate for svc");
    }
}
