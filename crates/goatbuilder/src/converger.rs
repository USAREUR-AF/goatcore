//! The converger interface (goatbuilder.md §"The converger interface") and its
//! scripted implementation on a fixture directory (goatbuilder-conformance.md
//! §"The fixture"). A verb never touches a host: it publishes a declaration,
//! observes the machines, and reads each machine's record through these three
//! calls. The push converger is behind the same trait (`Push`, gate D), and the
//! pull converger lands as another implementation of it; the scripted one is
//! what the conformance suite, the golden tests and `tutor` run on. A verb asks
//! [`select`] for the converger and calls its steps (`deploy`, `status`,
//! `verify`), whose defaults are written in the four calls alone.

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

/// Refuses (PULL_NOT_BUILT) when the repository's `goatbuilder.yaml` declares
/// `converger: pull-agent` anywhere in the environment `env`. A checkout with no
/// readable config, or no such environment, is not refused here.
pub fn refuse_pull_agent(repo: &Path, env: &str) -> Result<()> {
    let path = repo.join("ops/goatbuilder/goatbuilder.yaml");
    let Ok(cfg) = crate::config::load(&path) else { return Ok(()) };
    let wanted = env.replace('_', "-");
    let Some(e) = cfg.environments.iter().find(|e| e.name == env || e.name.replace('_', "-") == wanted) else { return Ok(()) };
    let places = pull_agent_places(e);
    if places.is_empty() {
        return Ok(());
    }
    Err(goat_errors::coded(PULL_NOT_BUILT, pull_agent_refusal(&e.name, &places)).into())
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

/// Where the declared refs come from: a fixture's `declared/pins.yaml`, or, for a
/// converger that uses the default steps on real hosts, the checkout's pins.
pub trait Declarations {
    /// Refuses an environment the configuration does not declare.
    fn environment(&self, env: &str) -> Result<()>;
    /// The components declared on the environment, or only `only`.
    fn components_on(&self, env: &str, only: Option<&str>) -> Vec<String>;
    /// The declared ref of a component on the environment (C1).
    fn declared_ref(&self, env: &str, component: &str) -> Result<String>;
    /// The machines that carry a component, when the declaration scopes it (a
    /// sited fixture's classes); `None` is every machine of the environment.
    fn scope(&self, _component: &str) -> Option<Vec<String>> {
        None
    }
    /// Before a deploy publishes anything: a fixture fast-forwards its
    /// declaration from the source of record (C3), and says so in a row.
    fn prepare(&mut self, _frozen: bool) -> Result<Option<crate::Check>> {
        Ok(None)
    }
}

/// What a converging verb asks a converger for.
#[derive(Debug, Clone, Copy)]
pub struct Converge<'a> {
    pub env: &'a str,
    pub component: Option<&'a str>,
    pub group: Option<&'a str>,
    /// One machine of `group`, real hosts only.
    pub node: Option<&'a str>,
    /// Who declares it (C4).
    pub actor: &'a str,
    pub dry_run: bool,
    /// Read every machine's record; deploy nothing.
    pub verify_only: bool,
    /// Refuse to fast-forward the declaration (C3).
    pub frozen: bool,
    /// The release ladder (ADR 1120, GB-373) on real hosts.
    pub ladder: crate::ladder::Mode<'a>,
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

    /// Checked before anything is published (C2: the scripted log is intact).
    fn before_publish(&self) -> Result<()> {
        Ok(())
    }
    /// The next serial a publication would take, when the converger knows it.
    fn next_serial(&self) -> Option<u64> {
        None
    }
    /// `deploy --dry-run`: what would be published, and to which machines.
    fn plan(&self, d: &dyn Declarations, req: &Converge) -> Result<crate::Report> {
        plan_with(self, d, req)
    }
    /// `deploy`: publish the declared ref, observe once, read every record back.
    fn deploy(&mut self, d: &mut dyn Declarations, req: &Converge) -> Result<crate::Report> {
        if req.dry_run {
            return self.plan(&*d, req);
        }
        deploy_with(self, d, req)
    }
    /// `status`: declared against recorded, per machine, reading only (C8).
    fn status(&self, d: &dyn Declarations, env: &str, only: Option<&str>) -> Result<crate::Report> {
        status_with(self, d, env, only)
    }
    /// `verify`: `status` for one component.
    fn verify(&self, d: &dyn Declarations, req: &Converge) -> Result<crate::Report> {
        self.status(d, req.env, req.component).map(|mut r| {
            r.verb = "verify";
            r
        })
    }
}

