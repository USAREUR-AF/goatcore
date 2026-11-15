//! The converger interface (goatbuilder.md §"The converger interface") and its
//! scripted implementation on a fixture directory (goatbuilder-conformance.md
//! §"The fixture"). A verb never touches a host: it publishes a declaration,
//! observes the machines, and reads each machine's record through these three
//! calls. The push-Ansible and pull-agent convergers land behind the same trait,
//! each behind the parity gate; the scripted one is what the conformance suite,
//! the golden tests and `tutor` run on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// The code a real-host verb and `doctor` refuse `converger: pull-agent` with:
/// the pull converger is not built, and the push converger must not stand in
/// for it (docs/design/goatbuilder-pull-converger-options.md).
pub const PULL_NOT_BUILT: &str = "GB-307";

/// Where an environment declares `converger: pull-agent`, as each converger
/// resolves (a class's value, else its site's, else the environment's): the
/// environment itself, `site <s>`, or `site <s> class <c>`. Empty when every
/// machine of the environment resolves to the push converger.
pub fn pull_agent_places(env: &crate::config::Environment) -> Vec<String> {
    const PULL: &str = "pull-agent";
    let mut places = Vec::new();
    let mut whole_sites = 0;
    for site in &env.sites {
        let at_site = site.converger.as_deref().unwrap_or(&env.converger);
        let pulled: Vec<&str> = site
            .classes
            .iter()
            .filter(|(_, c)| c.converger.as_deref().unwrap_or(at_site) == PULL)
            .map(|(k, _)| k.as_str())
            .collect();
        let whole = if site.classes.is_empty() { at_site == PULL } else { pulled.len() == site.classes.len() };
        if whole {
            whole_sites += 1;
            places.push(format!("site {}", site.name));
        } else {
            places.extend(pulled.into_iter().map(|c| format!("site {} class {c}", site.name)));
        }
    }
    // The environment's own value, reaching every machine: name it once.
    if env.converger == PULL && whole_sites == env.sites.len() {
        return vec![format!("environment {}", env.name)];
    }
    places
}

/// How the real-host verbs reach an environment's machines: `deploy`, `release`,
/// `rollback` and `doctor` take their converger from [`for_environment`] (or
/// [`for_config`]) and never decide push or pull themselves, so the pull
/// converger, when it is built, is a new implementation here and not a change to
/// the verbs (docs/design/goatbuilder-pull-converger-options.md, option A).
/// `push-ansible` is the one implementation; `pull-agent` refuses (GB-307). The
/// [`Converger`] trait below is the conformance suite's interface, which the
/// scripted fixture implements; this one is what the real-host verbs call.
pub trait HostConverger {
    /// Ok when a verb may converge this environment's machines; else the refusal.
    fn admit(&self) -> Result<()>;
    /// `deploy` on real hosts.
    fn deploy(&self, repo: &Path, o: &crate::push::Options) -> Result<crate::Report>;
}

/// The push converger: Ansible plays run from the operator's machine.
pub struct PushAnsible;

impl HostConverger for PushAnsible {
    fn admit(&self) -> Result<()> {
        Ok(())
    }
    fn deploy(&self, repo: &Path, o: &crate::push::Options) -> Result<crate::Report> {
        crate::push::deploy_by_push(repo, o)
    }
}

/// The pull converger, which is not built: an environment declaring it anywhere
/// is refused (PULL_NOT_BUILT), and the push converger never stands in for it.
pub struct PullAgent {
    env: String,
    places: Vec<String>,
}

impl HostConverger for PullAgent {
    fn admit(&self) -> Result<()> {
        Err(goat_errors::coded(PULL_NOT_BUILT, pull_agent_refusal(&self.env, &self.places)).into())
    }
    fn deploy(&self, repo: &Path, o: &crate::push::Options) -> Result<crate::Report> {
        // A verify-only deploy reads each host's ledger and converges nothing, so
        // it is not refused (as before the trait).
        if o.verify_only {
            return crate::push::deploy_by_push(repo, o);
        }
        self.admit()?;
        unreachable!("admit refuses the pull converger")
    }
}

/// The converger an environment declares, as `doctor` reads it from a parsed
/// config: the pull converger when any machine resolves to `pull-agent`.
pub fn for_config(env: &crate::config::Environment) -> Box<dyn HostConverger> {
    let places = pull_agent_places(env);
    if places.is_empty() {
        Box::new(PushAnsible)
    } else {
        Box::new(PullAgent { env: env.name.clone(), places })
    }
}

