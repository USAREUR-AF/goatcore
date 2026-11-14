//! The converger interface (goatbuilder.md §"The converger interface") and its
//! scripted implementation on a fixture directory (goatbuilder-conformance.md
//! §"The fixture"). A verb never touches a host: it publishes a declaration,
//! observes the machines, and reads each machine's record through these three
//! calls. The push-Ansible and pull-agent convergers land behind the same trait,
//! each behind the parity gate; the scripted one is what the conformance suite,
//! the golden tests and `tutor` run on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// What one observation of one machine says (C7): done only when its record
/// names the declared ref; pending and unknown are never reported as done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Converged,
    Pending,
    Unknown,
}

/// One machine's record row, in ledger form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    pub ts: String,
    pub component: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub action: String,
    #[serde(default)]
    pub serial: u64,
    /// Who declared what this row converged to (C4).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub actor: String,
}

/// One accepted publication (C2: signed and ordered — the serial is the order).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    pub serial: u64,
    pub env: String,
    pub component: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub actor: String,
    pub ts: String,
    /// The publication's seal: sha256 over its other fields (C2). A row whose
    /// seal does not match its content was altered after it was published. On
    /// hosts the same guarantee is the IntentV1 serial and its detached
    /// signature, which `pins --check` verifies (CF-5).
    #[serde(default)]
    pub seal: String,
}

impl Publication {
    pub fn sealed(mut self) -> Self {
        self.seal = self.expected_seal();
        self
    }

    pub fn expected_seal(&self) -> String {
        use sha2::{Digest, Sha256};
        let body = format!("{}\t{}\t{}\t{}\t{}\t{}", self.serial, self.env, self.component, self.reference, self.actor, self.ts);
        Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    }
}

pub trait Converger {
    /// The machines a declaration for this environment must reach.
    fn machines(&self, env: &str) -> Vec<String>;
    /// Publish a declaration; returns its serial. A serial not greater than the
    /// last accepted one is refused (C2).
    fn publish(&mut self, env: &str, component: &str, reference: &str, actor: &str) -> Result<u64>;
    /// One observation of every machine for this declaration.
    fn observe(&mut self, env: &str, component: &str, reference: &str) -> Result<BTreeMap<String, Outcome>> {
        self.observe_on(env, component, reference, None)
    }
    /// One observation of the named machines only — one class of a site, as
    /// `standup` converges class by class — or of every machine when `None`.
    fn observe_on(&mut self, env: &str, component: &str, reference: &str, only: Option<&[String]>) -> Result<BTreeMap<String, Outcome>>;
    /// The machine's last record for the component; `Ok(None)` when it has
    /// none, `Err` when it cannot be read (C7: unknown).
    fn read_record(&self, env: &str, machine: &str, component: &str) -> Result<Option<Record>>;
}

/// `converger.yaml`: one word (or `after: N`) per machine.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Behaviour {
    Word(String),
    After { after: u32 },
}

#[derive(Debug, Deserialize)]
struct Script {
    machines: BTreeMap<String, BTreeMap<String, Behaviour>>,
}

/// The scripted converger over a fixture directory. Observation counts for
/// `after: N` machines persist in `state/ticks.json` so a second run resumes
/// where the first stopped (C5).
pub struct Scripted {
    root: PathBuf,
    script: Script,
    ticks: BTreeMap<String, u32>,
    /// Each environment's converger kind from goatbuilder.yaml (C6); an
    /// environment not named here is push-ansible.
    kinds: BTreeMap<String, String>,
}