fn nothing_declared(env: &str, only: Option<&str>) -> anyhow::Error {
    anyhow::Error::from(goat_errors::coded("GB-255", format!("nothing declared for {env}{}", only.map(|o| format!(" and component {o}")).unwrap_or_default())))
}

fn mutable_tag(env: &str, component: &str, declared: &str) -> crate::Check {
    crate::Check {
        name: format!("{env}/{component}/declaration"),
        ok: false,
        detail: format!("{declared} is a mutable tag; a deploy declares a digest (C1)"),
        fix: Some("pin the declaration to <image>@sha256:<digest> in declared/pins.yaml".into()),
    }
}

fn row(name: &str, ok: bool, detail: String) -> crate::Check {
    crate::Check { name: name.to_string(), ok, detail, fix: None }
}

/// The default `deploy`, in the four calls alone. Done is what each machine's
/// RECORD says, never what the observation claimed (C7); a second run over a
/// converged environment changes nothing (C5).
pub fn deploy_with<C: Converger + ?Sized>(c: &mut C, d: &mut dyn Declarations, req: &Converge) -> Result<crate::Report> {
    let (env, only, actor) = (req.env, req.component, req.actor);
    d.environment(env)?;
    crate::verbs::attributed(actor)?;
    let mut checks: Vec<crate::Check> = d.prepare(req.frozen)?.into_iter().collect();
    // C2 before anything is published: the log is intact and in order.
    c.before_publish()?;
    let components = d.components_on(env, only);
    if components.is_empty() {
        return Err(nothing_declared(env, only));
    }
    for component in components {
        let declared = d.declared_ref(env, &component)?;
        if !declared.contains("@sha256:") {
            checks.push(mutable_tag(env, &component, &declared));
            continue;
        }
        let serial = c.publish(env, &component, &declared, actor)?;
        checks.push(row(&format!("{env}/{component}/publish"), true, format!("serial {serial}: {} by {actor}", crate::verbs::short(&declared))));
        let observed = c.observe(env, &component, &declared)?;
        for (machine, outcome) in observed {
            let name = format!("{env}/{machine}/{component}");
            match c.read_record(env, &machine, &component) {
                Ok(Some(r)) if r.reference == declared => checks.push(row(&name, true, format!("converged on {}", crate::verbs::short(&declared)))),
                Ok(_) => checks.push(row(&name, false, format!("pending ({})", match outcome { Outcome::Pending => "not yet converged", Outcome::Converged => "observation says converged but the record does not", Outcome::Unknown => "unknown" }))),
                Err(e) => checks.push(row(&name, false, format!("unknown: {e}"))),
            }
        }
    }
    let exit = if checks.iter().any(|c| c.fix.is_some()) { crate::EXIT_REFUSED } else { crate::EXIT_NOT_CONVERGED };
    Ok(crate::Report::new("deploy", checks, exit))
}

/// The default `deploy --dry-run`: what would be published, and to which
/// machines, with nothing published and no record written.
pub fn plan_with<C: Converger + ?Sized>(c: &C, d: &dyn Declarations, req: &Converge) -> Result<crate::Report> {
    let (env, only) = (req.env, req.component);
    d.environment(env)?;
    let components = d.components_on(env, only);
    if components.is_empty() {
        return Err(nothing_declared(env, only));
    }
    let machines = c.machines(env);
    let serial = match c.next_serial() {
        Some(n) => format!("as serial {n}"),
        None => "as the next serial".to_string(),
    };
    let mut checks = Vec::new();
    for component in components {
        let declared = d.declared_ref(env, &component)?;
        if !declared.contains("@sha256:") {
            checks.push(mutable_tag(env, &component, &declared));
            continue;
        }
        checks.push(row(
            &format!("{env}/{component}/plan"),
            true,
            format!("DRY: would publish {} {serial} and observe {} machine(s): {}", crate::verbs::short(&declared), machines.len(), machines.join(" ")),
        ));
    }
    let exit = if checks.iter().any(|c| c.fix.is_some()) { crate::EXIT_REFUSED } else { crate::EXIT_OK };
    let mut r = crate::Report::new("deploy", checks, exit);
    r.exit = exit;
    r.ok = exit == crate::EXIT_OK;
    Ok(r)
}