/// The converger the repository's `goatbuilder.yaml` declares for `env`. A
/// checkout with no readable config, or no such environment, is pushed to as it
/// always was.
pub fn for_environment(repo: &Path, env: &str) -> Box<dyn HostConverger> {
    let path = repo.join("ops/goatbuilder/goatbuilder.yaml");
    let Ok(cfg) = crate::config::load(&path) else { return Box::new(PushAnsible) };
    let wanted = env.replace('_', "-");
    match cfg.environments.iter().find(|e| e.name == env || e.name.replace('_', "-") == wanted) {
        Some(e) => for_config(e),
        None => Box::new(PushAnsible),
    }
}

/// Refuses (PULL_NOT_BUILT) when the repository's `goatbuilder.yaml` declares
/// `converger: pull-agent` anywhere in the environment `env`: the converger's
/// [`HostConverger::admit`]. A checkout with no readable config, or no such
/// environment, is not refused here.
pub fn refuse_pull_agent(repo: &Path, env: &str) -> Result<()> {
    for_environment(repo, env).admit()
}

/// The refusal's words, shared by `doctor` and the real-host verbs.
pub fn pull_agent_refusal(env: &str, places: &[String]) -> String {
    format!(
        "environment {env} declares converger: pull-agent ({}); the pull converger is not built, and goatbuilder will not push to machines declared to pull",
        places.join(", ")
    )
}

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
        let text = std::fs::read_to_string(&p).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", p.display())))?;
        let script: Script = serde_yaml::from_str(&text).map_err(|e| goat_errors::coded("GB-024", format!("{}: {e}", p.display())))?;
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
        std::fs::create_dir_all(self.root.join("state")).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(self.root.join("state"))).display())))?;
        let p = self.root.join("state/transports.jsonl");
        let mut text = std::fs::read_to_string(&p).unwrap_or_default();
        text.push_str(&serde_json::to_string(&serde_json::json!({ "env": env, "transport": kind })).map_err(|e| goat_errors::coded("GB-322", format!("could not serialize: {e}")))?);
        text.push('\n');
        std::fs::write(&p, text).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", p.display())).into())
    }

    /// Every publication, checked (C2): each row's seal matches its content and
    /// each serial is greater than every serial before it. Returns the rows.
    pub fn publications(&self) -> Result<Vec<Publication>> {
        let p = self.root.join("declared/publications.jsonl");
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(goat_errors::coded("GB-254", format!("{}: {e}", p.display())).into()),
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
            std::fs::create_dir_all(d).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(d)).display())))?;
        }
        let mut text = std::fs::read_to_string(&p).unwrap_or_default();
        text.push_str(&serde_json::to_string(rec).map_err(|e| goat_errors::coded("GB-322", format!("could not serialize: {e}")))?);
        text.push('\n');
        std::fs::write(&p, text).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", p.display())).into())
    }

    /// A teardown: append an `action: teardown` row with an empty ref. History
    /// is kept — nothing earlier in the machine's record is removed.
    pub fn record_teardown(&self, env: &str, machine: &str, component: &str) -> Result<()> {
        let rec = Record { ts: crate::push::stamp_pub(), component: component.into(), reference: String::new(), action: "teardown".into(), serial: self.last_serial(), actor: String::new() };
        self.append_record(env, machine, &rec)
    }

    fn save_ticks(&self) -> Result<()> {
        std::fs::create_dir_all(self.root.join("state")).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(self.root.join("state"))).display())))?;
        std::fs::write(self.root.join("state/ticks.json"), serde_json::to_string(&self.ticks).map_err(|e| goat_errors::coded("GB-322", format!("could not serialize: {e}")))?).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(self.root.join("state/ticks.json"))).display())))?;
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
            std::fs::create_dir_all(d).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(d)).display())))?;
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
        text.push_str(&serde_json::to_string(&pubn).map_err(|e| goat_errors::coded("GB-322", format!("could not serialize: {e}")))?);
        text.push('\n');
        std::fs::write(&p, text).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(&p)).display())))?;
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

    fn env_of(v: serde_json::Value) -> crate::config::Environment {
        serde_json::from_value(v).unwrap()
    }

    /// Every place an environment can declare the pull converger is named, as
    /// each machine's converger resolves (class, else site, else environment).
    #[test]
    fn pull_agent_places_names_the_environment_a_site_and_a_class() {
        use serde_json::json;
        let push = env_of(json!({"name": "dev", "converger": "push-ansible", "sites": [{"name": "alpha", "classes": {"core": {}}}]}));
        assert!(pull_agent_places(&push).is_empty());
        let whole = env_of(json!({"name": "dev", "converger": "pull-agent"}));
        assert_eq!(pull_agent_places(&whole), vec!["environment dev".to_string()]);
        let site = env_of(json!({"name": "dev", "converger": "push-ansible", "sites": [{"name": "alpha", "converger": "pull-agent"}, {"name": "beta"}]}));
        assert_eq!(pull_agent_places(&site), vec!["site alpha".to_string()]);
        let class = env_of(json!({"name": "dev", "converger": "push-ansible", "sites": [{"name": "bravo", "classes": {"core": {}, "pod": {"converger": "pull-agent"}}}]}));
        assert_eq!(pull_agent_places(&class), vec!["site bravo class pod".to_string()]);
        // A site or class that overrides a pull-agent environment back to push
        // is not named; the rest of the environment still is.
        let back = env_of(json!({"name": "dev", "converger": "pull-agent", "sites": [{"name": "alpha", "converger": "push-ansible"}, {"name": "beta"}]}));
        assert_eq!(pull_agent_places(&back), vec!["site beta".to_string()]);
    }

    fn repo_with(converger: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/standup/goatbuilder.yaml");
        let text = std::fs::read_to_string(fixture).unwrap();
        let text = text.replacen("- name: dev\n    converger: push-ansible", &format!("- name: dev\n    converger: {converger}"), 1);
        std::fs::create_dir_all(t.path().join("ops/goatbuilder")).unwrap();
        std::fs::write(t.path().join("ops/goatbuilder/goatbuilder.yaml"), text).unwrap();
        t
    }

    /// A pull-agent environment is refused with the code, naming that pull is
    /// not built; a push environment, an unknown one and a checkout with no
    /// config are not refused here.
    #[test]
    fn refuse_pull_agent_refuses_only_a_pull_agent_environment() {
        let pull = repo_with("pull-agent");
        let e = refuse_pull_agent(pull.path(), "dev").unwrap_err();
        let shown = format!("{e:#}");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some(PULL_NOT_BUILT), "{shown}");
        assert!(shown.contains("not built") && shown.contains("environment dev"), "{shown}");
        assert!(refuse_pull_agent(pull.path(), "other").is_ok());
        let push = repo_with("push-ansible");
        assert!(refuse_pull_agent(push.path(), "dev").is_ok());
        let none = tempfile::tempdir().unwrap();
        assert!(refuse_pull_agent(none.path(), "dev").is_ok());
    }

    /// The real-host verbs take their converger from the environment's
    /// declaration: push-ansible is the one implementation and admits; an
    /// environment declaring pull-agent anywhere gets the pull converger, which
    /// refuses with GB-307 and the same words the verbs and doctor print, a
    /// verify-only deploy (which converges nothing) excepted. A checkout with no
    /// config, or no such environment, is pushed to as before.
    #[test]
    fn the_host_converger_follows_the_declaration_and_pull_refuses() {
        let push = repo_with("push-ansible");
        assert!(for_environment(push.path(), "dev").admit().is_ok());
        let pull = repo_with("pull-agent");
        let c = for_environment(pull.path(), "dev");
        let e = c.admit().unwrap_err();
        let coded = e.downcast_ref::<goat_errors::Coded>().expect("a coded refusal");
        assert_eq!(coded.code, PULL_NOT_BUILT);
        assert_eq!(coded.message, pull_agent_refusal("dev", &["environment dev".to_string()]));
        // The deploy entry refuses through the trait before any step, a dry run too.
        for dry_run in [false, true] {
            let o = crate::push::Options { component: "portal", goatnet: "dev", only_group: None, dry_run, verify_only: false, ack: "" };
            let e = c.deploy(pull.path(), &o).unwrap_err();
            assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some(PULL_NOT_BUILT), "{e:#}");
        }
        assert!(for_environment(pull.path(), "other").admit().is_ok());
        let none = tempfile::tempdir().unwrap();
        assert!(for_environment(none.path(), "dev").admit().is_ok());
        // The same choice from a parsed environment, as doctor makes it.
        let env = env_of(serde_json::json!({"name": "dev", "converger": "push-ansible", "sites": [{"name": "bravo", "classes": {"core": {}, "pod": {"converger": "pull-agent"}}}]}));
        let e = for_config(&env).admit().unwrap_err();
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.message.clone()), Some(pull_agent_refusal("dev", &["site bravo class pod".to_string()])));
    }

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