impl Scripted {
    pub fn open(root: &Path) -> Result<Self> {
        let p = root.join("converger.yaml");
        let text = std::fs::read_to_string(&p).with_context(|| format!("{}", p.display()))?;
        let script: Script = serde_yaml::from_str(&text).with_context(|| format!("{}", p.display()))?;
        let ticks = match std::fs::read_to_string(root.join("state/ticks.json")) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        };
        Ok(Scripted { root: root.to_path_buf(), script, ticks, kinds: BTreeMap::new() })
    }

    /// The converger kind per environment, from the fixture's config (C6).
    pub fn set_kinds(&mut self, kinds: BTreeMap<String, String>) {
        self.kinds = kinds;
    }

    /// Record that a transport to an environment's machines was opened (C6):
    /// a push converger reaches into the machines; a pull agent's machines
    /// fetch the declaration themselves, so nothing is opened for them.
    fn open_transport(&self, env: &str) -> Result<()> {
        let kind = self.kinds.get(env).map(String::as_str).unwrap_or("push-ansible");
        if kind == "pull-agent" {
            return Ok(());
        }
        std::fs::create_dir_all(self.root.join("state"))?;
        let p = self.root.join("state/transports.jsonl");
        let mut text = std::fs::read_to_string(&p).unwrap_or_default();
        text.push_str(&serde_json::to_string(&serde_json::json!({ "env": env, "transport": kind }))?);
        text.push('\n');
        std::fs::write(&p, text).with_context(|| format!("{}", p.display()))
    }

    /// Every publication, checked (C2): each row's seal matches its content and
    /// each serial is greater than every serial before it. Returns the rows.
    pub fn publications(&self) -> Result<Vec<Publication>> {
        let p = self.root.join("declared/publications.jsonl");
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(anyhow!("{}: {e}", p.display())),
        };
        let mut out: Vec<Publication> = Vec::new();
        for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let n = i + 1;
            let pubn: Publication = serde_json::from_str(line)
                .map_err(|e| goat_errors::coded("GB-045", format!("declared/publications.jsonl line {n}: not a publication ({e})")))?;
            if pubn.seal != pubn.expected_seal() {
                return Err(goat_errors::coded("GB-045", format!("declared/publications.jsonl line {n} (serial {}): the signature does not match its content; it was altered after it was published", pubn.serial)).into());
            }
            if let Some(last) = out.iter().map(|p| p.serial).max() {
                if pubn.serial <= last {
                    return Err(goat_errors::coded("GB-045", format!("declared/publications.jsonl line {n}: serial {} is older than last accepted {last}; a replayed or reordered declaration is never applied", pubn.serial)).into());
                }
            }
            out.push(pubn);
        }
        Ok(out)
    }

    fn behaviour(&self, env: &str, machine: &str) -> Option<&Behaviour> {
        self.script.machines.get(env).and_then(|m| m.get(machine))
    }

    fn record_path(&self, env: &str, machine: &str) -> PathBuf {
        self.root.join("records").join(env).join(format!("{machine}.jsonl"))
    }

    fn append_record(&self, env: &str, machine: &str, rec: &Record) -> Result<()> {
        let p = self.record_path(env, machine);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut text = std::fs::read_to_string(&p).unwrap_or_default();
        text.push_str(&serde_json::to_string(rec)?);
        text.push('\n');
        std::fs::write(&p, text).with_context(|| format!("{}", p.display()))
    }

    /// A teardown: append an `action: teardown` row with an empty ref. History
    /// is kept — nothing earlier in the machine's record is removed.
    pub fn record_teardown(&self, env: &str, machine: &str, component: &str) -> Result<()> {
        let rec = Record { ts: crate::push::stamp_pub(), component: component.into(), reference: String::new(), action: "teardown".into(), serial: self.last_serial(), actor: String::new() };
        self.append_record(env, machine, &rec)
    }

    fn save_ticks(&self) -> Result<()> {
        std::fs::create_dir_all(self.root.join("state"))?;
        std::fs::write(self.root.join("state/ticks.json"), serde_json::to_string(&self.ticks)?)?;
        Ok(())
    }

    pub fn last_serial(&self) -> u64 {
        std::fs::read_to_string(self.root.join("declared/publications.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Publication>(l).ok())
            .map(|p| p.serial)
            .max()
            .unwrap_or(0)
    }

    /// Records are "converged" when they name the declared ref; a machine the
    /// script marks unreadable answers Err.
    fn current(&self, env: &str, machine: &str, component: &str, reference: &str) -> Result<bool> {
        Ok(self
            .read_record(env, machine, component)?
            .map(|r| r.reference == reference)
            .unwrap_or(false))
    }
}

fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_utc(secs)
}

/// Seconds since the Unix epoch as ISO 8601 UTC (2026-11-01T11:58:05Z), the
/// format every other record uses; the date from the day count is Howard
/// Hinnant's civil_from_days, so no date crate is needed.
fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