/// The default `status`: each machine's record against the declared ref, and
/// the machines list (gate D).
pub fn status_with<C: Converger + ?Sized>(c: &C, d: &dyn Declarations, env: &str, only: Option<&str>) -> Result<crate::Report> {
    d.environment(env)?;
    let mut checks = Vec::new();
    let mut graded = Vec::new();
    for component in d.components_on(env, only) {
        let declared = d.declared_ref(env, &component)?;
        // A sited fixture scopes each component to the class that carries it.
        let machines = d.scope(&component).unwrap_or_else(|| c.machines(env));
        for machine in machines {
            let name = format!("{env}/{machine}/{component}");
            let short = crate::verbs::short;
            let (check, state, reason) = match c.read_record(env, &machine, &component) {
                Ok(Some(r)) if r.reference == declared => (row(&name, true, format!("converged on {}", short(&declared))), "converged", "declared"),
                Ok(Some(r)) => (row(&name, false, format!("pending: record names {}, declared {}", short(&r.reference), short(&declared))), "pending", "other-ref"),
                Ok(None) => (row(&name, false, format!("pending: no record yet; declared {}", short(&declared))), "pending", "no-record"),
                Err(e) => (row(&name, false, format!("unknown: {e}")), "unknown", "unreadable"),
            };
            checks.push(check);
            graded.push(crate::Machine::new(env, &machine, &component, state, reason, &name));
        }
    }
    if checks.is_empty() {
        return Err(nothing_declared(env, only));
    }
    let mut r = crate::Report::new("status", checks, crate::EXIT_NOT_CONVERGED);
    r.machines = Some(graded);
    Ok(r)
}

/// The push converger on a real checkout (gate D): the push code behind the
/// trait. `publish` writes nothing (publishing is the merge: the deploy applies
/// what the checkout's pins declare); `observe_on` runs the deploy play and
/// reads every machine's outcome from that run; `read_record` reads one
/// machine's newest ledger row. Its `deploy`, `status` and `verify` are the
/// push code's own, called unchanged, so every report on real hosts is what it
/// was before the trait.
pub struct Push {
    repo: PathBuf,
    /// Who declared what `publish` accepted; the deploy records it (C4).
    actor: String,
}

impl Push {
    pub fn new(repo: &Path) -> Self {
        Push { repo: repo.to_path_buf(), actor: String::new() }
    }
}

/// The serial of the environment's signed intent (IntentV1,
/// `ops/consistency/intent/<goatnet>/intent.json`, whose signature `pins`
/// verifies). 0 when the environment has none: today's push deploy does not
/// require the intent, and the push converger does not start to. A pull
/// converger will require the signed index.
fn intent_serial(repo: &Path, env: &str) -> u64 {
    let p = repo.join("ops/consistency/intent").join(env).join("intent.json");
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("serial").and_then(|s| s.as_u64()))
        .unwrap_or(0)
}

impl Converger for Push {
    fn machines(&self, env: &str) -> Vec<String> {
        crate::inspect::machines(&self.repo, env).unwrap_or_default()
    }

    fn publish(&mut self, env: &str, component: &str, reference: &str, actor: &str) -> Result<u64> {
        let d = crate::push::declared(&self.repo, component, env, None)?;
        if !d.members.iter().any(|g| crate::push::want(&self.repo, g, &d.var).as_deref() == Some(reference)) {
            return Err(goat_errors::coded(
                "GB-046",
                format!("{component} on {env}: {reference} is not what the checkout's pins declare; the push converger deploys the checkout's declaration, so it is published by merging it"),
            )
            .into());
        }
        self.actor = actor.to_string();
        Ok(intent_serial(&self.repo, env))
    }

    fn observe_on(&mut self, env: &str, component: &str, _reference: &str, only: Option<&[String]>) -> Result<BTreeMap<String, Outcome>> {
        let o = crate::push::Options { component, goatnet: env, only_group: None, node: None, dry_run: false, verify_only: false, ack: &self.actor, ladder: Default::default() };
        let (_, graded) = crate::push::deploy_graded(&self.repo, &o)?;
        Ok(graded
            .into_iter()
            .filter(|m| only.map(|o| o.contains(&m.host)).unwrap_or(true))
            .map(|m| {
                let outcome = match m.state {
                    "converged" => Outcome::Converged,
                    "pending" => Outcome::Pending,
                    _ => Outcome::Unknown,
                };
                (m.host, outcome)
            })
            .collect())
    }

    fn read_record(&self, env: &str, machine: &str, component: &str) -> Result<Option<Record>> {
        crate::inspect::record(&self.repo, env, machine, component)
    }

    fn deploy(&mut self, _d: &mut dyn Declarations, req: &Converge) -> Result<crate::Report> {
        let component = req.component.ok_or_else(|| goat_errors::coded("GB-018", "deploy on real hosts takes one --component"))?;
        let o = crate::push::Options { component, goatnet: req.env, only_group: req.group, node: req.node, dry_run: req.dry_run, verify_only: req.verify_only, ack: req.actor, ladder: req.ladder };
        crate::push::deploy(&self.repo, &o)
    }

    fn status(&self, _d: &dyn Declarations, env: &str, only: Option<&str>) -> Result<crate::Report> {
        crate::inspect::status(&self.repo, env, only)
    }

    fn verify(&self, _d: &dyn Declarations, req: &Converge) -> Result<crate::Report> {
        let component = req.component.ok_or_else(|| goat_errors::coded("GB-018", "verify takes one component"))?;
        crate::inspect::verify(&self.repo, component, req.env, req.group, req.dry_run)
    }
}

/// The declarations a converger reads on real hosts when it uses the default
/// steps. The push converger reads the checkout's pins itself and never asks;
/// the pull converger brings the signed per-environment index.
pub struct NoDeclarations;

impl Declarations for NoDeclarations {
    fn environment(&self, _env: &str) -> Result<()> {
        Ok(())
    }
    fn components_on(&self, _env: &str, only: Option<&str>) -> Vec<String> {
        only.map(|c| vec![c.to_string()]).unwrap_or_default()
    }
    fn declared_ref(&self, env: &str, component: &str) -> Result<String> {
        Err(goat_errors::coded("GB-255", format!("nothing declared for {env} and component {component}: this converger reads no declarations on real hosts")).into())
    }
}

/// What a verb converges through: a fixture (`--mock`), or a converger and the
/// declarations it reads.
pub enum Target {
    Fixture(Box<crate::verbs::Fixture>),
    Converger { conv: Box<dyn Converger>, decl: Box<dyn Declarations> },
}

/// The one place a verb gets its converger (gate D): the fixture's scripted
/// converger under `--mock`, else the push converger. The pull converger, when
/// it is built, is chosen here from `converger:`; until then the push code
/// keeps refusing `converger: pull-agent` (GB-307) before it changes a host.
pub fn select(repo: &Path, mock: Option<&Path>) -> Result<Target> {
    Ok(match mock {
        Some(dir) => Target::Fixture(Box::new(crate::verbs::Fixture::open(dir)?)),
        None => Target::Converger { conv: Box::new(Push::new(repo)), decl: Box::new(NoDeclarations) },
    })
}

impl Target {
    pub fn deploy(&mut self, req: &Converge) -> Result<crate::Report> {
        match self {
            Target::Fixture(f) if req.dry_run => crate::verbs::deploy_plan(f, req.env, req.component),
            Target::Fixture(f) => crate::verbs::deploy(f, req.env, req.component, req.actor, req.frozen),
            Target::Converger { conv, decl } => conv.deploy(decl.as_mut(), req),
        }
    }

    pub fn status(&self, env: &str, only: Option<&str>) -> Result<crate::Report> {
        match self {
            Target::Fixture(f) => crate::verbs::status(f, env, only),
            Target::Converger { conv, decl } => conv.status(decl.as_ref(), env, only),
        }
    }

    pub fn verify(&self, req: &Converge) -> Result<crate::Report> {
        match self {
            Target::Fixture(f) => crate::inspect::verify_mock(f, req.component.unwrap_or(""), req.env),
            Target::Converger { conv, decl } => conv.verify(decl.as_ref(), req),
        }
    }
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
    fn before_publish(&self) -> Result<()> {
        self.publications().map(|_| ())
    }

    fn next_serial(&self) -> Option<u64> {
        Some(self.last_serial() + 1)
    }

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