impl Converger for Scripted {
    fn machines(&self, env: &str) -> Vec<String> {
        self.script
            .machines
            .get(env)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn publish(&mut self, env: &str, component: &str, reference: &str, actor: &str) -> Result<u64> {
        if actor.trim().is_empty() {
            return Err(goat_errors::coded("GB-047", format!("{env}/{component}: a declaration names who made it; pass --ack \"<your name>\" (C4)")).into());
        }
        let serial = self.publications()?.iter().map(|p| p.serial).max().unwrap_or(0) + 1;
        let p = self.root.join("declared/publications.jsonl");
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let pubn = Publication {
            serial,
            env: env.to_string(),
            component: component.to_string(),
            reference: reference.to_string(),
            actor: actor.to_string(),
            ts: now(),
            seal: String::new(),
        }
        .sealed();
        let mut text = std::fs::read_to_string(&p).unwrap_or_default();
        text.push_str(&serde_json::to_string(&pubn)?);
        text.push('\n');
        std::fs::write(&p, text)?;
        Ok(serial)
    }

    fn observe_on(&mut self, env: &str, component: &str, reference: &str, only: Option<&[String]>) -> Result<BTreeMap<String, Outcome>> {
        let last = self.publications()?.into_iter().last();
        let serial = last.as_ref().map(|p| p.serial).unwrap_or(0);
        let actor = last.filter(|p| p.env == env && p.component == component).map(|p| p.actor).unwrap_or_default();
        self.open_transport(env)?;
        let mut out = BTreeMap::new();
        for machine in self.machines(env).into_iter().filter(|m| only.map(|o| o.contains(m)).unwrap_or(true)) {
            let key = format!("{env}/{machine}");
            let behaviour = self.behaviour(env, &machine).cloned();
            let outcome = match behaviour {
                Some(Behaviour::Word(w)) if w == "unreadable" => Outcome::Unknown,
                Some(Behaviour::Word(w)) if w == "never" => Outcome::Pending,
                Some(Behaviour::Word(w)) if w == "converge-now" => {
                    if !self.current(env, &machine, component, reference)? {
                        self.append_record(env, &machine, &Record {
                            ts: now(),
                            component: component.to_string(),
                            reference: reference.to_string(),
                            action: "deploy".to_string(),
                            serial,
                            actor: actor.clone(),
                        })?;
                    }
                    Outcome::Converged
                }
                Some(Behaviour::After { after }) => {
                    if self.current(env, &machine, component, reference)? {
                        Outcome::Converged
                    } else {
                        let t = self.ticks.entry(key).or_insert(0);
                        *t += 1;
                        if *t >= after {
                            self.append_record(env, &machine, &Record {
                                ts: now(),
                                component: component.to_string(),
                                reference: reference.to_string(),
                                action: "deploy".to_string(),
                                serial,
                                actor: actor.clone(),
                            })?;
                            Outcome::Converged
                        } else {
                            Outcome::Pending
                        }
                    }
                }
                Some(Behaviour::Word(w)) => return Err(goat_errors::coded("GB-024", format!("converger.yaml: {machine}: '{w}' is not a behaviour this converger knows")).into()),
                None => Outcome::Unknown,
            };
            out.insert(machine, outcome);
        }
        self.save_ticks()?;
        Ok(out)
    }

    fn read_record(&self, env: &str, machine: &str, component: &str) -> Result<Option<Record>> {
        if let Some(Behaviour::Word(w)) = self.behaviour(env, machine) {
            if w == "unreadable" {
                return Err(goat_errors::coded("GB-024", format!("{machine}: record unreadable")).into());
            }
        }
        let p = self.record_path(env, machine);
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", p.display())))),
        };
        let last = text
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<Record>(l).ok())
            .find(|r| r.component == component);
        Ok(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock records carry ISO 8601 UTC like every other record (the walkthrough
    /// saw `epoch+20758d43085s` beside `2026-10-20T10:00:00Z` in `history`).
    #[test]
    fn mock_timestamps_are_iso_utc() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(1_793_534_285), "2026-11-01T11:58:05Z");
    }