    /// Mock records carry ISO 8601 UTC like every other record (the walkthrough
    /// saw `epoch+20758d43085s` beside `2026-10-20T10:00:00Z` in `history`).
    /// Gate D's property: a converger that implements only the four calls
    /// (`machines`, `publish`, `observe_on`, `read_record`) drives `deploy`,
    /// `status`, `verify` and a dry run through the same arm of [`Target`] the
    /// push converger takes on real hosts, so a second converger needs no verb change.
    struct Recording {
        calls: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        records: BTreeMap<String, String>,
        serial: u64,
    }

    impl Converger for Recording {
        fn machines(&self, _env: &str) -> Vec<String> {
            self.calls.borrow_mut().push("machines".into());
            vec!["m1".into(), "m2".into()]
        }
        fn publish(&mut self, _env: &str, _component: &str, reference: &str, _actor: &str) -> Result<u64> {
            self.calls.borrow_mut().push("publish".into());
            self.serial += 1;
            self.records.insert("m1".into(), reference.into());
            Ok(self.serial)
        }
        fn observe_on(&mut self, _env: &str, _component: &str, reference: &str, _only: Option<&[String]>) -> Result<BTreeMap<String, Outcome>> {
            self.calls.borrow_mut().push("observe_on".into());
            Ok(["m1", "m2"].iter().map(|m| (m.to_string(), if self.records.get(*m).map(String::as_str) == Some(reference) { Outcome::Converged } else { Outcome::Pending })).collect())
        }
        fn read_record(&self, _env: &str, machine: &str, component: &str) -> Result<Option<Record>> {
            self.calls.borrow_mut().push("read_record".into());
            Ok(self.records.get(machine).map(|r| Record { ts: String::new(), component: component.into(), reference: r.clone(), action: "deploy".into(), serial: 0, actor: String::new() }))
        }
    }

    struct OneComponent;

    impl Declarations for OneComponent {
        fn environment(&self, _env: &str) -> Result<()> {
            Ok(())
        }
        fn components_on(&self, _env: &str, _only: Option<&str>) -> Vec<String> {
            vec!["portal".into()]
        }
        fn declared_ref(&self, _env: &str, _component: &str) -> Result<String> {
            Ok("ghcr.io/o/portal:v2@sha256:aaaa".into())
        }
    }

    #[test]
    fn a_converger_with_only_the_four_calls_drives_every_converging_verb() {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let rec = Recording { calls: calls.clone(), records: BTreeMap::new(), serial: 0 };
        let mut t = Target::Converger { conv: Box::new(rec), decl: Box::new(OneComponent) };
        let req = Converge { env: "dev", component: Some("portal"), group: None, node: None, actor: "tester", dry_run: false, verify_only: false, frozen: false, ladder: Default::default() };

        let plan = t.deploy(&Converge { dry_run: true, ..req }).unwrap();
        assert!(plan.ok && plan.checks[0].detail.contains("would publish") && plan.checks[0].detail.contains("2 machine(s)"), "{:?}", plan.checks);

        let d = t.deploy(&req).unwrap();
        assert_eq!(d.verb, "deploy");
        let rows: Vec<(&str, bool)> = d.checks.iter().map(|c| (c.name.as_str(), c.ok)).collect();
        assert_eq!(rows, [("dev/portal/publish", true), ("dev/m1/portal", true), ("dev/m2/portal", false)]);
        assert_eq!(d.exit, crate::EXIT_NOT_CONVERGED);

        let s = t.status("dev", None).unwrap();
        let m: Vec<(&str, &str, &str)> = s.machines.as_ref().unwrap().iter().map(|m| (m.host.as_str(), m.state, m.reason)).collect();
        assert_eq!(m, [("m1", "converged", "declared"), ("m2", "pending", "no-record")]);
        let v = t.verify(&req).unwrap();
        assert_eq!(v.verb, "verify");
        assert_eq!(v.machines, s.machines);

        // Every step reached its answer through the four calls, and each was used.
        let used: std::collections::BTreeSet<String> = calls.borrow().iter().cloned().collect();
        assert_eq!(used, ["machines", "observe_on", "publish", "read_record"].iter().map(|s| s.to_string()).collect());
    }

    #[test]
    fn select_gives_the_fixture_s_converger_under_mock_and_push_on_real_hosts() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/compat/documents/fixtures/basic");
        assert!(matches!(select(std::path::Path::new("."), Some(&fixture)).unwrap(), Target::Fixture(_)));
        assert!(matches!(select(std::path::Path::new("."), None).unwrap(), Target::Converger { .. }));
    }

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