    fn fixture(script: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("converger.yaml"), script).unwrap();
        t
    }

    const SCRIPT: &str = "machines:\n  dev:\n    now: converge-now\n    later: { after: 2 }\n    stuck: never\n    dark: unreadable\n";

    #[test]
    fn each_publication_takes_the_next_serial() {
        let t = fixture(SCRIPT);
        let mut c = Scripted::open(t.path()).unwrap();
        assert_eq!(c.last_serial(), 0);
        assert_eq!(c.publish("dev", "portal", "r1", "A").unwrap(), 1);
        assert_eq!(c.publish("dev", "portal", "r2", "A").unwrap(), 2);
        assert_eq!(Scripted::open(t.path()).unwrap().last_serial(), 2);
    }

    #[test]
    fn never_stays_pending_and_unreadable_is_unknown_never_done() {
        let t = fixture(SCRIPT);
        let mut c = Scripted::open(t.path()).unwrap();
        c.publish("dev", "portal", "r1", "A").unwrap();
        let o = c.observe("dev", "portal", "r1").unwrap();
        assert!(matches!(o["stuck"], Outcome::Pending));
        assert!(matches!(o["dark"], Outcome::Unknown));
        assert!(c.read_record("dev", "dark", "portal").is_err());
        assert!(c.read_record("dev", "stuck", "portal").unwrap().is_none());
    }

    #[test]
    fn converge_now_writes_one_record_and_a_second_observation_adds_none() {
        let t = fixture(SCRIPT);
        let mut c = Scripted::open(t.path()).unwrap();
        c.publish("dev", "portal", "r1", "A").unwrap();
        c.observe("dev", "portal", "r1").unwrap();
        let path = t.path().join("records/dev/now.jsonl");
        let once = std::fs::read_to_string(&path).unwrap();
        c.observe("dev", "portal", "r1").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once, "C5: no second record");
        assert_eq!(c.read_record("dev", "now", "portal").unwrap().unwrap().reference, "r1");
    }

    #[test]
    fn after_n_is_pending_until_the_nth_observation_and_resumes_across_reopen() {
        let t = fixture(SCRIPT);
        {
            let mut c = Scripted::open(t.path()).unwrap();
            c.publish("dev", "portal", "r1", "A").unwrap();
            assert!(matches!(c.observe("dev", "portal", "r1").unwrap()["later"], Outcome::Pending));
        }
        let mut c = Scripted::open(t.path()).unwrap();
        assert!(matches!(c.observe("dev", "portal", "r1").unwrap()["later"], Outcome::Converged), "the tick count survives a new run");
    }

    #[test]
    fn a_new_ref_is_not_converged_by_an_old_record() {
        let t = fixture(SCRIPT);
        let mut c = Scripted::open(t.path()).unwrap();
        c.publish("dev", "portal", "r1", "A").unwrap();
        c.observe("dev", "portal", "r1").unwrap();
        assert!(!c.current("dev", "now", "portal", "r2").unwrap());
    }

    #[test]
    fn the_newest_record_for_the_asked_component_wins() {
        let t = fixture(SCRIPT);
        let mut c = Scripted::open(t.path()).unwrap();
        c.publish("dev", "portal", "r1", "A").unwrap();
        c.observe("dev", "portal", "r1").unwrap();
        c.observe("dev", "netvis", "n1").unwrap();
        c.observe("dev", "portal", "r2").unwrap();
        assert_eq!(c.read_record("dev", "now", "portal").unwrap().unwrap().reference, "r2");
        assert_eq!(c.read_record("dev", "now", "netvis").unwrap().unwrap().reference, "n1");
    }

    #[test]
    fn observe_on_touches_only_the_named_machines() {
        let t = fixture(SCRIPT);
        let mut c = Scripted::open(t.path()).unwrap();
        c.publish("dev", "portal", "r1", "A").unwrap();
        let only = vec!["now".to_string()];
        let o = c.observe_on("dev", "portal", "r1", Some(&only)).unwrap();
        assert_eq!(o.len(), 1);
        assert!(!t.path().join("records/dev/later.jsonl").exists());
    }

    #[test]
    fn an_unknown_behaviour_word_is_refused_naming_the_machine() {
        let t = fixture("machines:\n  dev:\n    odd: sometimes\n");
        let mut c = Scripted::open(t.path()).unwrap();
        let e = c.observe("dev", "portal", "r1").unwrap_err().to_string();
        assert!(e.contains("odd") && e.contains("sometimes"), "{e}");
    }

    #[test]
    fn an_undeclared_environment_has_no_machines() {
        let t = fixture(SCRIPT);
        let c = Scripted::open(t.path()).unwrap();
        assert!(c.machines("staging").is_empty());
        assert_eq!(c.machines("dev").len(), 4);
    }

    #[test]
    fn a_missing_script_is_an_error_naming_the_file() {
        let t = tempfile::tempdir().unwrap();
        let e = Scripted::open(t.path()).err().unwrap().to_string();
        assert!(e.contains("converger.yaml"), "{e}");
    }
}

