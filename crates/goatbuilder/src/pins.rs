//! `pins`: ONE resolution pass with TWO renderings — the verb that replaces
//! ops/pins/generate-pins.py behind the parity gate (byte-identical pins.yml
//! and intent.json, goatbuilder-conformance.md §The parity gate).
//!
//!   1. `ops/ansible/group_vars/<member>/pins.yml` for every member host-group
//!      of every goatnet listed in ops/pins/fleet-pins.yaml (plus the lite
//!      sidecar ops/pins/lite-pins.yaml, ADR 1213 — merged at load).
//!   2. `ops/consistency/intent/<goatnet>/intent.json` — IntentV1, the typed
//!      intent artifact (docs/design/consistency-foundation.md §3, ADR 1110 D1).
//!
//! The resolution semantics are the Python package's
//! (ops/consistency/python/goat_consistency/: resolve.py, intent.py,
//! signing.py), ported function for function and held to the same outputs
//! by ops/goatbuilder/parity-pins.sh. Two precedences live here and nowhere
//! else in this binary:
//!
//!   pins  — member deviation -> goatnet deviation -> fleet value
//!   vars  — member group_vars -> group_vars/all.yml -> role default
//!
//! Modes: (default) rewrite every listed member's pins.yml and every listed
//! goatnet's intent.json; `--check` validate, regenerate, and refuse (exit 1)
//! on any difference, on an intent whose detached offline-CA signature is
//! missing or does not verify (CF-5), and on a governed pin defined outside
//! pins.yml. The unmigrated tail (ops/pins/unmigrated-pins.txt) may only
//! shrink; its baseline is declared in that file.
//!
//! Exit codes: 0 ok, 1 drift/violation, 2 usage or schema error — the
//! script's, which are also this binary's (C10).

use crate::runner::Cmd;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::{Check, Report, EXIT_REFUSED};

pub const GENERATED_BASENAME: &str = "pins.yml";
const INTENT_FORMAT: &str = "goat-intent-v1";
const TTL_SECONDS: i64 = 2_592_000;
const UNHASHED: [&str; 3] = ["serial", "generated_at", "content_hash"];
const HISTORY_DEPTH: usize = 10;
const ARTIFACT_TYPES: [&str; 7] = ["image", "config", "acl", "data", "cert", "forward-only", "release"];
/// The architectures a `release` component names a file for.
const RELEASE_ARCHES: [&str; 2] = ["amd64", "arm64"];
/// The keys of a `release:` declaration.
const RELEASE_KEYS: [&str; 5] = ["bundle", "checksums", "files", "play", "signer"];
/// The signature formats of `release.bundle`; the first is the default.
const RELEASE_BUNDLES: [&str; 3] = ["cosign", "sigstore", "attestation"];
const DEFAULT_ARTIFACT_TYPE: &str = "image";
const TRUTHY: [&str; 3] = ["true", "yes", "on"];
const FALSY: [&str; 3] = ["false", "no", "off"];

pub const SIGNER_HINT: &str = "re-sign with ops/enrollment/cmd/intent-sign (cd ops/enrollment && go run ./cmd/intent-sign --intent <path> --root-key <ca.key.age> --age-identity <keys.txt>) and commit intent.json and intent.json.sig together";

/// The code of an authoring surface that is not usable as one (the package's
/// SchemaError): fleet-pins.yaml, its sidecar, parity-capabilities.yaml, the
/// unmigrated-pins list or an intent breaks the pins schema or its rules. It
/// exits 2, as the uncoded error did.
pub const SCHEMA_REFUSED: &str = "GB-365";

/// Refuse a pins file that breaks the schema, with [`SCHEMA_REFUSED`].
fn die<T>(msg: String) -> Result<T> {
    Err(anyhow::Error::from(goat_errors::coded(SCHEMA_REFUSED, msg)))
}

// --- where things are (paths.py) ------------------------------------------------

pub struct Paths {
    pub repo: PathBuf,
}

impl Paths {
    pub fn new(repo: &Path) -> Self {
        Paths { repo: repo.to_path_buf() }
    }
    pub fn table(&self) -> PathBuf {
        self.repo.join("ops/pins/fleet-pins.yaml")
    }
    /// The lite sidecar (ADR 1213): goatnet rows + deviations only, merged at load.
    pub fn lite_table(&self) -> PathBuf {
        self.repo.join("ops/pins/lite-pins.yaml")
    }
    pub fn caps(&self) -> PathBuf {
        self.repo.join("ops/pins/parity-capabilities.yaml")
    }
    /// The capabilities file and the extensions a deployment supplies beside it
    /// (`ops/pins/parity-capabilities.d/*.yaml`), in merge order (resolve.py `caps_files`).
    pub fn caps_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.caps()];
        if let Ok(rd) = std::fs::read_dir(self.repo.join("ops/pins/parity-capabilities.d")) {
            let mut ext: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    !name.starts_with('.') && name.ends_with(".yaml")
                })
                .collect();
            ext.sort();
            files.extend(ext);
        }
        files
    }
    pub fn allowlist(&self) -> PathBuf {
        self.repo.join("ops/pins/unmigrated-pins.txt")
    }
    pub fn group_vars(&self) -> PathBuf {
        self.repo.join("ops/ansible/group_vars")
    }
    pub fn roles(&self) -> PathBuf {
        self.repo.join("ops/ansible/roles")
    }
    pub fn intent_schema(&self) -> PathBuf {
        self.repo.join("ops/consistency/schema/intent-v1.schema.json")
    }
    pub fn pubkey_dir(&self) -> PathBuf {
        self.repo.join("ops/enrollment/public-keys")
    }
    pub fn intent_path(&self, goatnet: &str) -> PathBuf {
        self.repo.join("ops/consistency/intent").join(goatnet).join("intent.json")
    }
    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.repo).unwrap_or(p).display().to_string()
    }
}

// --- YAML as the Python package sees it ----------------------------------------

/// A YAML document as a JSON value, with PyYAML's shapes where they differ from
/// serde_yaml's: a tagged node is its inner value, a non-string key is its
/// text. Bool-words (`yes`, `on`) stay strings — every consumer below
/// normalises them exactly as the package normalises the bool they would be.
pub fn yaml_to_json(v: &serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                n.as_f64().map(Value::from).unwrap_or(Value::Null)
            }
        }
        serde_yaml::Value::String(s) => Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => Value::Array(seq.iter().map(yaml_to_json).collect()),
        serde_yaml::Value::Mapping(m) => {
            let mut out = Map::new();
            for (k, val) in m {
                let key = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Null => "null".into(),
                    other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
                };
                out.insert(key, yaml_to_json(val));
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(&t.value),
    }
}

pub fn read_yaml(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    // The file was read; what it holds does not parse (GB-329, not GB-254).
    let v: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-329", format!("{}: {e}", path.display()))))?;
    let j = yaml_to_json(&v);
    Ok(if j.is_object() { j } else { Value::Object(Map::new()) })
}

/// Python truthiness for the values the package tests with `or` / `if`.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Python `str(value)` for a scalar YAML value.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True".into() } else { "False".into() },
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `" ".join(str(text or "").split())`.
pub fn one_line(v: Option<&Value>) -> String {
    let s = if truthy(v) { py_str(v.unwrap()) } else { String::new() };
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn obj(v: &Value) -> Option<&Map<String, Value>> {
    v.as_object()
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn s_of(v: Option<&Value>) -> String {
    v.map(py_str).unwrap_or_default()
}

// --- the pin table (resolve.py) ------------------------------------------------

/// The merged pin table: fleet-pins.yaml plus the lite sidecar beside it.
/// `lite` is the set of goatnets the sidecar declared — `build_body` needs it to
/// know which `sources` a goatnet's generation was rendered from.
pub struct Table {
    pub goatnets: Map<String, Value>,
    pub components: Map<String, Value>,
    pub lite: BTreeSet<String>,
}

pub const LITE_BASENAME: &str = "lite-pins.yaml";

/// The sidecar beside a fleet table — beside a scratch copy too, so a parity run
/// on a copied tree reads ITS copy, never the checkout's.
pub fn lite_table_for(path: &Path) -> PathBuf {
    path.parent().map(|d| d.join(LITE_BASENAME)).unwrap_or_else(|| PathBuf::from(LITE_BASENAME))
}

pub fn load_table(path: &Path) -> Result<Table> {
    let data = read_yaml(path)?;
    let goatnets = get(&data, "goatnets").and_then(obj).cloned();
    let components = get(&data, "components").and_then(obj).cloned();
    let mut goatnets = match goatnets {
        Some(g) => g,
        None => return die("fleet-pins.yaml: `goatnets:` must be a mapping".into()),
    };
    let mut components = match components {
        Some(c) if !c.is_empty() => c,
        _ => return die("fleet-pins.yaml: `components:` must be a non-empty mapping".into()),
    };
    for (cname, spec) in &components {
        if get(spec, OPTED_KEY).is_some() {
            return die(format!("component '{cname}': `{OPTED_KEY}:` is not a key a table may set"));
        }
    }
    let lite = load_lite(&lite_table_for(path), &mut goatnets, &mut components)?;
    apply_opt_in(&goatnets, &mut components)?;
    // A checkout whose only goatnet is a goatlite (an open-source export) declares
    // none in fleet-pins.yaml and one in the sidecar; the merged table must not be empty.
    if goatnets.is_empty() {
        return die("fleet-pins.yaml + lite-pins.yaml: no goatnet is declared in either table".into());
    }
    Ok(Table { goatnets, components, lite })
}

/// An opt-in component (`opt_in: true`) runs only on a goatnet that names it in
/// its own row's `opt_in:` list (resolve.py `apply_opt_in`, same words). Every
/// other goatnet does not resolve, deploy or verify it, and leaves it out of its
/// intent. The loader records the goatnets that opted in under this key, which
/// no table may write itself.
pub const OPTED_KEY: &str = "_opted_in";

pub fn apply_opt_in(goatnets: &Map<String, Value>, components: &mut Map<String, Value>) -> Result<()> {
    for (gn, gspec) in goatnets {
        let Some(listed) = get(gspec, "opt_in") else { continue };
        if listed.is_null() {
            continue;
        }
        let Some(names) = listed.as_array().filter(|a| a.iter().all(|v| v.is_string())) else {
            return die(format!("goatnet '{gn}': `opt_in:` must be a list of component names"));
        };
        for n in names {
            let cname = py_str(n);
            match components.get(&cname) {
                Some(spec) if spec.is_object() => {
                    if get(spec, "opt_in") != Some(&Value::Bool(true)) {
                        return die(format!("goatnet '{gn}': `opt_in:` names '{cname}', which is not an opt-in component (`opt_in: true`)"));
                    }
                }
                _ => return die(format!("goatnet '{gn}': `opt_in:` names '{cname}', which is not a component")),
            }
        }
    }
    for (cname, spec) in components.iter_mut() {
        if get(spec, "opt_in") != Some(&Value::Bool(true)) {
            continue;
        }
        let opted: Vec<Value> = goatnets
            .iter()
            .filter(|(_, g)| get(g, "opt_in").and_then(|l| l.as_array()).is_some_and(|a| a.iter().any(|v| v.as_str() == Some(cname.as_str()))))
            .map(|(gn, _)| Value::String(gn.clone()))
            .collect();
        if let Some(o) = spec.as_object_mut() {
            o.insert(OPTED_KEY.into(), Value::Array(opted));
        }
    }
    Ok(())
}

/// True when `spec` is an opt-in component that `goatnet` did not opt in to.
pub fn opted_out(spec: &Value, goatnet: &str) -> bool {
    get(spec, "opt_in") == Some(&Value::Bool(true))
        && !get(spec, OPTED_KEY).and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|v| v.as_str() == Some(goatnet)))
}

/// Read the lite sidecar and merge it into the fleet table IN PLACE (resolve.py
/// `load_lite`, same words). A lite goatnet (ADR 1213) is declared there and
/// never in fleet-pins.yaml, so rendering one moves no committed intent's
/// `sources` hash. The sidecar carries `goatnets:` rows and, under
/// `components:`, `deviations:` blocks for those goatnets or their members —
/// component DEFINITIONS live in fleet-pins.yaml and a sidecar carrying one is
/// refused. Goatnets union (a name in both is an error); deviations union per
/// component (a scope in both is an error). A missing sidecar is an empty one.
fn load_lite(path: &Path, goatnets: &mut Map<String, Value>, components: &mut Map<String, Value>) -> Result<BTreeSet<String>> {
    let mut declared = BTreeSet::new();
    if !path.is_file() {
        return Ok(declared);
    }
    let where_ = format!("ops/pins/{LITE_BASENAME}");
    let table = "ops/pins/fleet-pins.yaml";
    let data = read_yaml(path)?;
    let Some(top) = obj(&data) else {
        return die(format!("{where_}: must be a mapping with `goatnets:` and `components:`"));
    };
    let extra: Vec<&String> = top.keys().filter(|k| k.as_str() != "goatnets" && k.as_str() != "components").collect();
    if !extra.is_empty() {
        return die(format!("{where_}: unknown top-level key(s) {} — the sidecar carries `goatnets:` and `components:` (deviations only)", extra.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(", ")));
    }
    let lite_goatnets = match top.get("goatnets") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(g)) => g.clone(),
        Some(_) => return die(format!("{where_}: `goatnets:` must be a mapping")),
    };
    let lite_components = match top.get("components") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(c)) => c.clone(),
        Some(_) => return die(format!("{where_}: `components:` must be a mapping")),
    };
    let mut scopes: BTreeSet<String> = BTreeSet::new();
    for (gn, spec) in &lite_goatnets {
        if goatnets.contains_key(gn) {
            return die(format!("{where_}: goatnet '{gn}' is also declared in {table} — a goatnet lives in one table"));
        }
        goatnets.insert(gn.clone(), spec.clone());
        declared.insert(gn.clone());
        scopes.insert(gn.clone());
        scopes.extend(members_of(spec));
    }
    for (cname, spec) in &lite_components {
        let spec = match spec {
            Value::Null => Map::new(),
            Value::Object(o) => o.clone(),
            _ => return die(format!("{where_}: component '{cname}' must be a mapping")),
        };
        if !components.contains_key(cname) {
            return die(format!("{where_}: component '{cname}' is not declared in {table} — component definitions live in fleet-pins.yaml; the sidecar carries deviations only"));
        }
        let other: Vec<String> = spec.keys().filter(|k| k.as_str() != "deviations").map(|k| format!("`{k}:`")).collect();
        if !other.is_empty() {
            return die(format!("{where_}: component '{cname}' carries {} — component definitions live in fleet-pins.yaml; the sidecar carries `deviations:` only", other.join(", ")));
        }
        let devs = match spec.get("deviations") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(d)) => d.clone(),
            Some(_) => return die(format!("{where_}: component '{cname}': `deviations:` must be a mapping")),
        };
        let Some(target) = components.get_mut(cname).and_then(|c| c.as_object_mut()) else {
            return die(format!("{table}: component '{cname}' is not a mapping"));
        };
        if !matches!(target.get("deviations"), Some(Value::Object(_))) {
            target.insert("deviations".into(), Value::Object(Map::new()));
        }
        let target_devs = target.get_mut("deviations").and_then(|d| d.as_object_mut()).expect("deviations map just ensured");
        for (scope, dev) in devs {
            if !scopes.contains(&scope) {
                return die(format!("{where_}: component '{cname}': deviation scope '{scope}' is neither a goatnet declared in this sidecar nor one of its members"));
            }
            if target_devs.contains_key(&scope) {
                return die(format!("{where_}: component '{cname}': deviation '{scope}' is also declared in {table}"));
            }
            target_devs.insert(scope, dev);
        }
    }
    Ok(declared)
}

fn members_of(spec: &Value) -> Vec<String> {
    get(spec, "members").and_then(|m| m.as_array()).map(|a| a.iter().map(py_str).collect()).unwrap_or_default()
}

pub fn is_digest(reference: &str) -> bool {
    let re = regex::Regex::new(r"@sha256:[0-9a-f]{64}$").unwrap();
    re.is_match(reference)
}

fn check_ref(cname: &str, where_: &str, reference: &Value, allow_mutable: bool) -> Result<()> {
    let r = match reference.as_str() {
        Some(r) if !r.trim().is_empty() => r,
        _ => return die(format!("component '{cname}' {where_}: image ref must be a non-empty string")),
    };
    if is_digest(r) {
        return Ok(());
    }
    if !allow_mutable {
        return die(format!("component '{cname}' {where_}: '{r}' is a mutable/bare tag. Pins must carry a multi-arch INDEX digest (@sha256:…, F-245). A mutable tag is legal only inside a deviation stanza whose reason says why."));
    }
    Ok(())
}

/// A release pin: `<owner>/<repository>@<tag>@sha256:<digest>`, the digest
/// being that of the release's checksums file, which lists every file's own.
pub fn is_release_ref(reference: &str) -> bool {
    let re = regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9._-]+@[A-Za-z0-9][A-Za-z0-9._+-]*(?:/[A-Za-z0-9][A-Za-z0-9._+-]*)*@sha256:[0-9a-f]{64}$").unwrap();
    re.is_match(reference)
}

/// A release component's signer: the workflow that signs its releases, as
/// GitHub names it in a certificate's SAN, without the ref (the pin's tag).
pub fn is_release_signer(signer: &str) -> bool {
    let re = regex::Regex::new(r"^https://github\.com/[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9._-]+/\.github/workflows/[A-Za-z0-9._-]+\.ya?ml$").unwrap();
    re.is_match(signer)
}

/// A play's path inside an archive, plain and relative.
fn release_play_path(p: &str) -> bool {
    regex::Regex::new(r"\A[A-Za-z0-9_][A-Za-z0-9_./-]*\.ya?ml\z").unwrap().is_match(p) && !p.split('/').any(|x| ["", ".", ".."].contains(&x))
}

/// A file name of a release: one path segment of `[A-Za-z0-9._+-]`, the names
/// the `release-file` role accepts on the host, so a name it would refuse is
/// refused here, before anything is fetched.
fn release_file_name(v: &Value) -> bool {
    matches!(v.as_str(), Some(s) if !s.is_empty() && s != "." && s != ".." && !s.ends_with(".attestations.json") && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)))
}

/// The `<owner>/<repository>` a release signer names: the repository whose
/// workflow signs the release.
pub(crate) fn signer_repository(signer: &str) -> Option<String> {
    let rest = signer.strip_prefix("https://github.com/")?;
    let mut parts = rest.splitn(3, '/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

/// A release pin's repository must be its signer's repository, without
/// regard to case (GitHub's names are not case-sensitive).
pub(crate) fn pin_repository_is_signers(pin_repo: &str, signer: &str) -> bool {
    signer_repository(signer).is_some_and(|r| r.eq_ignore_ascii_case(pin_repo))
}

/// A `release` component (gate J23): its `release:` declaration and every pin
/// it carries. Mirrors goat_consistency.resolve.check_release word for word.
fn check_release(cname: &str, spec: &Value, fleet: &Value, devs: &Map<String, Value>) -> Result<()> {
    let rel = match get(spec, "release") {
        Some(Value::Object(o)) => o,
        _ => return die(format!("component '{cname}' is artifact_type 'release' but has no `release:` mapping (checksums, files, signer)")),
    };
    let mut unknown: Vec<&String> = rel.keys().filter(|k| !RELEASE_KEYS.contains(&k.as_str())).collect();
    unknown.sort();
    if let Some(k) = unknown.first() {
        return die(format!("component '{cname}': `release:` has an unknown key '{k}' (known: {})", RELEASE_KEYS.join(", ")));
    }
    if !rel.get("checksums").map(release_file_name).unwrap_or(false) {
        return die(format!("component '{cname}': `release.checksums:` must name the release's checksums file, one file name"));
    }
    if rel.get("bundle").is_some_and(|b| !b.as_str().is_some_and(|b| RELEASE_BUNDLES.contains(&b))) {
        return die(format!("component '{cname}': `release.bundle:` must be one of {} (the signature beside the checksums file)", RELEASE_BUNDLES.join(", ")));
    }
    if let Some(p) = rel.get("play") {
        if !p.as_str().is_some_and(release_play_path) {
            return die(format!("component '{cname}': `release.play:` is a play's path inside the release's archive"));
        }
        if ["deploy_playbook", "source"].iter().any(|k| get(spec, k).is_some()) {
            return die(format!("component '{cname}' declares `release.play` and deploy_playbook or source; its play is in one place"));
        }
    }
    let files = match rel.get("files") {
        Some(Value::Object(f)) if !f.is_empty() => f,
        _ => return die(format!("component '{cname}': `release.files:` must map each architecture ({}) to its file name", RELEASE_ARCHES.join(", "))),
    };
    let mut arches: Vec<&String> = files.keys().collect();
    arches.sort();
    for arch in arches {
        if !RELEASE_ARCHES.contains(&arch.as_str()) {
            return die(format!("component '{cname}': `release.files:` names architecture '{arch}' (known: {})", RELEASE_ARCHES.join(", ")));
        }
        if !release_file_name(&files[arch.as_str()]) {
            return die(format!("component '{cname}': `release.files.{arch}:` must be one file name"));
        }
    }
    if !is_release_signer(&s_of(rel.get("signer").filter(|v| v.is_string()))) {
        return die(format!("component '{cname}': `release.signer:` is the workflow that signs the release, https://github.com/<owner>/<repository>/.github/workflows/<file>.yml, with no ref: the pin's tag is its ref"));
    }
    let mut refs: Vec<(String, &Value)> = Vec::new();
    if !fleet.is_null() {
        refs.push(("fleet".into(), fleet));
    }
    for (scope, dev) in devs {
        if let Some(v) = get(dev, "value") {
            refs.push((format!("deviation '{scope}'"), v));
        }
    }
    for (where_, v) in refs {
        let r = v.as_str().unwrap_or_default();
        if !is_release_ref(r) {
            return die(format!("component '{cname}' {where_}: '{}' is not a release pin <owner>/<repository>@<tag>@sha256:<digest of its checksums file>", py_str(v)));
        }
        let repo = r.split_once('@').map(|(repo, _)| repo).unwrap_or_default();
        let signer = rel.get("signer").and_then(Value::as_str).unwrap_or_default();
        if !pin_repository_is_signers(repo, signer) {
            return die(format!("component '{cname}' {where_}: the pin's repository '{repo}' is not the repository of `release.signer:` ('{}'): a release pin's repository must be its signer's repository", signer_repository(signer).unwrap_or_default()));
        }
    }
    Ok(())
}

/// Schema + policy validation. Returns member -> goatnet (in the table's order).
pub fn validate(p: &Paths, t: &Table) -> Result<Vec<(String, String)>> {
    let mut member_of: Vec<(String, String)> = Vec::new();
    let claimed = |member_of: &Vec<(String, String)>, m: &str| member_of.iter().find(|(x, _)| x == m).map(|(_, g)| g.clone());
    for (gn, spec) in &t.goatnets {
        let members = members_of(spec);
        if members.is_empty() {
            return die(format!("goatnet '{gn}' declares no members"));
        }
        for m in members {
            if let Some(prev) = claimed(&member_of, &m) {
                return die(format!("member '{m}' is claimed by both '{prev}' and '{gn}'"));
            }
            if !p.group_vars().join(&m).is_dir() {
                return die(format!("goatnet '{gn}' lists member '{m}', but {}/{m}/ does not exist", p.rel(&p.group_vars())));
            }
            member_of.push((m, gn.clone()));
        }
    }
    for (name, _) in &member_of {
        if t.goatnets.contains_key(name) {
            return die(format!("name '{name}' is both a goatnet and a member host-group; deviation keys would be ambiguous"));
        }
    }
    let mut seen_vars: BTreeMap<String, String> = BTreeMap::new();
    for (cname, spec) in &t.components {
        let var = get(spec, "var");
        let var = match var {
            Some(v) if truthy(Some(v)) => py_str(v),
            _ => return die(format!("component '{cname}' has no `var:`")),
        };
        if let Some(prev) = seen_vars.get(&var) {
            return die(format!("var '{var}' is declared by both component '{prev}' and '{cname}'"));
        }
        seen_vars.insert(var, cname.clone());
        let fleet = match get(spec, "fleet") {
            Some(f) => f,
            None => return die(format!("component '{cname}' has no `fleet:` (use `fleet: null` when there is no value of record yet)")),
        };
        if !fleet.is_null() {
            check_ref(cname, "fleet", fleet, false)?;
        }
        let devs = match get(spec, "deviations") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(o)) => o.clone(),
            Some(_) => return die(format!("component '{cname}': `deviations:` must be a mapping")),
        };
        check_one_host(cname, spec)?;
        if let Some(o) = get(spec, "opt_in") {
            if !o.is_boolean() {
                return die(format!("component '{cname}': `opt_in:` must be true or false"));
            }
        }
        for (scope, dev) in &devs {
            let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
            let owner = if t.goatnets.contains_key(scope) { Some(scope.clone()) } else { claimed(&member_of, scope) };
            if let Some(owner) = owner {
                if opted_out(spec, &owner) && !truthy(get(&dev, "absent")) {
                    return die(format!("component '{cname}' is opt-in (`opt_in: true`), and '{scope}' sets a value for goatnet '{owner}', which does not list it under `opt_in:`. Add it to that goatnet's `opt_in:` list, or drop the deviation"));
                }
            }
            if !t.goatnets.contains_key(scope) && !member_of.iter().any(|(m, _)| m == scope) {
                return die(format!("component '{cname}': deviation scope '{scope}' is neither a listed goatnet nor a member host-group"));
            }
            if s_of(get(&dev, "reason").filter(|v| truthy(Some(v)))).trim().is_empty() {
                return die(format!("component '{cname}': deviation '{scope}' has no `reason:` — a deviation without a reason is a schema error (design doc §2 principle 2)"));
            }
            if truthy(get(&dev, "absent")) {
                if get(&dev, "value").is_some() {
                    return die(format!("component '{cname}': deviation '{scope}' sets both `absent:` and `value:`"));
                }
                continue;
            }
            match get(&dev, "value") {
                None => return die(format!("component '{cname}': deviation '{scope}' has neither `value:` nor `absent: true`")),
                Some(v) => check_ref(cname, &format!("deviation '{scope}'"), v, true)?,
            }
        }
        if let Some(watch) = get(spec, "render_watch") {
            if !watch.is_null() {
                let list = match watch.as_array() {
                    Some(l) if !l.is_empty() => l,
                    _ => return die(format!("component '{cname}': `render_watch:` must be a non-empty list of on-host file paths")),
                };
                for pth in list {
                    match pth.as_str() {
                        Some(s) if !s.trim().is_empty() => {}
                        _ => return die(format!("component '{cname}': `render_watch:` entries must be non-empty path strings")),
                    }
                }
                let set: BTreeSet<String> = list.iter().map(py_str).collect();
                if set.len() != list.len() {
                    return die(format!("component '{cname}': `render_watch:` lists a path twice"));
                }
                if matches!(get(spec, "ledger_component"), Some(Value::Null)) {
                    return die(format!("component '{cname}' declares `render_watch:` but `ledger_component: null` — the render manifest lives under /var/lib/goat/<ledger_component>/, so a component with no ledger home has nowhere to record its renders. Give it a deploying role (ADR 1060 coverage) first."));
                }
            }
        }
        let atype = artifact_type(spec);
        if !ARTIFACT_TYPES.contains(&atype.as_str()) {
            return die(format!("component '{cname}': artifact_type '{atype}' is not one of {} (ADR 1060's dispatch set — the artifact type decides whether undo means rollback, restore, rotate or refuse)", ARTIFACT_TYPES.join(", ")));
        }
        if let Some(why) = crate::source::problem(cname, spec) {
            return die(why);
        }
        if atype == "release" {
            check_release(cname, spec, fleet, &devs)?;
        } else if get(spec, "release").is_some() {
            return die(format!("component '{cname}' declares `release:` but its artifact_type is '{atype}'; only a release component names a release"));
        }
        if fleet.is_null() {
            for (gn, spec_gn) in &t.goatnets {
                if opted_out(spec, gn) {
                    continue;
                }
                for m in members_of(spec_gn) {
                    if !devs.contains_key(&m) && !devs.contains_key(gn) {
                        return die(format!("component '{cname}' has `fleet: null` but '{m}' (goatnet '{gn}') carries no deviation — the pin is unresolvable"));
                    }
                }
            }
        }
    }
    check_member_over(t, &member_of)?;
    Ok(member_of)
}

// --- ADR 1256: a member deviation names the goatnet value it overrides ----------
// `over:` must equal what the member's goatnet resolves the component to without
// the member deviation: a ref, `absent`, or `none` (`fleet: null`, no goatnet
// deviation). Mirrors goat_consistency.resolve.check_member_over word for word
// (ops/goatbuilder/parity-pins-member-over.sh holds the two equal).

const OVER_ABSENT: &str = "absent";
const OVER_NONE: &str = "none";

/// What `goatnet` resolves a component to with no member deviation, as the `over:` token.
pub fn goatnet_resolution(spec: &Value, goatnet: &str) -> String {
    if let Some(dev) = deviations(spec).get(goatnet) {
        let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
        if truthy(get(&dev, "absent")) {
            return OVER_ABSENT.into();
        }
        return py_str(get(&dev, "value").unwrap_or(&Value::Null));
    }
    match get(spec, "fleet") {
        None | Some(Value::Null) => OVER_NONE.into(),
        Some(f) => py_str(f),
    }
}

/// The line an operator pastes under the member deviation.
pub fn over_line(token: &str) -> String {
    if token == OVER_ABSENT || token == OVER_NONE {
        format!("over: {token}")
    } else {
        format!("over: \"{token}\"")
    }
}

fn refuse<T>(code: &str, msg: String) -> Result<T> {
    Err(anyhow::Error::from(goat_errors::coded(code, msg)))
}

/// GB-358: a member `value:` deviation must carry `over:`, and only it may.
/// GB-357: `over:` must name what the member's goatnet resolves to now.
pub fn check_member_over(t: &Table, member_of: &[(String, String)]) -> Result<()> {
    for (cname, spec) in &t.components {
        for (scope, dev) in &deviations(spec) {
            let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
            let over = get(&dev, "over");
            if t.goatnets.contains_key(scope) {
                if over.is_some() {
                    return refuse("GB-358", format!("GB-358 pins: component '{cname}': `over:` is on the goatnet deviation '{scope}'. Only a member deviation names what it overrides; a goatnet deviation overrides fleet. Delete the `over:` line."));
                }
                continue;
            }
            if truthy(get(&dev, "absent")) {
                if over.is_some() {
                    return refuse("GB-358", format!("GB-358 pins: component '{cname}': member deviation '{scope}' is `absent: true` and also carries `over:`. A member that does not run the component shadows nothing. Delete the `over:` line."));
                }
                continue;
            }
            let goatnet = match member_of.iter().find(|(m, _)| m == scope) {
                Some((_, g)) => g.clone(),
                None => continue,
            };
            let now = goatnet_resolution(spec, &goatnet);
            let value = py_str(get(&dev, "value").unwrap_or(&Value::Null));
            let line = over_line(&now);
            match over {
                None => {
                    return refuse("GB-358", format!("GB-358 pins: component '{cname}': member deviation '{scope}' pins {value} but does not name what it overrides. Goatnet '{goatnet}' resolves '{cname}' to {now} without it. To keep {value} on {scope} deliberately, add under deviations.{scope}:  {line}  Or delete the deviation, and {scope} takes {now}."));
                }
                Some(o) if py_str(o) != now => {
                    let was = py_str(o);
                    return refuse("GB-357", format!("GB-357 pins: component '{cname}': member deviation '{scope}' was written over {was}, but goatnet '{goatnet}' now resolves '{cname}' to {now}, so {scope} would silently keep {value}. Delete the deviation, and {scope} takes {now}. Or, to keep {value} on {scope} deliberately, set under deviations.{scope}:  {line}"));
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// Member value deviations that pin what their goatnet already resolves to: a
/// warning, not a refusal.
pub fn redundant_member_deviations(t: &Table, member_of: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut comps: Vec<(&String, &Value)> = t.components.iter().collect();
    comps.sort_by(|a, b| a.0.cmp(b.0));
    for (cname, spec) in comps {
        let devs = deviations(spec);
        let mut scopes: Vec<(&String, &Value)> = devs.iter().collect();
        scopes.sort_by(|a, b| a.0.cmp(b.0));
        for (scope, dev) in scopes {
            let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
            if t.goatnets.contains_key(scope) || truthy(get(&dev, "absent")) {
                continue;
            }
            let goatnet = match member_of.iter().find(|(m, _)| m == scope) {
                Some((_, g)) => g.clone(),
                None => continue,
            };
            let now = goatnet_resolution(spec, &goatnet);
            if py_str(get(&dev, "value").unwrap_or(&Value::Null)) == now {
                out.push(format!("WARN pins: component '{cname}': member deviation '{scope}' pins {now}, which goatnet '{goatnet}' already resolves to. It changes nothing today and would hold {scope} back from the goatnet's next value. Delete it."));
            }
        }
    }
    out
}

fn deviations(spec: &Value) -> Map<String, Value> {
    match get(spec, "deviations") {
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    }
}

/// `one_host_var:` names the group variable that names the one host of each
/// member group the component runs on (push::placement); resolve.py
/// check_one_host, same words.
pub fn check_one_host(cname: &str, spec: &Value) -> Result<()> {
    let Some(v) = get(spec, "one_host_var") else { return Ok(()) };
    if !v.as_str().is_some_and(|v| regex::Regex::new(r"^[a-z_][a-z0-9_]*$").unwrap().is_match(v)) {
        return die(format!("component '{cname}': `one_host_var:` is the name of a group variable that names one host, such as example_host"));
    }
    if truthy(get(spec, "single_homed_role")) {
        return die(format!("component '{cname}' declares both `one_host_var:` and `single_homed_role:`; a component runs on one host of each group by one of them"));
    }
    Ok(())
}

/// member deviation → goatnet deviation → fleet. Returns (value, deviation|None);
/// value None = declared absent (or `fleet: null`).
pub fn resolve(spec: &Value, member: &str, goatnet: &str) -> (Option<Value>, Option<Value>) {
    if opted_out(spec, goatnet) {
        return (None, None);
    }
    let devs = deviations(spec);
    for scope in [member, goatnet] {
        if let Some(dev) = devs.get(scope) {
            let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
            if truthy(get(&dev, "absent")) {
                return (None, Some(dev));
            }
            let v = get(&dev, "value").cloned().unwrap_or(Value::Null);
            return (Some(v), Some(dev));
        }
    }
    let fleet = get(spec, "fleet").cloned().unwrap_or(Value::Null);
    (if fleet.is_null() { None } else { Some(fleet) }, None)
}

pub fn deviation_scope(spec: &Value, member: &str, goatnet: &str) -> Option<String> {
    let devs = deviations(spec);
    for scope in [member, goatnet] {
        if devs.contains_key(scope) {
            return Some(scope.to_string());
        }
    }
    None
}

/// The vars the table governs for `goatnet`'s members (resolve.py
/// governed_vars_for): an opt-in component's var only where the goatnet opted in.
pub fn governed_vars_for(components: &Map<String, Value>, goatnet: &str) -> BTreeSet<String> {
    components.iter().filter(|(_, spec)| !opted_out(spec, goatnet)).filter_map(|(_, spec)| get(spec, "var").filter(|v| truthy(Some(v))).map(py_str)).collect()
}

pub fn artifact_type(spec: &Value) -> String {
    match get(spec, "artifact_type") {
        Some(v) if truthy(Some(v)) => py_str(v),
        _ => DEFAULT_ARTIFACT_TYPE.into(),
    }
}

// --- the group_vars tree ---------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Resolution {
    pub key: Option<String>,
    pub state: &'static str,
    pub source: Option<String>,
    pub detail: Option<String>,
}

fn sorted_yml(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "yml").unwrap_or(false) && p.is_file()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// key -> (value, source path). Ansible loads a group_vars dir alphabetically
/// and the LATER file wins on a key conflict; this mirrors that.
pub fn scope_vars(p: &Paths, directory: &Path) -> Result<BTreeMap<String, (Value, String)>> {
    let mut out = BTreeMap::new();
    for path in sorted_yml(directory) {
        if let Value::Object(m) = read_yaml(&path)? {
            for (k, v) in m {
                out.insert(k, (v, p.rel(&path)));
            }
        }
    }
    Ok(out)
}

/// key -> list of (role, value), for the last-resort resolution layer.
pub fn role_defaults(p: &Paths) -> Result<BTreeMap<String, Vec<(String, Value)>>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(p.roles())
        .map(|rd| rd.flatten().map(|e| e.path().join("defaults/main.yml")).filter(|f| f.is_file()).collect())
        .unwrap_or_default();
    paths.sort();
    let mut out: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    for path in paths {
        let role = path.parent().and_then(|d| d.parent()).and_then(|r| r.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Value::Object(m) = read_yaml(&path)? {
            for (k, v) in m {
                out.entry(k).or_default().push((role.clone(), v));
            }
        }
    }
    Ok(out)
}

pub fn all_scope_vars(p: &Paths) -> Result<BTreeMap<String, (Value, String)>> {
    let all = p.group_vars().join("all.yml");
    if !all.exists() {
        return Ok(BTreeMap::new());
    }
    let mut out = BTreeMap::new();
    if let Value::Object(m) = read_yaml(&all)? {
        for (k, v) in m {
            out.insert(k, (v, p.rel(&all)));
        }
    }
    Ok(out)
}

pub fn resolve_var_full(var: &str, member_scope: &BTreeMap<String, (Value, String)>, all_scope: &BTreeMap<String, (Value, String)>, defaults: &BTreeMap<String, Vec<(String, Value)>>) -> Resolution {
    if let Some((value, source)) = member_scope.get(var) {
        return render_value(value, source);
    }
    if let Some((value, _)) = all_scope.get(var) {
        return render_value(value, "group_vars/all.yml");
    }
    let hits = defaults.get(var).cloned().unwrap_or_default();
    if hits.len() == 1 {
        return render_value(&hits[0].1, &format!("role default: {}", hits[0].0));
    }
    if hits.len() > 1 {
        let mut roles: Vec<&str> = hits.iter().map(|(r, _)| r.as_str()).collect();
        roles.sort();
        return Resolution { key: None, state: "unreadable", source: None, detail: Some(format!("defined as a default by {} roles ({}) — no single default to attribute", hits.len(), roles.join(", "))) };
    }
    Resolution { key: None, state: "unset", source: None, detail: Some("no group_vars entry, no role default".into()) }
}

pub fn render_value(value: &Value, source: &str) -> Resolution {
    let resolved = |key: String| Resolution { key: Some(key), state: "resolved", source: Some(source.to_string()), detail: None };
    match value {
        Value::Bool(b) => resolved(b.to_string()),
        Value::String(s) => {
            if s.contains("{{") || s.contains("lookup(") {
                return Resolution { key: None, state: "unreadable", source: Some(source.to_string()), detail: Some("templated".into()) };
            }
            let low = s.trim().to_lowercase();
            if TRUTHY.contains(&low.as_str()) || FALSY.contains(&low.as_str()) {
                let norm = if TRUTHY.contains(&low.as_str()) { "true" } else { "false" };
                return resolved(norm.into());
            }
            resolved(s.clone())
        }
        Value::Null => resolved("null".into()),
        other => resolved(py_str(other)),
    }
}

// --- the curated capability half, and the ungoverned tail -------------------------

/// Rows keyed by these fields; an extension row with the same key replaces the
/// row in place, a new key is appended. Under `deviations`, a var an extension
/// declares carries the extension's scopes only (resolve.py `load_caps`).
const CAPS_ROW_KEYS: [(&str, &str); 4] = [("capabilities", "var"), ("node_shape", "var"), ("retired", "var"), ("cross_goatnet", "capability")];

pub fn load_caps(p: &Paths) -> Result<Value> {
    let mut data: Option<Value> = None;
    for path in p.caps_files() {
        let part = read_yaml(&path)?;
        check_caps(p, &path, &part)?;
        let Some(base) = data.as_mut() else {
            data = Some(part);
            continue;
        };
        let base = base.as_object_mut().expect("read_yaml returns a mapping");
        for (section, key) in CAPS_ROW_KEYS {
            let rows = match get(&part, section) {
                Some(Value::Array(a)) if !a.is_empty() => a.clone(),
                _ => continue,
            };
            let mut merged = match base.get(section) {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            for row in rows {
                let k = get(&row, key).cloned().unwrap_or(Value::Null);
                match merged.iter().position(|r| get(r, key).cloned().unwrap_or(Value::Null) == k) {
                    Some(i) => merged[i] = row,
                    None => merged.push(row),
                }
            }
            base.insert(section.to_string(), Value::Array(merged));
        }
        if let Some(Value::Object(devs)) = get(&part, "deviations") {
            let mut all = match base.get("deviations") {
                Some(Value::Object(o)) => o.clone(),
                _ => Map::new(),
            };
            // A var an extension declares carries the extension's scopes only.
            for (var, scopes) in devs {
                all.insert(var.clone(), scopes.clone());
            }
            base.insert("deviations".to_string(), Value::Object(all));
        }
    }
    Ok(data.unwrap_or_else(|| Value::Object(Map::new())))
}

fn check_caps(p: &Paths, path: &Path, data: &Value) -> Result<()> {
    for section in ["capabilities", "node_shape", "retired"] {
        let rows = match get(data, section) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return die(format!("{}: `{section}:` must be a list", p.rel(path))),
        };
        for row in rows {
            let row = if row.is_object() { row } else { Value::Object(Map::new()) };
            if !truthy(get(&row, "var")) {
                return die(format!("{}: a `{section}:` row has no `var:`", p.rel(path)));
            }
            if s_of(get(&row, "why").filter(|v| truthy(Some(v)))).trim().is_empty() {
                return die(format!("{}: `{section}` row '{}' has no `why:` — a tracked row must say why it earns one", p.rel(path), s_of(get(&row, "var"))));
            }
        }
    }
    let devs = match get(data, "deviations") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return die(format!("{}: `deviations:` must be a mapping", p.rel(path))),
    };
    for (var, scopes) in &devs {
        if let Some(scopes) = scopes.as_object() {
            for (scope, dev) in scopes {
                let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
                if s_of(get(&dev, "reason").filter(|v| truthy(Some(v)))).trim().is_empty() {
                    return die(format!("{}: deviation {var}/{scope} has no `reason:` — a deviation without a reason is a schema error (design doc §2 principle 2)", p.rel(path)));
                }
            }
        }
    }
    Ok(())
}

/// The shrink-only unmigrated-pin tail: (rows, count, declared baseline).
pub fn load_allowlist(p: &Paths) -> Result<(BTreeSet<String>, usize, Option<usize>)> {
    let path = p.allowlist();
    if !path.exists() {
        return Ok((BTreeSet::new(), 0, None));
    }
    let mut rows = BTreeSet::new();
    let mut baseline = None;
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
    for raw in text.lines() {
        if let Some(b) = raw.trim().strip_prefix("# baseline:") {
            if let Some(n) = b.split_whitespace().next().and_then(|x| x.parse::<usize>().ok()) {
                baseline = Some(n);
            }
        }
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if !line.contains(':') {
            return die(format!("{}: malformed row '{line}' (want <member_group>:<var>)", p.rel(&path)));
        }
        rows.insert(line.to_string());
    }
    let n = rows.len();
    Ok((rows, n, baseline))
}

/// Governed-shaped pin keys defined outside pins.yml in a listed member:
/// (member, key, source path), in the package's scan order.
pub fn scan_outside_pins(p: &Paths, member_of: &[(String, String)]) -> Result<Vec<(String, String, String)>> {
    let re = regex::Regex::new(r"(_image|_release_tag)$").unwrap();
    let mut members: Vec<&String> = member_of.iter().map(|(m, _)| m).collect();
    members.sort();
    let mut found = Vec::new();
    for member in members {
        for path in sorted_yml(&p.group_vars().join(member)) {
            if path.file_name().map(|n| n == GENERATED_BASENAME).unwrap_or(false) {
                continue;
            }
            if let Value::Object(m) = read_yaml(&path)? {
                for k in m.keys() {
                    if re.is_match(k) {
                        found.push((member.clone(), k.clone(), p.rel(&path)));
                    }
                }
            }
        }
    }
    Ok(found)
}

// --- rendering 1: pins.yml -----------------------------------------------------------

const HEADER: &str = "---
# {member} image pins. GENERATED FILE — do not hand-edit; regenerate via
# ops/pins/generate-pins.py.
#
# Rendered from ops/pins/fleet-pins.yaml, which is the ONE home for every
# governed `*_image` / release-tag pin of the `{goatnet}` goatnet. Rationale,
# rollback targets and deviation reasons live there, next to the value.
# Canon: docs/design/environment-parity.md §3.
#
# To bump an image: edit fleet-pins.yaml, rerun the generator, commit both. The
# `fleet-pins` CI job refuses drift between the table and this file, and refuses
# a governed pin defined anywhere else in this goatnet's group_vars.
#
# WHY the copies are generated rather than hand-kept:
{members_block}
#
# PRECEDENCE HAZARD (the reason a hand pin must not come back): within one
# group_vars directory ansible loads files alphabetically and the later file
# WINS on a key conflict. `vars.yml` sorts after `pins.yml`, so a pin re-added
# to vars.yml would silently shadow this file. That is what the CI job's
# outside-pins.yml scan exists to catch.

";

pub fn wrap_comment(text: &str) -> Vec<String> {
    let (width, indent) = (76usize, "# ");
    let mut out = Vec::new();
    let mut line = indent.to_string();
    for word in text.split_whitespace() {
        if line.chars().count() + word.chars().count() + 1 > width && line.trim() != indent.trim() {
            out.push(line.trim_end().to_string());
            line = format!("{indent}{word} ");
        } else {
            line.push_str(word);
            line.push(' ');
        }
    }
    if line.trim() != indent.trim() {
        out.push(line.trim_end().to_string());
    }
    out
}

pub fn render_pins(member: &str, goatnet: &str, t: &Table) -> String {
    let members = members_of(&t.goatnets[goatnet]);
    let peers: Vec<&String> = members.iter().filter(|m| m.as_str() != member).collect();
    let members_sentence = if !peers.is_empty() {
        let mut all = vec![member.to_string()];
        all.extend(peers.iter().map(|s| s.to_string()));
        format!("the `{goatnet}` goatnet's mgmt nodes are one fabric (design doc §7 node-parity rule). {} render from the same table row, so bumping one and missing the other is structurally impossible.", all.join(" and "))
    } else {
        format!("the `{goatnet}` goatnet has one member today; the table row is still the single home so a second node inherits the pins by existing.")
    };
    let body = HEADER.replace("{member}", member).replace("{goatnet}", goatnet).replace("{members_block}", &wrap_comment(&members_sentence).join("\n"));

    let mut lines: Vec<String> = vec![
        "# --- consistency: which generation governs this node (CF-3) ---".into(),
        format!("consistency_goatnet: \"{goatnet}\""),
        format!("consistency_member_group: \"{member}\""),
        String::new(),
    ];
    let mut watch: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cname, spec) in &t.components {
        let paths: Vec<String> = match get(spec, "render_watch") {
            Some(Value::Array(a)) => a.iter().map(py_str).collect(),
            _ => Vec::new(),
        };
        if paths.is_empty() {
            continue;
        }
        let (value, _dev) = resolve(spec, member, goatnet);
        if value.is_none() {
            continue;
        }
        let key = match get(spec, "ledger_component") {
            Some(v) if truthy(Some(v)) => py_str(v),
            _ => cname.clone(),
        };
        watch.insert(key, paths);
    }
    if !watch.is_empty() {
        lines.push("# --- consistency: declared config render watch-sets (CF-4) ---".into());
        lines.push("# Consumed by rollback-ledger/tasks/render-manifest.yml (default-off:".into());
        lines.push("# consistency_render_manifest_enabled). Declared in fleet-pins.yaml;".into());
        lines.push("# see docs/design/consistency-foundation.md §5.2a.".into());
        lines.push("consistency_render_watch:".into());
        for (key, paths) in &watch {
            lines.push(format!("  \"{key}\":"));
            for pth in paths {
                lines.push(format!("    - \"{pth}\""));
            }
        }
        lines.push(String::new());
    }
    for (cname, spec) in &t.components {
        if opted_out(spec, goatnet) {
            continue; // an opt-in component this goatnet did not opt in to: not named
        }
        let (value, dev) = resolve(spec, member, goatnet);
        let reason = |d: &Value| s_of(get(d, "reason")).trim().to_string();
        match value {
            None => {
                lines.push(format!("# {cname} — NOT DEPLOYED here (declared `absent` in fleet-pins.yaml)."));
                let d = dev.unwrap_or(Value::Object(Map::new()));
                lines.extend(wrap_comment(&format!("Reason: {}", reason(&d))));
                lines.push(String::new());
                continue;
            }
            Some(v) => {
                lines.push(format!("# {cname}"));
                if let Some(d) = &dev {
                    lines.extend(wrap_comment(&format!("DEVIATION from the fleet value. Reason: {}", reason(d))));
                    if truthy(get(d, "until")) {
                        lines.push(format!("# until: {}", s_of(get(d, "until"))));
                    }
                }
                if truthy(get(spec, "note")) {
                    lines.extend(wrap_comment(s_of(get(spec, "note")).trim()));
                }
                lines.push(format!("{}: \"{}\"", s_of(get(spec, "var")), py_str(&v)));
                lines.push(String::new());
            }
        }
    }
    format!("{body}{}\n", lines.join("\n").trim_end_matches('\n'))
}

// --- rendering 2: IntentV1 (intent.py) ---------------------------------------------

/// THE serialization contract: keys sorted, two-space indent, UTF-8, one
/// trailing newline — CF-5 signs SHA-256 over these exact bytes.
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, 0, &mut out);
    out.push('\n');
    out
}

fn write_canonical(v: &Value, depth: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match v {
        Value::Object(m) if !m.is_empty() => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (i, k) in keys.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push_str(": ");
                write_canonical(&m[*k], depth + 1, out);
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
        Value::Object(_) => out.push_str("{}"),
        Value::Array(a) if !a.is_empty() => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                write_canonical(x, depth + 1, out);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        Value::Array(_) => out.push_str("[]"),
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

pub fn sha256_of(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    format!("sha256:{:x}", h.finalize())
}

pub fn sha256_of_file(path: &Path) -> Result<String> {
    Ok(sha256_of(&std::fs::read(path).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?))
}

pub fn content_hash(body: &Map<String, Value>) -> String {
    let trimmed: Map<String, Value> = body.iter().filter(|(k, _)| !UNHASHED.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    sha256_of(canonical_json(&Value::Object(trimmed)).as_bytes())
}

fn declared_block(scope: &str, dev: &Value) -> Value {
    let mut out = Map::new();
    out.insert("scope".into(), Value::String(scope.into()));
    out.insert("reason".into(), Value::String(one_line(get(dev, "reason"))));
    if truthy(get(dev, "until")) {
        out.insert("until".into(), Value::String(s_of(get(dev, "until"))));
    }
    Value::Object(out)
}

fn var_cell(r: &Resolution) -> Value {
    let mut cell = Map::new();
    match r.state {
        "resolved" => {
            cell.insert("state".into(), json!("resolved"));
            cell.insert("value".into(), r.key.clone().map(Value::String).unwrap_or(Value::Null));
            cell.insert("source".into(), r.source.clone().map(Value::String).unwrap_or(Value::Null));
        }
        "unreadable" => {
            cell.insert("state".into(), json!("unreadable"));
            cell.insert("detail".into(), r.detail.clone().map(Value::String).unwrap_or(Value::Null));
            if let Some(s) = &r.source {
                if !s.is_empty() {
                    cell.insert("source".into(), Value::String(s.clone()));
                }
            }
        }
        _ => {
            cell.insert("state".into(), json!("unset"));
            cell.insert("detail".into(), r.detail.clone().map(Value::String).unwrap_or(Value::Null));
        }
    }
    Value::Object(cell)
}

struct Tree {
    all_scope: BTreeMap<String, (Value, String)>,
    defaults: BTreeMap<String, Vec<(String, Value)>>,
    scopes: BTreeMap<String, BTreeMap<String, (Value, String)>>,
}

fn var_row(row: &Value, members: &[String], goatnet: &str, tree: &Tree, cap_devs: &Map<String, Value>) -> Value {
    let var = s_of(get(row, "var"));
    let mut out = Map::new();
    out.insert("var".into(), Value::String(var.clone()));
    out.insert("why".into(), Value::String(one_line(get(row, "why"))));
    let mut cells = Map::new();
    for m in members {
        cells.insert(m.clone(), var_cell(&resolve_var_full(&var, &tree.scopes[m], &tree.all_scope, &tree.defaults)));
    }
    out.insert("members".into(), Value::Object(cells));
    let mut declared = Map::new();
    if let Some(Value::Object(scopes)) = cap_devs.get(&var) {
        for (scope, dev) in scopes {
            if members.contains(scope) || scope == goatnet {
                let dev = if dev.is_object() { dev.clone() } else { Value::Object(Map::new()) };
                declared.insert(scope.clone(), declared_block(scope, &dev));
            }
        }
    }
    if !declared.is_empty() {
        out.insert("declared".into(), Value::Object(declared));
    }
    Value::Object(out)
}

fn resolve_ca_id(spec: &Value, members: &[String], tree: &Tree) -> Value {
    let var = match get(spec, "ca_id_var") {
        Some(v) if truthy(Some(v)) => py_str(v),
        _ => return Value::Null,
    };
    let mut values: BTreeSet<String> = BTreeSet::new();
    for m in members {
        let r = resolve_var_full(&var, &tree.scopes[m], &tree.all_scope, &tree.defaults);
        let key = match &r.key {
            Some(k) if r.state == "resolved" && !k.is_empty() => k.clone(),
            _ => continue,
        };
        if r.source.as_deref().unwrap_or("").starts_with("role default:") {
            continue;
        }
        values.insert(key);
    }
    if values.len() == 1 {
        Value::String(values.into_iter().next().unwrap())
    } else {
        Value::Null
    }
}

/// Everything except serial / generated_at / content_hash — exactly the part that is hashed.
#[allow(clippy::too_many_arguments)]
pub fn build_body(p: &Paths, goatnet: &str, t: &Table, caps: &Value, member_of: &[(String, String)], allow: &BTreeSet<String>, outside: &[(String, String, String)]) -> Result<Map<String, Value>> {
    let spec = &t.goatnets[goatnet];
    let members = members_of(spec);
    let mut scopes = BTreeMap::new();
    for m in &members {
        scopes.insert(m.clone(), scope_vars(p, &p.group_vars().join(m))?);
    }
    let tree = Tree { all_scope: all_scope_vars(p)?, defaults: role_defaults(p)?, scopes };
    let cap_devs = match get(caps, "deviations") {
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    };

    let mut comps = Vec::new();
    for (cname, cspec) in &t.components {
        // An opt-in component this goatnet did not opt in to is not in its intent.
        if opted_out(cspec, goatnet) {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("name".into(), Value::String(cname.clone()));
        entry.insert("var".into(), get(cspec, "var").cloned().unwrap_or(Value::Null));
        entry.insert("artifact_type".into(), Value::String(artifact_type(cspec)));
        entry.insert("members".into(), Value::Object(Map::new()));
        if let Some(l) = get(cspec, "ledger_component") {
            entry.insert("ledger_component".into(), l.clone());
        }
        if truthy(get(cspec, "render_watch")) {
            entry.insert("render_watch".into(), get(cspec, "render_watch").cloned().unwrap_or(Value::Null));
        }
        if truthy(get(cspec, "note")) {
            entry.insert("note".into(), Value::String(one_line(get(cspec, "note"))));
        }
        let mut cells = Map::new();
        for m in &members {
            let (value, dev) = resolve(cspec, m, goatnet);
            let scope = deviation_scope(cspec, m, goatnet).unwrap_or_default();
            match value {
                None => {
                    let mut c = Map::new();
                    c.insert("state".into(), json!("absent"));
                    c.insert("declared".into(), declared_block(&scope, &dev.unwrap_or(Value::Object(Map::new()))));
                    cells.insert(m.clone(), Value::Object(c));
                }
                Some(v) => {
                    let mut c = Map::new();
                    c.insert("state".into(), json!("resolved"));
                    c.insert("value".into(), v);
                    if let Some(d) = dev {
                        c.insert("declared".into(), declared_block(&scope, &d));
                    }
                    cells.insert(m.clone(), Value::Object(c));
                }
            }
        }
        entry.insert("members".into(), Value::Object(cells));
        comps.push(Value::Object(entry));
    }

    let mut retired = Vec::new();
    if let Some(Value::Array(rows)) = get(caps, "retired") {
        for row in rows {
            let var = s_of(get(row, "var"));
            let mut present: Vec<Value> = members.iter().filter(|m| tree.scopes[*m].contains_key(&var)).map(|m| Value::String(m.clone())).collect();
            if tree.all_scope.contains_key(&var) {
                present.push(json!("group_vars/all.yml"));
            }
            retired.push(json!({"var": var, "why": one_line(get(row, "why")), "present_in": present}));
        }
    }

    let mut sorted_outside: Vec<&(String, String, String)> = outside.iter().collect();
    sorted_outside.sort();
    let ungoverned: Vec<Value> = sorted_outside
        .iter()
        .filter(|(m, _, _)| member_of.iter().any(|(mm, g)| mm == m && g == goatnet))
        .map(|(m, k, path)| json!({"member": m, "var": k, "source": path, "allowlisted": allow.contains(&format!("{m}:{k}"))}))
        .collect();

    let rows = |section: &str| -> Vec<Value> {
        match get(caps, section) {
            Some(Value::Array(a)) => a.iter().map(|r| var_row(r, &members, goatnet, &tree, &cap_devs)).collect(),
            _ => Vec::new(),
        }
    };

    // The input files this generation was rendered from (intent.py `sources_of`).
    // A goatnet declared in fleet-pins.yaml: the fleet table and the capabilities
    // file — unchanged since CF-1, so a lite render beside it moves no committed
    // intent. A goatnet declared in the lite sidecar (ADR 1213): the fleet table,
    // the sidecar and the capabilities file.
    let mut sources = Map::new();
    sources.insert(p.rel(&p.table()), Value::String(sha256_of_file(&p.table())?));
    if t.lite.contains(goatnet) {
        sources.insert(p.rel(&p.lite_table()), Value::String(sha256_of_file(&p.lite_table())?));
    }
    for caps_file in p.caps_files() {
        sources.insert(p.rel(&caps_file), Value::String(sha256_of_file(&caps_file)?));
    }

    let mut body = Map::new();
    body.insert("format".into(), json!(INTENT_FORMAT));
    body.insert("goatnet".into(), json!(goatnet));
    body.insert("members".into(), Value::Array(members.iter().map(|m| Value::String(m.clone())).collect()));
    body.insert("ttl_seconds".into(), json!(TTL_SECONDS));
    body.insert("ca_id".into(), resolve_ca_id(spec, &members, &tree));
    body.insert("sources".into(), Value::Object(sources));
    body.insert("components".into(), Value::Array(comps));
    body.insert("capabilities".into(), Value::Array(rows("capabilities")));
    body.insert("node_shape".into(), Value::Array(rows("node_shape")));
    body.insert("retired".into(), Value::Array(retired));
    body.insert("ungoverned".into(), Value::Array(ungoverned));
    Ok(body)
}

/// The committed artifact, or {} when there isn't a usable one.
pub fn read_committed(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn has_identity(doc: &Map<String, Value>) -> bool {
    doc.get("serial").and_then(|s| s.as_i64()).map(|s| s >= 1).unwrap_or(false)
        && doc.get("content_hash").and_then(|c| c.as_str()).map(|c| !c.is_empty()).unwrap_or(false)
        && doc.get("generated_at").and_then(|c| c.as_str()).map(|c| !c.is_empty()).unwrap_or(false)
}

fn identity(doc: &Map<String, Value>) -> Value {
    json!({"serial": doc["serial"], "content_hash": doc["content_hash"], "generated_at": doc["generated_at"]})
}

fn history_of(doc: &Map<String, Value>) -> Vec<Value> {
    doc.get("history").and_then(|h| h.as_array()).map(|a| a.iter().filter(|x| x.is_object()).cloned().collect()).unwrap_or_default()
}

fn succeed(committed: &Map<String, Value>) -> Vec<Value> {
    let mut tail = history_of(committed);
    if has_identity(committed) {
        tail.insert(0, identity(committed));
    }
    tail.truncate(HISTORY_DEPTH);
    tail
}

/// Attach the history tail and the generation identity. `serial` and
/// `generated_at` move ONLY when the body changes.
pub fn render_intent(body: &Map<String, Value>, committed: &Map<String, Value>, now: Option<&str>) -> (String, Map<String, Value>) {
    let mut keep = body.clone();
    keep.insert("history".into(), Value::Array(history_of(committed)));
    let mut digest = content_hash(&keep);
    let serial = committed.get("serial").and_then(|s| s.as_i64()).filter(|s| *s >= 1);
    let generated_at = committed.get("generated_at").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(String::from);
    let unchanged = committed.get("content_hash").and_then(|c| c.as_str()) == Some(digest.as_str()) && serial.is_some() && generated_at.is_some();
    let (mut doc, serial, generated_at) = if unchanged {
        (keep, serial.unwrap(), generated_at.unwrap())
    } else {
        let mut doc = body.clone();
        doc.insert("history".into(), Value::Array(succeed(committed)));
        digest = content_hash(&doc);
        let serial = serial.map(|s| s + 1).unwrap_or(1);
        let stamp = now.map(String::from).unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
        (doc, serial, stamp)
    };
    doc.insert("serial".into(), json!(serial));
    doc.insert("generated_at".into(), Value::String(generated_at));
    doc.insert("content_hash".into(), Value::String(digest.clone()));
    (canonical_json(&Value::Object(doc.clone())), doc)
}

pub fn self_consistent(doc: &Map<String, Value>) -> (String, String) {
    (doc.get("content_hash").map(py_str).unwrap_or_else(|| "None".into()), content_hash(doc))
}

pub fn validate_schema(p: &Paths, doc: &Map<String, Value>, where_: &str) -> Result<()> {
    let schema_path = p.intent_schema();
    let text = std::fs::read_to_string(&schema_path).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", schema_path.display()))))?;
    // Read, but not JSON, or not a draft 7 schema: the checkout's file is damaged (GB-329).
    let schema: Value = serde_json::from_str(&text).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-329", format!("{} is not JSON: {e}", p.rel(&schema_path)))))?;
    let compiled = jsonschema::JSONSchema::options().with_draft(jsonschema::Draft::Draft7).compile(&schema).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-329", format!("{} does not compile as a draft 7 schema: {e}", p.rel(&schema_path)))))?;
    let value = Value::Object(doc.clone());
    if let Err(errors) = compiled.validate(&value) {
        if let Some(e) = errors.into_iter().next() {
            let loc = e.instance_path.to_string();
            let loc = if loc.is_empty() { "(document root)".to_string() } else { loc.trim_start_matches('/').to_string() };
            return die(format!("{where_} fails {} at {loc}: {e}", p.rel(&schema_path)));
        }
    }
    Ok(())
}

// --- verify-at-read for the detached signature (signing.py) --------------------------

pub struct Verification {
    pub state: &'static str,
    pub detail: String,
    /// The openssl command that was started, as a dry run prints it (C11).
    pub ran: Option<String>,
}

fn clean_spki_pem(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut body = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "-----BEGIN PUBLIC KEY-----" {
            inside = true;
        } else if line == "-----END PUBLIC KEY-----" {
            break;
        } else if inside && !line.is_empty() && !line.contains(':') {
            body.push(line.to_string());
        }
    }
    if body.is_empty() {
        return None;
    }
    Some(format!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n", body.join("\n")))
}

pub fn verify_file(p: &Paths, path: &Path, doc: &Map<String, Value>) -> Verification {
    use base64::Engine;
    let failed = |d: String| Verification { state: "failed", detail: d, ran: None };
    let raw = match std::fs::read(path) {
        Ok(r) => r,
        Err(e) => return failed(format!("cannot read {}: {e}", path.display())),
    };
    let sig_path = PathBuf::from(format!("{}.sig", path.display()));
    if !sig_path.exists() {
        return failed(format!("{} has no detached signature at {} — an unsigned intent is refused, never graded ({SIGNER_HINT})", p.rel(path), p.rel(&sig_path)));
    }
    let ca_id = doc.get("ca_id").and_then(|c| c.as_str()).unwrap_or("");
    if ca_id.is_empty() || ca_id.contains('/') {
        return failed(format!("{} declares no usable ca_id — the verification key is resolved from the committed SPKI set by CA id, so an artifact naming none (or a path-shaped one) cannot be verified", p.rel(path)));
    }
    let pubkey_path = p.pubkey_dir().join(format!("{ca_id}.pem"));
    if !pubkey_path.exists() {
        return failed(format!("{} declares ca_id={ca_id} but {} is not committed — the key is never taken on the artifact's word alone", p.rel(path), p.rel(&pubkey_path)));
    }
    let pem = match clean_spki_pem(&pubkey_path) {
        Some(x) => x,
        None => return failed(format!("{} carries no PUBLIC KEY block", p.rel(&pubkey_path))),
    };
    let openssl = std::env::var("PATH").ok().and_then(|pth| std::env::split_paths(&pth).map(|d| d.join("openssl")).find(|f| f.is_file()));
    let openssl = match openssl {
        Some(o) => o,
        None => return Verification { state: "unavailable", detail: format!("cannot verify {}: openssl is not on PATH in this context. Refusing to treat an unverifiable intent as verified (fail closed)", p.rel(path)), ran: None },
    };
    let sig_der = match std::fs::read_to_string(&sig_path).map_err(|e| e.to_string()).and_then(|t| base64::engine::general_purpose::STANDARD.decode(t.trim()).map_err(|e| e.to_string())) {
        Ok(b) => b,
        Err(e) => return failed(format!("{} is not base64: {e}", p.rel(&sig_path))),
    };
    let tmp = std::env::temp_dir().join(format!("intent-verify-{}-{}", std::process::id(), ca_id));
    let _ = std::fs::create_dir_all(&tmp);
    let (tp, ts, tb) = (tmp.join("spki.pem"), tmp.join("sig.der"), tmp.join("body"));
    let written = std::fs::write(&tp, pem).and(std::fs::write(&ts, &sig_der)).and(std::fs::write(&tb, &raw));
    // Run in the scratch directory, by file name: the line is the same each run.
    let mut cmd = Cmd::new(&openssl);
    cmd.current_dir(&tmp).args(["dgst", "-sha256", "-verify", "spki.pem", "-signature", "sig.der", "body"]);
    let ran = written.is_ok().then(|| cmd.line());
    let result = match written {
        Err(e) => Verification { state: "unavailable", detail: format!("cannot verify {}: openssl did not run ({e}). Refusing to treat an unverifiable intent as verified (fail closed)", p.rel(path)), ran: None },
        Ok(()) => match cmd.output() {
            Err(e) => Verification { state: "unavailable", detail: format!("cannot verify {}: openssl did not run ({e}). Refusing to treat an unverifiable intent as verified (fail closed)", p.rel(path)), ran: None },
            Ok(o) if !o.status.success() => failed(format!("{} FAILED signature verification against {} — the file changed after signing, or the signature is not this artifact's. Refusing to grade against an intent nobody signed ({SIGNER_HINT})", p.rel(path), p.rel(&pubkey_path))),
            Ok(_) => Verification { state: "verified", detail: format!("signature valid (ca {ca_id}, {})", p.rel(&pubkey_path)), ran: None },
        },
    };
    let _ = std::fs::remove_dir_all(&tmp);
    Verification { ran, ..result }
}

// --- the verb ------------------------------------------------------------------------

/// The one resolution pass, as data: every member's rendered pins.yml and every
/// goatnet's rendered intent, for callers (release, fastlane) that regenerate.
pub struct Rendered {
    pub pins: Vec<(String, PathBuf, String)>,
    pub intents: Vec<RenderedIntent>,
}

/// (goatnet, path, text, document, changed-against-the-committed-file)
pub type RenderedIntent = (String, PathBuf, String, Map<String, Value>, bool);

fn unified_diff(current: &str, text: &str, from: &str, to: &str) -> Vec<String> {
    let diff = similar::TextDiff::from_lines(current, text);
    let out = diff.unified_diff().context_radius(3).header(from, to).to_string();
    out.lines().map(|l| l.to_string()).collect()
}

pub fn run(repo: &Path, check: bool) -> Result<Report> {
    let p = Paths::new(repo);
    let mut rows: Vec<Check> = Vec::new();
    let mut failed = false;
    let fail = |rows: &mut Vec<Check>, s: String| rows.push(Check { name: "pins".into(), ok: false, detail: s, fix: None });
    let note = |rows: &mut Vec<Check>, s: String| rows.push(Check { name: "pins".into(), ok: true, detail: s, fix: None });

    let t = load_table(&p.table())?;
    let member_of = validate(&p, &t)?;
    for w in redundant_member_deviations(&t, &member_of) {
        note(&mut rows, w);
    }
    let mut sorted: Vec<&(String, String)> = member_of.iter().collect();
    sorted.sort();
    for (member, goatnet) in sorted {
        let text = render_pins(member, goatnet, &t);
        let path = p.group_vars().join(member).join(GENERATED_BASENAME);
        let relpath = p.rel(&path);
        if check {
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            if current != text {
                failed = true;
                fail(&mut rows, format!("FAIL: {relpath} is out of date with ops/pins/fleet-pins.yaml"));
                for l in unified_diff(&current, &text, &format!("{relpath} (committed)"), &format!("{relpath} (rendered)")) {
                    note(&mut rows, format!("  {l}"));
                }
            }
        } else {
            std::fs::write(&path, &text).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
            note(&mut rows, format!("wrote {relpath}"));
        }
    }

    let caps = load_caps(&p)?;
    let (allow, allow_len, baseline) = load_allowlist(&p)?;
    let outside = scan_outside_pins(&p, &member_of)?;
    let mut goatnets: Vec<&String> = t.goatnets.keys().collect();
    goatnets.sort();
    for goatnet in &goatnets {
        let path = p.intent_path(goatnet);
        let rp = p.rel(&path);
        let committed = read_committed(&path);
        let body = build_body(&p, goatnet, &t, &caps, &member_of, &allow, &outside)?;
        let (text, doc) = render_intent(&body, &committed, None);
        validate_schema(&p, &doc, &format!("rendered intent for '{goatnet}'"))?;
        if check {
            if !committed.is_empty() {
                validate_schema(&p, &committed, &format!("committed {rp}"))?;
                let (stated, actual) = self_consistent(&committed);
                if stated != actual {
                    failed = true;
                    fail(&mut rows, format!("FAIL: {rp} carries content_hash {stated} but its body hashes to {actual} — the file was hand-edited."));
                }
                let v = verify_file(&p, &path, &committed);
                if v.state != "verified" {
                    failed = true;
                    fail(&mut rows, format!("FAIL: {}", v.detail));
                }
            }
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            if current != text {
                failed = true;
                fail(&mut rows, format!("FAIL: {rp} is out of date with the tree."));
                for l in unified_diff(&current, &text, &format!("{rp} (committed)"), &format!("{rp} (rendered)")) {
                    note(&mut rows, format!("  {l}"));
                }
            }
        } else {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(d)).display())))?;
            }
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            std::fs::write(&path, &text).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
            note(&mut rows, format!("wrote {rp} (serial {}, {})", doc["serial"], doc["content_hash"].as_str().unwrap_or("")));
            if current != text {
                note(&mut rows, format!("      body changed — the committed signature is now STALE; {SIGNER_HINT}"));
            }
        }
    }

    if !check {
        return Ok(Report::new("pins", rows, EXIT_REFUSED));
    }

    for (member, key, path) in &outside {
        let row = format!("{member}:{key}");
        let goatnet = member_of.iter().find(|(m, _)| m == member).map(|(_, g)| g.as_str()).unwrap_or("");
        if governed_vars_for(&t.components, goatnet).contains(key) {
            failed = true;
            fail(&mut rows, format!("FAIL: {path} defines {key} ({member}), which is GOVERNED by ops/pins/fleet-pins.yaml."));
            note(&mut rows, "      vars.yml sorts after pins.yml, so this hand pin silently shadows the generated one. Delete it.".into());
        } else if !allow.contains(&row) {
            failed = true;
            fail(&mut rows, format!("FAIL: {path} defines {key} ({member}) outside the generated pins.yml."));
            note(&mut rows, format!("      Migrate it into ops/pins/fleet-pins.yaml, or add '{row}' to ops/pins/unmigrated-pins.txt"));
            note(&mut rows, "      and raise its `# baseline:` line (a visible, reviewable regression).".into());
        }
    }
    let baseline = match baseline {
        Some(b) => b,
        None => {
            failed = true;
            fail(&mut rows, "FAIL: ops/pins/unmigrated-pins.txt declares no `# baseline: <n>` line — the shrink-only ratchet has no number to hold.".into());
            0
        }
    };
    if allow_len > baseline {
        failed = true;
        fail(&mut rows, format!("FAIL: ops/pins/unmigrated-pins.txt has {allow_len} rows; baseline is {baseline} (it may only shrink)."));
    }
    let present: BTreeSet<String> = outside.iter().map(|(m, k, _)| format!("{m}:{k}")).collect();
    let stale: Vec<&String> = allow.iter().filter(|r| !present.contains(*r)).collect();
    if !stale.is_empty() {
        failed = true;
        fail(&mut rows, format!("FAIL: ops/pins/unmigrated-pins.txt lists rows that no longer exist: {}", stale.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        note(&mut rows, "      Delete them and lower the `# baseline:` line — the ratchet only pays off if it tightens.".into());
    }
    if failed {
        note(&mut rows, String::new());
        note(&mut rows, "Canon: docs/design/environment-parity.md §3. Regenerate with `goatbuilder pins` or `python3 ops/pins/generate-pins.py` (both avenues, ADR 1201).".into());
        let mut r = Report::new("pins", rows, EXIT_REFUSED);
        r.ok = false;
        r.exit = EXIT_REFUSED;
        return Ok(r);
    }
    note(&mut rows, format!("ok: fleet pins — {} component(s) across {} member group(s); {allow_len}/{baseline} unmigrated pins allowlisted (shrinking); intent current for {}.", t.components.len(), member_of.len(), goatnets.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    Ok(Report::new("pins", rows, EXIT_REFUSED))
}

/// One goatnet's part of `pins --check`, as deploy runs it before any host: its
/// members' pins.yml are what the table renders, and its committed intent is
/// there, its content_hash is its body's, its detached signature verifies, and
/// it is what the tree renders. No network. Each reason is in the words of `generate-pins.py --check --goatnet`,
/// the script avenue's half.
pub fn check_goatnet(repo: &Path, goatnet: &str) -> GoatnetCheck {
    let mut c = GoatnetCheck::default();
    if let Err(e) = check_goatnet_into(repo, goatnet, &mut c) {
        c.why.push(format!("{e:#}"));
    }
    c
}

/// What [`check_goatnet`] found.
#[derive(Default)]
pub struct GoatnetCheck {
    /// Why the intent does not verify; empty when it does.
    pub why: Vec<String>,
    /// What verified.
    pub detail: String,
    /// The command it started.
    pub ran: Option<String>,
}

fn check_goatnet_into(repo: &Path, goatnet: &str, c: &mut GoatnetCheck) -> Result<()> {
    let p = Paths::new(repo);
    let t = load_table(&p.table())?;
    let member_of = validate(&p, &t)?;
    if !t.goatnets.contains_key(goatnet) {
        return Err(goat_errors::coded("GB-002", format!("unknown goatnet {goatnet:?}")).into());
    }
    let mut members: Vec<&String> = member_of.iter().filter(|(_, g)| g == goatnet).map(|(m, _)| m).collect();
    members.sort();
    for m in members {
        let path = p.group_vars().join(m).join(GENERATED_BASENAME);
        if std::fs::read_to_string(&path).unwrap_or_default() != render_pins(m, goatnet, &t) {
            c.why.push(format!("{} is out of date with ops/pins/fleet-pins.yaml", p.rel(&path)));
        }
    }
    let path = p.intent_path(goatnet);
    let rp = p.rel(&path);
    let committed = read_committed(&path);
    if committed.is_empty() {
        c.why.push(format!("{rp} is not there, or is not a JSON object: goatnet {goatnet} has no signed intent"));
        return Ok(());
    }
    let (stated, actual) = self_consistent(&committed);
    if stated != actual {
        c.why.push(format!("{rp} carries content_hash {stated} but its body hashes to {actual} — the file was hand-edited."));
    }
    let v = verify_file(&p, &path, &committed);
    c.ran = v.ran;
    if v.state == "verified" {
        c.detail = v.detail;
    } else {
        c.why.push(v.detail);
    }
    validate_schema(&p, &committed, &format!("committed {rp}"))?;
    let caps = load_caps(&p)?;
    let (allow, _, _) = load_allowlist(&p)?;
    let outside = scan_outside_pins(&p, &member_of)?;
    let body = build_body(&p, goatnet, &t, &caps, &member_of, &allow, &outside)?;
    let (text, _) = render_intent(&body, &committed, None);
    if std::fs::read_to_string(&path).unwrap_or_default() != text {
        c.why.push(format!("{rp} is out of date with the tree."));
    }
    Ok(())
}

/// Render everything without writing: what `release` and `fastlane` call to
/// regenerate pins and intents as a typed step.
pub fn render_all(repo: &Path) -> Result<Rendered> {
    let p = Paths::new(repo);
    let t = load_table(&p.table())?;
    let member_of = validate(&p, &t)?;
    let caps = load_caps(&p)?;
    let (allow, _, _) = load_allowlist(&p)?;
    let outside = scan_outside_pins(&p, &member_of)?;
    let mut sorted: Vec<&(String, String)> = member_of.iter().collect();
    sorted.sort();
    let pins = sorted.iter().map(|(m, g)| (m.clone(), p.group_vars().join(m).join(GENERATED_BASENAME), render_pins(m, g, &t))).collect();
    let mut goatnets: Vec<&String> = t.goatnets.keys().collect();
    goatnets.sort();
    let mut intents = Vec::new();
    for g in goatnets {
        let path = p.intent_path(g);
        let committed = read_committed(&path);
        let body = build_body(&p, g, &t, &caps, &member_of, &allow, &outside)?;
        let (text, doc) = render_intent(&body, &committed, None);
        let changed = std::fs::read_to_string(&path).unwrap_or_default() != text;
        intents.push((g.clone(), path, text, doc, changed));
    }
    Ok(Rendered { pins, intents })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over_table(devs: Value, fleet: Value) -> (Table, Vec<(String, String)>) {
        let goatnets = serde_json::json!({"zagros": {"members": ["zagros_a_hosts", "zagros_b_hosts"]}});
        let comps = serde_json::json!({"app": {"var": "app_image", "fleet": fleet, "deviations": devs}});
        let t = Table { goatnets: goatnets.as_object().unwrap().clone(), components: comps.as_object().unwrap().clone(), lite: BTreeSet::new() };
        let m = vec![("zagros_a_hosts".to_string(), "zagros".to_string()), ("zagros_b_hosts".to_string(), "zagros".to_string())];
        (t, m)
    }

    fn over_refusal(devs: Value, fleet: Value) -> Option<String> {
        let (t, m) = over_table(devs, fleet);
        check_member_over(&t, &m).err().map(|e| format!("{e:#}"))
    }

    const OLD: &str = "ghcr.io/example/app@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const FIX: &str = "ghcr.io/example/app@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const PIN: &str = "ghcr.io/example/app@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    #[test]
    fn a_member_deviation_written_over_an_older_goatnet_value_is_refused_gb357() {
        let msg = over_refusal(serde_json::json!({"zagros": {"value": FIX, "reason": "hotfix"}, "zagros_b_hosts": {"value": PIN, "over": OLD, "reason": "r"}}), Value::String(OLD.into())).unwrap();
        assert!(msg.starts_with("GB-357") && msg.contains(&format!("over: \"{FIX}\"")), "{msg}");
    }

    #[test]
    fn a_member_value_without_over_is_refused_gb358_with_the_line_to_paste() {
        let msg = over_refusal(serde_json::json!({"zagros_b_hosts": {"value": PIN, "reason": "r"}}), Value::String(OLD.into())).unwrap();
        assert!(msg.starts_with("GB-358") && msg.contains(&format!("over: \"{OLD}\"")), "{msg}");
    }

    #[test]
    fn over_naming_the_current_value_absent_members_and_goatnet_deviations_pass() {
        assert!(over_refusal(serde_json::json!({"zagros": {"value": FIX, "reason": "h"}, "zagros_b_hosts": {"value": PIN, "over": FIX, "reason": "r"}}), Value::String(OLD.into())).is_none());
        assert!(over_refusal(serde_json::json!({"zagros": {"value": FIX, "reason": "h"}, "zagros_b_hosts": {"absent": true, "reason": "r"}}), Value::String(OLD.into())).is_none());
        assert!(over_refusal(serde_json::json!({"zagros_a_hosts": {"value": PIN, "over": "none", "reason": "r"}, "zagros_b_hosts": {"value": PIN, "over": "none", "reason": "r"}}), Value::Null).is_none());
    }

    #[test]
    fn over_where_it_means_nothing_is_refused_gb358() {
        let on_goatnet = over_refusal(serde_json::json!({"zagros": {"value": FIX, "over": OLD, "reason": "h"}}), Value::String(OLD.into())).unwrap();
        assert!(on_goatnet.starts_with("GB-358") && on_goatnet.contains("goatnet deviation"), "{on_goatnet}");
        let on_absent = over_refusal(serde_json::json!({"zagros_b_hosts": {"absent": true, "over": OLD, "reason": "r"}}), Value::String(OLD.into())).unwrap();
        assert!(on_absent.starts_with("GB-358") && on_absent.contains("absent"), "{on_absent}");
    }

    #[test]
    fn a_redundant_member_deviation_is_one_warning() {
        let (t, m) = over_table(serde_json::json!({"zagros_b_hosts": {"value": OLD, "over": OLD, "reason": "r"}}), Value::String(OLD.into()));
        assert!(check_member_over(&t, &m).is_ok());
        let w = redundant_member_deviations(&t, &m);
        assert!(w.len() == 1 && w[0].starts_with("WARN pins") && w[0].contains("zagros_b_hosts"), "{w:?}");
    }

    const REL: &str = "example-org/example-tool@v1.2.0@sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    /// One component in a table with no goatnet: validate() reads only the component.
    fn release_refusal(comp: Value) -> Option<String> {
        let t = Table { goatnets: Map::new(), components: json!({"tool": comp}).as_object().unwrap().clone(), lite: BTreeSet::new() };
        validate(&Paths::new(Path::new("/nonexistent")), &t).err().map(coded_text)
    }

    /// "<code> <message>" of a coded refusal.
    fn coded_text(e: anyhow::Error) -> String {
        let code = e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone()).unwrap_or_default();
        format!("{code} {e:#}")
    }

    #[test]
    fn one_host_var_is_a_variable_name_and_not_beside_a_single_homed_role() {
        let c = |v: Value| check_one_host("map", &v).err().map(coded_text).unwrap_or_default();
        assert_eq!(c(json!({"one_host_var": "map_host"})), "");
        assert_eq!(c(json!({})), "");
        for bad in [json!({"one_host_var": "Map-Host"}), json!({"one_host_var": 3}), json!({"one_host_var": ""})] {
            assert!(c(bad).contains("component 'map': `one_host_var:` is the name of a group variable that names one host, such as example_host"));
        }
        assert!(c(json!({"one_host_var": "map_host", "single_homed_role": "A"})).contains("component 'map' declares both `one_host_var:` and `single_homed_role:`; a component runs on one host of each group by one of them"));
    }

    fn release_comp() -> Value {
        json!({"var": "tool_release", "artifact_type": "release", "fleet": REL,
               "release": {"checksums": "SHA256SUMS", "files": {"amd64": "tool-linux-amd64", "arm64": "tool-linux-arm64"}, "signer": "https://github.com/example-org/example-tool/.github/workflows/release.yml"}})
    }

    #[test]
    fn a_release_component_with_its_declaration_and_a_release_pin_passes() {
        assert_eq!(release_refusal(release_comp()), None);
        let mut one_arch = release_comp();
        one_arch["release"]["files"] = json!({"arm64": "tool-linux-arm64"});
        assert_eq!(release_refusal(one_arch), None);
        for format in ["cosign", "sigstore", "attestation"] {
            let mut named = release_comp();
            named["release"]["bundle"] = json!(format);
            assert_eq!(release_refusal(named), None);
        }
        let mut carried = release_comp();
        carried["release"]["play"] = json!("deploy/ansible/tool-deploy.yml");
        assert_eq!(release_refusal(carried), None);
        let mut unset = release_comp();
        unset["fleet"] = Value::Null;
        assert_eq!(release_refusal(unset), None, "fleet: null with no goatnet to resolve is not a release-pin question");
        let mut slash = release_comp();
        slash["fleet"] = json!(REL.replace("@v1.2.0@", "@tool/v1.2.0@"));
        assert_eq!(release_refusal(slash), None, "a tag may hold a slash, as goatbuilder/v0.7.3 does");
    }

    #[test]
    fn a_release_component_missing_or_misdeclaring_its_release_is_refused_gb365() {
        let case = |edit: &dyn Fn(&mut Value), want: &str| {
            let mut c = release_comp();
            edit(&mut c);
            let msg = release_refusal(c).unwrap_or_default();
            assert!(msg.starts_with("GB-365") && msg.contains(want), "{want}: {msg}");
        };
        case(&|c| { c.as_object_mut().unwrap().remove("release"); }, "has no `release:` mapping");
        case(&|c| c["release"] = json!("SHA256SUMS"), "has no `release:` mapping");
        case(&|c| c["release"]["url"] = json!("x"), "unknown key 'url'");
        case(&|c| c["release"]["bundle"] = json!("other"), "`release.bundle:` must be one of cosign, sigstore, attestation");
        case(&|c| c["release"]["bundle"] = Value::Null, "`release.bundle:`");
        case(&|c| c["release"]["bundle"] = json!(["cosign"]), "`release.bundle:`");
        case(&|c| c["release"]["play"] = json!("../play.yml"), "`release.play:` is a play's path");
        case(&|c| c["release"]["play"] = json!("deploy/play.sh"), "`release.play:` is a play's path");
        case(&|c| c["release"]["play"] = Value::Null, "`release.play:` is a play's path");
        case(
            &|c| {
                c["release"]["play"] = json!("deploy/play.yml");
                c["deploy_playbook"] = json!("tool-deploy.yml");
            },
            "its play is in one place",
        );
        case(&|c| { c["release"].as_object_mut().unwrap().remove("checksums"); }, "`release.checksums:`");
        case(&|c| c["release"]["checksums"] = json!("../SHA256SUMS"), "`release.checksums:`");
        case(&|c| c["release"]["checksums"] = json!("tool.attestations.json"), "`release.checksums:`");
        case(&|c| c["release"]["files"]["amd64"] = json!("tool-linux-arm64.attestations.json"), "`release.files.amd64:`");
        case(&|c| c["release"]["files"] = json!({}), "`release.files:` must map");
        case(&|c| c["release"]["files"]["riscv64"] = json!("tool"), "architecture 'riscv64'");
        case(&|c| c["release"]["files"]["amd64"] = json!("bin/tool"), "`release.files.amd64:`");
        case(&|c| c["release"]["signer"] = json!(" "), "`release.signer:`");
        case(&|c| c["release"]["signer"] = json!("https://github.com/example-org/example-tool/.github/workflows/release.yml@refs/tags/v1.2.0"), "with no ref");
        case(&|c| c["release"]["signer"] = json!("example-org/example-tool"), "`release.signer:` is the workflow");
        case(&|c| c["fleet"] = json!("example-org/example-tool@tool//v1@sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"), "is not a release pin");
        case(&|c| c["fleet"] = json!("ghcr.io/example-org/tool@sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"), "is not a release pin");
        case(&|c| c["fleet"] = json!("example-org/example-tool@sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"), "is not a release pin");
        case(&|c| c["release"]["files"]["amd64"] = json!("tool linux amd64"), "`release.files.amd64:`");
        case(&|c| c["release"]["checksums"] = json!("SHA256SUMS;x"), "`release.checksums:`");
    }

    /// The signer names the workflow, and its repository is the pin's: a pin
    /// of another repository is refused even when the signer is unchanged.
    #[test]
    fn a_release_pin_s_repository_must_be_its_signer_s_repository() {
        let mut other = release_comp();
        other["fleet"] = json!(REL.replace("example-org/example-tool@", "other-org/example-tool@"));
        let msg = release_refusal(other).unwrap_or_default();
        assert!(msg.starts_with("GB-365") && msg.contains("the pin's repository 'other-org/example-tool' is not the repository of `release.signer:` ('example-org/example-tool')"), "{msg}");
        let mut fork = release_comp();
        fork["fleet"] = json!(REL.replace("/example-tool@", "/example-tool-fork@"));
        assert!(release_refusal(fork).unwrap_or_default().contains("a release pin's repository must be its signer's repository"));
        let mut devs = Map::new();
        devs.insert("zagros".into(), json!({"value": REL.replace("example-org/", "other-org/"), "reason": "r"}));
        let c = release_comp();
        let msg = check_release("tool", &c, &c["fleet"], &devs).err().map(coded_text).unwrap_or_default();
        assert!(msg.contains("deviation 'zagros'") && msg.contains("must be its signer's repository"), "{msg}");
        let mut case = release_comp();
        case["fleet"] = json!(REL.replace("example-org/example-tool@", "Example-Org/Example-Tool@"));
        assert_eq!(release_refusal(case), None, "GitHub's names are not case-sensitive");
    }

    #[test]
    fn a_release_deviation_carries_a_release_pin_too() {
        let mut devs = Map::new();
        devs.insert("zagros".into(), json!({"value": "example-org/example-tool@v1.3.0-rc.1", "reason": "r"}));
        let c = release_comp();
        let msg = check_release("tool", &c, &c["fleet"], &devs).err().map(coded_text).unwrap_or_default();
        assert!(msg.starts_with("GB-365") && msg.contains("deviation 'zagros'") && msg.contains("is not a release pin"), "{msg}");
        devs.insert("zagros".into(), json!({"value": REL.replace("v1.2.0", "v1.3.0"), "reason": "r"}));
        assert!(check_release("tool", &c, &c["fleet"], &devs).is_ok());
        devs.insert("zagros".into(), json!({"absent": true, "reason": "r"}));
        assert!(check_release("tool", &c, &c["fleet"], &devs).is_ok());
    }

    #[test]
    fn only_a_release_component_names_a_release() {
        let mut c = release_comp();
        c["artifact_type"] = json!("image");
        c["fleet"] = json!(OLD);
        let msg = release_refusal(c).unwrap_or_default();
        assert!(msg.starts_with("GB-365") && msg.contains("declares `release:` but its artifact_type is 'image'"), "{msg}");
    }

    #[test]
    fn canonical_json_sorts_keys_indents_two_and_ends_with_one_newline() {
        let v = json!({"b": 1, "a": {"d": [], "c": {}}, "s": "é\"\\\n"});
        assert_eq!(canonical_json(&v), "{\n  \"a\": {\n    \"c\": {},\n    \"d\": []\n  },\n  \"b\": 1,\n  \"s\": \"é\\\"\\\\\\n\"\n}\n");
    }

    #[test]
    fn content_hash_excludes_the_generation_identity() {
        let mut a = Map::new();
        a.insert("x".into(), json!(1));
        let mut b = a.clone();
        b.insert("serial".into(), json!(9));
        b.insert("generated_at".into(), json!("now"));
        b.insert("content_hash".into(), json!("sha256:0"));
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn wrap_comment_matches_the_generator() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let got = wrap_comment("the `zagros` goatnet's mgmt nodes are one fabric (design doc §7 node-parity rule). zagros_hosts and zagros_b_hosts and zagros_cloud_hosts render from the same table row, so bumping one and missing the other is structurally impossible.");
        // the generator counts the trailing space before it decides to wrap
        assert_eq!(got, vec![
            "# the `zagros` goatnet's mgmt nodes are one fabric (design doc §7 node-parity",
            "# rule). zagros_hosts and zagros_b_hosts and zagros_cloud_hosts render from the",
            "# same table row, so bumping one and missing the other is structurally",
            "# impossible.",
        ]);
        assert!(wrap_comment("   ").is_empty());
    }

    #[test]
    fn render_value_normalises_as_the_package_does() {
        assert_eq!(render_value(&json!(true), "s").key.as_deref(), Some("true"));
        assert_eq!(render_value(&json!("Yes"), "s").key.as_deref(), Some("true"));
        assert_eq!(render_value(&json!("off"), "s").key.as_deref(), Some("false"));
        assert_eq!(render_value(&json!(null), "s").key.as_deref(), Some("null"));
        assert_eq!(render_value(&json!(5), "s").key.as_deref(), Some("5"));
        let r = render_value(&json!("{{ x }}"), "s");
        assert_eq!((r.state, r.detail.as_deref()), ("unreadable", Some("templated")));
        let r = render_value(&json!("a lookup('env', 'X') b"), "s");
        assert_eq!(r.state, "unreadable");
    }

    #[test]
    fn resolve_precedence_is_member_then_goatnet_then_fleet() {
        let spec = json!({"fleet": "f@sha256:0", "deviations": {"g": {"value": "gv", "reason": "r"}, "m": {"absent": true, "reason": "r"}}});
        assert_eq!(resolve(&spec, "m", "g").0, None);
        assert_eq!(resolve(&spec, "other", "g").0, Some(json!("gv")));
        assert_eq!(resolve(&spec, "other", "h").0, Some(json!("f@sha256:0")));
        assert_eq!(deviation_scope(&spec, "other", "g").as_deref(), Some("g"));
    }

    /// An opt-in component resolves only for a goatnet whose row lists it
    /// (resolve.py apply_opt_in / opted_out, same words).
    #[test]
    fn an_opt_in_component_resolves_only_where_a_goatnet_opts_in() {
        let goatnets: Map<String, Value> = serde_json::from_value(json!({
            "g": {"members": ["m"], "opt_in": ["map"]}, "h": {"members": ["n"]}})).unwrap();
        let mut comps: Map<String, Value> = serde_json::from_value(json!({
            "map": {"var": "map_image", "opt_in": true, "fleet": "f@sha256:0"},
            "plain": {"var": "plain_image", "fleet": "p@sha256:0"}})).unwrap();
        apply_opt_in(&goatnets, &mut comps).unwrap();
        assert_eq!(resolve(&comps["map"], "m", "g").0, Some(json!("f@sha256:0")));
        assert_eq!(resolve(&comps["map"], "n", "h"), (None, None));
        assert!(opted_out(&comps["map"], "h") && !opted_out(&comps["map"], "g") && !opted_out(&comps["plain"], "h"));
        assert_eq!(resolve(&comps["plain"], "n", "h").0, Some(json!("p@sha256:0")));
        // Its var is governed only where a goatnet opted in (resolve.py governed_vars_for).
        let names = |g: &str| governed_vars_for(&comps, g).into_iter().collect::<Vec<_>>();
        assert_eq!((names("g"), names("h")), (vec!["map_image".to_string(), "plain_image".to_string()], vec!["plain_image".to_string()]));
        let refused = |g: Value| {
            let g: Map<String, Value> = serde_json::from_value(g).unwrap();
            apply_opt_in(&g, &mut comps.clone()).err().map(coded_text).unwrap_or_default()
        };
        assert!(refused(json!({"g": {"members": ["m"], "opt_in": ["plain"]}})).contains("goatnet 'g': `opt_in:` names 'plain', which is not an opt-in component (`opt_in: true`)"));
        assert!(refused(json!({"g": {"members": ["m"], "opt_in": ["nope"]}})).contains("which is not a component"));
        assert!(refused(json!({"g": {"members": ["m"], "opt_in": "map"}})).contains("`opt_in:` must be a list of component names"));
    }

    #[test]
    fn render_intent_is_a_byte_no_op_when_the_body_is_unchanged() {
        let mut body = Map::new();
        body.insert("format".into(), json!(INTENT_FORMAT));
        let (text1, doc1) = render_intent(&body, &Map::new(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(doc1["serial"], json!(1));
        let (text2, doc2) = render_intent(&body, &doc1, Some("2026-02-02T00:00:00Z"));
        assert_eq!(text1, text2);
        assert_eq!(doc2["generated_at"], json!("2026-01-01T00:00:00Z"));
        let mut body2 = body.clone();
        body2.insert("x".into(), json!(1));
        let (_, doc3) = render_intent(&body2, &doc2, Some("2026-03-03T00:00:00Z"));
        assert_eq!(doc3["serial"], json!(2));
        assert_eq!(doc3["history"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(doc3["history"][0]["serial"], json!(1));
    }

    #[test]
    fn the_repository_tree_renders_byte_identical_to_its_committed_pins_and_intents() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        // The parity case CI runs in full (ops/goatbuilder/parity-pins.sh); here
        // the committed outputs themselves are the oracle: a tree whose pins and
        // intents are current must render to exactly those bytes.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !repo.join("ops/pins/fleet-pins.yaml").is_file() {
            return;
        }
        let r = render_all(&repo).expect("render");
        for (m, path, text) in &r.pins {
            let committed = std::fs::read_to_string(path).unwrap_or_default();
            assert_eq!(&committed, text, "pins.yml for {m} differs from the committed file");
        }
        for (g, _path, _text, _doc, changed) in &r.intents {
            assert!(!changed, "intent for {g} differs from the committed file");
        }
    }
}

// --- the typed edits the drivers make (release, fastlane) ------------------------------
//
// fleet-pins.yaml's comments are load-bearing review context, so the table is
// edited as TEXT, block by block, exactly as rc.sh and fastlane.sh edit it:
// the component's block is the lines from `  <comp>:` to the next `  <name>:`.

fn component_block(s: &str, comp: &str) -> Option<(usize, usize)> {
    let head = format!("  {comp}:\n");
    let start = if s.starts_with(&head) { Some(0) } else { s.find(&format!("\n{head}")).map(|i| i + 1) }?;
    let after = start + head.len();
    let re = regex::Regex::new(r"(?m)^  [A-Za-z0-9_-]+:\n").unwrap();
    let end = re.find(&s[after..]).map(|m| after + m.start()).unwrap_or(s.len());
    Some((start, end))
}

/// Drop goatnet `g`'s VALUE deviation (its `      g:` line followed by a
/// `        value:` line, through the line before the next 6-space key).
fn drop_value_deviation(block: &str, g: &str) -> String {
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let key_re = regex::Regex::new(r"^      [A-Za-z0-9_-]+:").unwrap();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i] == format!("      {g}:\n") && i + 1 < lines.len() && lines[i + 1].starts_with("        value:") {
            i += 1;
            while i < lines.len() && !key_re.is_match(lines[i]) {
                i += 1;
            }
            continue;
        }
        out.push_str(lines[i]);
        i += 1;
    }
    out
}

/// rc.sh `set_deviation`: declare `reference` on goatnet `g` for `comp`, with the reason.
pub fn edit_set_deviation(path: &Path, comp: &str, g: &str, reference: &str, reason: &str) -> Result<()> {
    let s = std::fs::read_to_string(path).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", path.display())))?;
    let table = load_table(path)?;
    if !table.components.contains_key(comp) {
        return Err(goat_errors::coded("GB-001", format!("AssertionError: {comp} is not a component of {}", path.display())).into());
    }
    let (start, end) = component_block(&s, comp).ok_or_else(|| goat_errors::coded("GB-257", format!("{comp}: no component block in {}", path.display())))?;
    // A goatnet that already declares this value keeps its deviation as it
    // stands, reason included: the reason carries the day it was declared, so
    // rewriting it on a re-run from a later day changed the table and landed a
    // PR for nothing (rc.sh `set_deviation` holds the same rule).
    if s[start..end].contains(&format!("      {g}:\n        value: \"{reference}\"\n")) {
        return Ok(());
    }
    let entry = format!("      {g}:\n        value: \"{reference}\"\n        reason: >-\n          {}\n", reason.split('\n').collect::<Vec<_>>().join("\n          "));
    // An existing value deviation for `g` is replaced where it stands: re-declaring
    // the same value must leave the table byte-identical, so a re-run of a landed
    // step has nothing to land (moving it to the top reordered the map and opened
    // a PR on every re-run; found by the hermetic e2e tier).
    let original = &s[start..end];
    let dropped = drop_value_deviation(original, g);
    if dropped.len() != original.len() {
        let at = original.find(&format!("      {g}:\n        value:")).unwrap_or(0);
        let block = format!("{}{entry}{}", &original[..at], &dropped[at..]);
        return std::fs::write(path, format!("{}{block}{}", &s[..start], &s[end..])).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))));
    }
    let mut block = dropped;
    if let Some(i) = regex::Regex::new(r"(?m)^    deviations:\n").unwrap().find(&block).map(|m| m.end()) {
        block.insert_str(i, &entry);
    } else if let Some(i) = regex::Regex::new(r"(?m)^    fleet: .*\n").unwrap().find(&block).map(|m| m.end()) {
        block.insert_str(i, &format!("    deviations:\n{entry}"));
    }
    std::fs::write(path, format!("{}{block}{}", &s[..start], &s[end..])).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))
}

/// rc.sh's fold step: the candidate becomes `fleet:`; the deviations that carried it are gone.
pub fn edit_fold_fleet(path: &Path, comp: &str, reference: &str, old: &str, tag: &str, rec: &str, goatnets: &[String]) -> Result<()> {
    let s = std::fs::read_to_string(path).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
    let (start, end) = component_block(&s, comp).ok_or_else(|| anyhow::Error::from(goat_errors::coded("GB-257", format!("{comp}: no component block"))))?;
    let mut block = s[start..end].to_string();
    let today = chrono::Local::now().date_naive();
    let rollback = if old.is_empty() { "<none>".to_string() } else { old.rsplit('/').next().unwrap_or(old).to_string() };
    let cmt = format!("  # {today} graduate {tag} (ADR 1120/1173, record {rec}): the preprod candidate\n  # becomes the fleet value; the per-rung deviations that carried it are gone.\n  # ROLLBACK: {rollback}\n");
    let re = regex::Regex::new(r"(?m)^    fleet: .*\n").unwrap();
    let m = re.find(&block).ok_or_else(|| anyhow::Error::from(goat_errors::coded("GB-257", format!("AssertionError: {comp}: no fleet: line"))))?;
    block.replace_range(m.start()..m.end(), &format!("{cmt}    fleet: \"{reference}\"\n"));
    for g in goatnets {
        block = drop_value_deviation(&block, g);
    }
    // an empty `deviations:` map left behind
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let sib = regex::Regex::new(r"^(    [a-z_]+:|  [A-Za-z0-9_-]+:)").unwrap();
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if *l == "    deviations:\n" && (i + 1 == lines.len() || sib.is_match(lines[i + 1])) {
            continue;
        }
        out.push_str(l);
    }
    std::fs::write(path, format!("{}{out}{}", &s[..start], &s[end..])).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))
}

/// The rungs a graduation folds, and the (rung, value) pairs it keeps.
pub type FoldPlan = (Vec<String>, Vec<(String, String)>);

/// rc.sh's step 8 rung filter: which of `rungs` a graduation of `reference`
/// folds. A rung folds when it has no value deviation, when its deviation is
/// this candidate (same digest), or, on `prod`, when an open hotfix record
/// carries it. Any other value deviation is a newer candidate already on its
/// rung and is kept; returned with its value so the run can say so.
pub fn fold_rungs(path: &Path, comp: &str, reference: &str, prod: &str, hotfix_refs: &[String], rungs: &[String]) -> Result<FoldPlan> {
    let pins = read_yaml(path)?;
    let devs = pins.get("components").and_then(|c| c.get(comp)).and_then(|c| c.get("deviations"));
    let dig = |v: &str| v.rsplit('@').next().unwrap_or(v).to_string();
    let hot: Vec<String> = hotfix_refs.iter().map(|r| dig(r)).collect();
    let (mut fold, mut kept) = (Vec::new(), Vec::new());
    for g in rungs {
        match devs.and_then(|d| d.get(g.as_str())).and_then(|d| d.get("value")).and_then(|v| v.as_str()) {
            // Same candidate = same digest, or the same image tag (a dry run's
            // reference carries a digest placeholder).
            Some(v) if dig(v) != dig(reference) && v.split('@').next() != reference.split('@').next() && !(g == prod && hot.contains(&dig(v))) => kept.push((g.clone(), v.to_string())),
            _ => fold.push(g.clone()),
        }
    }
    Ok((fold, kept))
}

/// fastlane.sh's step 4: move the value of record; clear VALUE deviations; keep `absent: true`.
/// Returns the lines the script prints.
pub fn edit_move_fleet(path: &Path, comp: &str, new_ref: &str, old_ref: &str, reason: &str, ack: &str) -> Result<Vec<String>> {
    let src = std::fs::read_to_string(path).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", path.display())))?;
    let head = format!("  {comp}:\n");
    let m = if src.starts_with(&head) { Some(0) } else { src.find(&format!("\n{head}")).map(|i| i + 1) };
    let start = match m {
        Some(i) => i + head.len(),
        None => return Err(goat_errors::coded("GB-001", format!("fastlane: component {comp} not found in fleet-pins.yaml")).into()),
    };
    let nxt = regex::Regex::new(r"(?m)^  [a-z0-9][a-z0-9-]*:\n").unwrap().find(&src[start..]).map(|m| start + m.start()).unwrap_or(src.len());
    let block = &src[start..nxt];
    let today = chrono::Local::now().date_naive();
    let banner = format!("    # FASTLANE {today} (ADR 1127, ack {ack}): {reason}\n    # Went to EVERY goatnet at once with NO soak. Debt is recorded as a\n    # hotfix-class record and renders as hotfix-backfill-owed until this\n    # ref graduates normally.\n    # ROLLBACK: {old_ref}\n");
    let fleet_re = regex::Regex::new(r"(?m)^    fleet: .*$").unwrap();
    let n = fleet_re.find_iter(block).count();
    if n != 1 {
        return Err(goat_errors::coded("GB-021", format!("fastlane: expected exactly one `fleet:` line in {comp}, found {n}")).into());
    }
    let fm = fleet_re.find(block).unwrap();
    let mut new_block = format!("{}{banner}    fleet: \"{new_ref}\"{}", &block[..fm.start()], &block[fm.end()..]);
    let mut printed = Vec::new();
    if let Some(dm) = regex::Regex::new(r"(?m)^    deviations:\n").unwrap().find(&new_block) {
        let dev_start = dm.end();
        let dev_end = regex::Regex::new(r"(?m)^    [a-z_]+:").unwrap().find(&new_block[dev_start..]).map(|m| dev_start + m.start()).unwrap_or(new_block.len());
        let body = new_block[dev_start..dev_end].to_string();
        let key_re = regex::Regex::new(r"^      [A-Za-z_][\w-]*:\s*$").unwrap();
        let absent_re = regex::Regex::new(r"^\s*absent:\s*true\s*$").unwrap();
        let mut kept: Vec<&str> = Vec::new();
        let mut cur: Vec<&str> = Vec::new();
        let mut keep_cur = false;
        for line in body.split_inclusive('\n') {
            if key_re.is_match(line.trim_end_matches('\n')) {
                if !cur.is_empty() && keep_cur {
                    kept.extend(cur.iter());
                }
                cur = vec![line];
                keep_cur = false;
            } else if !cur.is_empty() {
                cur.push(line);
                if absent_re.is_match(line.trim_end_matches('\n')) {
                    keep_cur = true;
                }
            }
        }
        if !cur.is_empty() && keep_cur {
            kept.extend(cur.iter());
        }
        if !kept.is_empty() {
            new_block = format!("{}{}{}", &new_block[..dev_start], kept.concat(), &new_block[dev_end..]);
            printed.push("    deviations: cleared (absent: true markers PRESERVED — topology, not skew)".to_string());
        } else {
            new_block = format!("{}{}", &new_block[..dm.start()], &new_block[dev_end..]);
            printed.push("    deviations for this component: cleared (fastlane converges the fleet)".to_string());
        }
    }
    std::fs::write(path, format!("{}{new_block}{}", &src[..start], &src[nxt..])).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
    printed.push(format!("    fleet: -> {new_ref}"));
    Ok(printed)
}

/// What a regeneration did: the files written, and which intents CHANGED —
/// the only ones a re-sign may touch (an ECDSA signature is fresh every time,
/// so re-signing an unchanged intent is a diff that lands a pull request of
/// nothing but .sig files; measured five such PRs in one morning).
pub struct Regen {
    pub wrote: Vec<String>,
    pub changed_intents: Vec<String>,
}

pub fn regenerate(repo: &Path) -> Result<Regen> {
    let r = render_all(repo)?;
    let p = Paths::new(repo);
    let mut wrote = Vec::new();
    for (_, path, text) in &r.pins {
        std::fs::write(path, text).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
        wrote.push(p.rel(path));
    }
    let mut changed_intents = Vec::new();
    for (g, path, text, _doc, changed) in &r.intents {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", d.display())))?;
        }
        std::fs::write(path, text).map_err(|e| anyhow::Error::from(goat_errors::coded("GB-254", format!("{}: {e}", path.display()))))?;
        wrote.push(p.rel(path));
        if *changed {
            changed_intents.push(g.clone());
        }
    }
    Ok(Regen { wrote, changed_intents })
}

/// The CA key for a goatnet's intent: `GOAT_CA_KEY_<GOATNET>` when set, else
/// ~/.goat/state/<ca-name>/ca.key.age, the name derived from the
/// intent's ca_id (`dev-<name>-ecdsa-<date>`). Returns (key path, override
/// variable, ca_id); whether the file exists is the caller's refusal, in its
/// driver's words.
pub fn ca_key_for(intent: &Path, goatnet: &str) -> Result<(String, String, String)> {
    let ca = std::fs::read_to_string(intent).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.get("ca_id").and_then(|c| c.as_str()).map(String::from)).unwrap_or_default();
    if ca.is_empty() {
        return Err(goat_errors::coded("GB-035", format!("intent for goatnet {goatnet} declares no ca_id")).into());
    }
    let override_var = format!("GOAT_CA_KEY_{}", goatnet.to_uppercase().replace('-', "_"));
    let key = match std::env::var(&override_var) {
        Ok(k) if !k.is_empty() => k,
        _ => {
            let s = ca.trim_start_matches("dev-");
            let name = match s.find("-ecdsa-") { Some(i) => &s[..i], None => s };
            if name.is_empty() {
                return Err(goat_errors::coded("GB-035", format!("cannot derive a CA name from ca_id '{ca}' (goatnet {goatnet})")).into());
            }
            format!("{}/{name}/ca.key.age", crate::settings::state_dir_abs_global())
        }
    };
    Ok((key, override_var, ca))
}

/// `intent-sign` for one goatnet's intent, from the enrollment module root.
/// The audit log is the project's (`settings::ca_audit_log_abs_global`), named
/// on the call, so the entry lands in one file whatever the tool was built from.
pub fn sign_intent(repo: &Path, intent: &Path, key: &str) -> Result<Vec<String>> {
    let home = std::env::var("HOME").unwrap_or_default();
    let audit = crate::settings::ca_audit_log_abs_global();
    let out = Cmd::new("go")
        .current_dir(repo.join("ops/enrollment"))
        .args(["run", "./cmd/intent-sign", "--intent", intent.to_str().unwrap_or(""), "--root-key", key, "--age-identity", &format!("{home}/.config/age/keys.txt"), "--audit-log", &audit])
        .output()
        .map_err(|e| goat_errors::coded("GB-320", format!("could not start go run ./cmd/intent-sign: {e}")))?;
    if !out.status.success() {
        return Err(goat_errors::coded("GB-022", format!("intent-sign for {} failed: {}", intent.display(), String::from_utf8_lossy(&out.stderr).trim())).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect())
}

/// A regenerated tree whose only difference from main is detached signatures
/// (or no difference) has nothing to land: the pin is already merged.
pub fn nothing_to_land(changed_paths: &[String]) -> bool {
    changed_paths.iter().all(|p| p.ends_with(".sig"))
}

#[cfg(test)]
mod lite_tests {
    //! The lite sidecar's merge rules (ADR 1213) — the same cases as
    //! ops/consistency/python/tests/test_lite_sidecar.py, same words.
    use super::*;

    const D: &str = "@sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn fleet() -> String {
        format!("goatnets:\n  zagros:\n    members: [zagros_a]\ncomponents:\n  alpha:\n    var: alpha_image\n    fleet: \"a{D}\"\n    deviations:\n      zagros:\n        value: \"a:tag\"\n        reason: rc\n  beta:\n    var: beta_image\n    fleet: \"b{D}\"\n")
    }

    fn tree(name: &str, lite: Option<&str>) -> crate::testdir::Held<PathBuf> {
        let d = crate::testdir::path(&format!("goatbuilder-lite-{name}-"));
        std::fs::create_dir_all(d.join("ops/pins")).unwrap();
        std::fs::write(d.join("ops/pins/fleet-pins.yaml"), fleet()).unwrap();
        if let Some(l) = lite {
            std::fs::write(d.join("ops/pins/lite-pins.yaml"), l).unwrap();
        }
        d
    }

    #[test]
    fn a_lite_only_checkout_loads_and_an_empty_one_is_refused() {
        let d = tree("lite-only", Some("goatnets:\n  pasture:\n    members: [pasture_hosts]\ncomponents: {}\n"));
        let empty = fleet()
            .replace("goatnets:\n  zagros:\n    members: [zagros_a]\n", "goatnets: {}\n")
            .replace("    deviations:\n      zagros:\n        value: \"a:tag\"\n        reason: rc\n", "");
        std::fs::write(d.join("ops/pins/fleet-pins.yaml"), &empty).unwrap();
        let t = load_table(&d.join("ops/pins/fleet-pins.yaml")).expect("a lite-only checkout loads");
        assert_eq!(t.goatnets.keys().cloned().collect::<Vec<_>>(), vec!["pasture".to_string()]);
        assert!(t.lite.contains("pasture"));
        std::fs::write(d.join("ops/pins/lite-pins.yaml"), "goatnets: {}\ncomponents: {}\n").unwrap();
        let err = match load_table(&d.join("ops/pins/fleet-pins.yaml")) { Ok(_) => panic!("both empty was accepted"), Err(e) => e.to_string() };
        assert!(err.contains("no goatnet is declared"), "{err}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The code an error carries, if it was raised with `goat_errors::coded`.
    fn code_of(e: &anyhow::Error) -> Option<String> {
        e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone())
    }

    #[test]
    fn a_table_that_breaks_the_pins_schema_is_refused_with_gb_365() {
        let d = tree("schema-coded", Some("goatnets: []\ncomponents: {}\n"));
        let e = match load_table(&d.join("ops/pins/fleet-pins.yaml")) {
            Ok(_) => panic!("a sidecar whose goatnets: is a list was accepted"),
            Err(e) => e,
        };
        assert!(e.to_string().contains("`goatnets:` must be a mapping"), "{e}");
        assert_eq!(code_of(&e).as_deref(), Some("GB-365"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unmigrated_pins_list_that_cannot_be_read_is_refused_with_gb_254_and_its_path() {
        let d = tree("allowlist-unreadable", None);
        // A directory where the list belongs: it exists, and reading it fails.
        std::fs::create_dir_all(d.join("ops/pins/unmigrated-pins.txt")).unwrap();
        let e = match load_allowlist(&Paths::new(d.as_path())) {
            Ok(_) => panic!("an unreadable unmigrated-pins list was read"),
            Err(e) => e,
        };
        assert_eq!(code_of(&e).as_deref(), Some("GB-254"), "{e}");
        assert!(e.to_string().contains("unmigrated-pins.txt"), "the refusal names the file: {e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_malformed_unmigrated_pins_row_is_refused_with_gb_365() {
        let d = tree("allowlist-malformed", None);
        std::fs::write(d.join("ops/pins/unmigrated-pins.txt"), "# baseline: 1\nno-colon-here\n").unwrap();
        let e = match load_allowlist(&Paths::new(d.as_path())) {
            Ok(_) => panic!("a row with no colon was accepted"),
            Err(e) => e,
        };
        assert!(e.to_string().contains("malformed row 'no-colon-here'"), "{e}");
        assert_eq!(code_of(&e).as_deref(), Some("GB-365"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn refuses(name: &str, lite: &str, needle: &str) {
        let d = tree(name, Some(lite));
        let err = match load_table(&d.join("ops/pins/fleet-pins.yaml")) {
            Ok(_) => panic!("{name}: was accepted, must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains(needle), "{name}: refused, but not for the right reason: {err}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_sidecar_or_an_empty_one_is_the_fleet_table_alone() {
        for (i, lite) in [None, Some("goatnets: {}\ncomponents: {}\n"), Some("---\n# header\ngoatnets: {}\ncomponents: {}\n")].iter().enumerate() {
            let d = tree(&format!("empty{i}"), *lite);
            let t = load_table(&d.join("ops/pins/fleet-pins.yaml")).unwrap();
            assert_eq!(t.goatnets.keys().collect::<Vec<_>>(), vec!["zagros"]);
            assert_eq!(deviations(&t.components["alpha"]).keys().collect::<Vec<_>>(), vec!["zagros"]);
            assert!(t.lite.is_empty());
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn a_lite_goatnet_merges_its_row_and_its_deviations() {
        let lite = "goatnets:\n  pasture:\n    members: [pasture_hosts]\n    ca_id_var: goat_ca_id\ncomponents:\n  alpha:\n    deviations:\n      pasture:\n        absent: true\n        reason: tier ceiling\n  beta:\n    deviations:\n      pasture_hosts:\n        value: \"b:tag\"\n        reason: dev rung\n";
        let d = tree("merge", Some(lite));
        let t = load_table(&d.join("ops/pins/fleet-pins.yaml")).unwrap();
        assert_eq!(t.goatnets.keys().cloned().collect::<BTreeSet<_>>(), ["zagros", "pasture"].iter().map(|s| s.to_string()).collect());
        assert_eq!(t.lite, ["pasture"].iter().map(|s| s.to_string()).collect());
        assert_eq!(deviations(&t.components["alpha"]).keys().cloned().collect::<BTreeSet<_>>(), ["zagros", "pasture"].iter().map(|s| s.to_string()).collect());
        assert_eq!(deviations(&t.components["beta"])["pasture_hosts"]["value"], json!("b:tag"));
        assert_eq!(t.components["beta"]["fleet"], json!(format!("b{D}")));
        let (v, dev) = resolve(&t.components["alpha"], "pasture_hosts", "pasture");
        assert!(v.is_none());
        assert_eq!(dev.unwrap()["absent"], json!(true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_sidecar_refuses_what_belongs_in_fleet_pins() {
        refuses("both", "goatnets:\n  zagros:\n    members: [x]\ncomponents: {}\n", "one table");
        refuses("unknown-comp", "goatnets:\n  p:\n    members: [p_hosts]\ncomponents:\n  gamma:\n    deviations:\n      p:\n        absent: true\n        reason: r\n", "component definitions live in fleet-pins.yaml");
        refuses("definition", "goatnets:\n  p:\n    members: [p_hosts]\ncomponents:\n  alpha:\n    var: alpha_image\n    deviations:\n      p:\n        absent: true\n        reason: r\n", "component definitions live in fleet-pins.yaml");
        refuses("foreign-scope", "goatnets:\n  p:\n    members: [p_hosts]\ncomponents:\n  alpha:\n    deviations:\n      zagros_a:\n        absent: true\n        reason: r\n", "neither a goatnet declared in this sidecar nor one of its members");
        refuses("scope-in-both", "goatnets:\n  p:\n    members: [zagros]\ncomponents:\n  alpha:\n    deviations:\n      zagros:\n        absent: true\n        reason: r\n", "is also declared in");
        refuses("extra-key", "goatnets: {}\ncomponents: {}\nextra: 1\n", "unknown top-level key");
        refuses("not-a-map", "goatnets: [a]\ncomponents: {}\n", "`goatnets:` must be a mapping");
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    const TABLE: &str = "goatnets:\n  g:\n    members: [m]\ncomponents:\n  alpha:\n    var: alpha_image\n    # kept comment\n    fleet: \"a@sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n    deviations:\n      g:\n        value: \"a@sha256:1111111111111111111111111111111111111111111111111111111111111111\"\n        reason: old\n      h:\n        absent: true\n        reason: topology\n  beta:\n    var: beta_image\n    fleet: \"b@sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n";

    fn table_file(dir: &Path) -> PathBuf {
        let p = dir.join("fleet-pins.yaml");
        std::fs::write(&p, TABLE).unwrap();
        p
    }

    fn scratch(name: &str) -> crate::testdir::Held<PathBuf> {
        let d = crate::testdir::path(&format!("goatbuilder-pins-edit-{name}-"));
        std::fs::create_dir_all(d.join("ops/ansible/group_vars/m")).unwrap();
        d
    }

    #[test]
    fn set_deviation_replaces_the_value_and_keeps_absent_and_comments() {
        let d = scratch("set");
        let p = table_file(&d);
        edit_set_deviation(&p, "alpha", "g", "a@sha256:2222222222222222222222222222222222222222222222222222222222222222", "RC x\nline two").unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains("    # kept comment\n"));
        assert!(s.contains("    deviations:\n      g:\n        value: \"a@sha256:2222222222222222222222222222222222222222222222222222222222222222\"\n        reason: >-\n          RC x\n          line two\n      h:\n        absent: true\n"), "{s}");
        assert!(!s.contains("reason: old"));
        assert!(s.contains("  beta:\n    var: beta_image\n"));
        // a component with no deviations block gets one after fleet:
        edit_set_deviation(&p, "beta", "g", "b@sha256:3333333333333333333333333333333333333333333333333333333333333333", "r").unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.ends_with("  beta:\n    var: beta_image\n    fleet: \"b@sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n    deviations:\n      g:\n        value: \"b@sha256:3333333333333333333333333333333333333333333333333333333333333333\"\n        reason: >-\n          r\n"), "{s}");
    }

    #[test]
    fn set_deviation_twice_is_byte_identical_whatever_the_order() {
        let d = scratch("setidem");
        let p = table_file(&d);
        // A later rung's deviation lands above g (as preprod's lands above dev's);
        // re-declaring g exactly as it stands must then change nothing.
        let r = "a@sha256:2222222222222222222222222222222222222222222222222222222222222222";
        edit_set_deviation(&p, "alpha", "g", "a@sha256:1111111111111111111111111111111111111111111111111111111111111111", "old").unwrap();
        edit_set_deviation(&p, "alpha", "h2", r, "a later rung").unwrap();
        let before = std::fs::read_to_string(&p).unwrap();
        assert!(before.find("      h2:").unwrap() < before.find("      g:").unwrap());
        edit_set_deviation(&p, "alpha", "g", "a@sha256:1111111111111111111111111111111111111111111111111111111111111111", "old").unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert_eq!(before, after, "re-declaring an unchanged deviation must not move it");
    }

    #[test]
    fn fold_fleet_moves_the_value_and_drops_only_value_deviations() {
        let d = scratch("fold");
        let p = table_file(&d);
        edit_fold_fleet(&p, "alpha", "a@sha256:1111111111111111111111111111111111111111111111111111111111111111", "a@sha256:0000000000000000000000000000000000000000000000000000000000000000", "alpha/v1.0.0-rc.1", "rec.yaml", &["g".into(), "h".into()]).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains("  # ROLLBACK: a@sha256:0000000000000000000000000000000000000000000000000000000000000000\n    fleet: \"a@sha256:1111111111111111111111111111111111111111111111111111111111111111\"\n    deviations:\n      h:\n        absent: true\n        reason: topology\n  beta:\n"), "{s}");
        assert!(!s.contains("reason: old"));
    }

    #[test]
    fn fold_fleet_removes_an_emptied_deviations_map() {
        let d = scratch("fold2");
        let p = table_file(&d);
        let s0 = TABLE.replace("      h:\n        absent: true\n        reason: topology\n", "");
        std::fs::write(&p, s0).unwrap();
        edit_fold_fleet(&p, "alpha", "a@sha256:1111111111111111111111111111111111111111111111111111111111111111", "", "t", "r.yaml", &["g".into()]).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(!s.contains("deviations:"), "{s}");
        assert!(s.contains("  # ROLLBACK: <none>\n"));
    }

    #[test]
    fn move_fleet_clears_values_keeps_absent_and_reports_as_the_script() {
        let d = scratch("move");
        let p = table_file(&d);
        let printed = edit_move_fleet(&p, "alpha", "a@sha256:4444444444444444444444444444444444444444444444444444444444444444", "a@sha256:0000000000000000000000000000000000000000000000000000000000000000", "why", "Ack").unwrap();
        assert_eq!(printed, vec!["    deviations: cleared (absent: true markers PRESERVED — topology, not skew)".to_string(), "    fleet: -> a@sha256:4444444444444444444444444444444444444444444444444444444444444444".to_string()]);
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains("    # FASTLANE "));
        assert!(s.contains("    fleet: \"a@sha256:4444444444444444444444444444444444444444444444444444444444444444\"\n    deviations:\n      h:\n        absent: true\n        reason: topology\n  beta:\n"), "{s}");
        assert!(!s.contains("1111111111"));
    }

    #[test]
    fn nothing_to_land_is_true_for_signatures_only() {
        assert!(nothing_to_land(&[]));
        assert!(nothing_to_land(&["ops/consistency/intent/zagros/intent.json.sig".into()]));
        assert!(!nothing_to_land(&["ops/consistency/intent/zagros/intent.json.sig".into(), "ops/pins/fleet-pins.yaml".into()]));
    }
}

#[cfg(test)]
mod fold_rungs_tests {
    use super::fold_rungs;

    #[test]
    fn a_newer_candidate_on_dev_is_kept_and_this_candidates_pins_fold() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("fleet-pins.yaml");
        std::fs::write(
            &p,
            "components:\n  portal:\n    fleet: \"img:old@sha256:aaaa\"\n    deviations:\n      dev:\n        value: \"img:rc.2@sha256:cccc\"\n      pre:\n        value: \"img:rc.1@sha256:bbbb\"\n      prod:\n        value: \"img:hot@sha256:dddd\"\n",
        )
        .unwrap();
        let rungs = ["dev".to_string(), "pre".to_string(), "prod".to_string()];
        let (fold, kept) = fold_rungs(&p, "portal", "other:rc.1@sha256:bbbb", "prod", &["img:hot@sha256:dddd".to_string()], &rungs).unwrap();
        // A dry run's reference carries no digest: the tag alone identifies it.
        let (dry, _) = fold_rungs(&p, "portal", "img:rc.1@sha256:<digest of img:rc.1>", "prod", &[], &rungs).unwrap();
        assert_eq!(dry, vec!["pre".to_string()]);
        assert_eq!(fold, vec!["pre".to_string(), "prod".to_string()]);
        assert_eq!(kept, vec![("dev".to_string(), "img:rc.2@sha256:cccc".to_string())]);

        // Without a hotfix record behind it, a different prod pin is kept too.
        let (fold, _) = fold_rungs(&p, "portal", "img:rc.1@sha256:bbbb", "prod", &[], &rungs).unwrap();
        assert_eq!(fold, vec!["pre".to_string()]);
    }
}

#[cfg(test)]
mod malformed_checkout_file_tests {
    //! A checkout file goatbuilder reads that is not well formed stops with GB-329
    //! (exit 1), not GB-254 ("cannot be read or written"): the file was read.
    use super::*;

    fn code_of(e: &anyhow::Error) -> Option<String> {
        e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone())
    }

    fn exit_of(code: &str) -> Option<i32> {
        goat_errors::explain(code).and_then(|e| e.exit)
    }

    #[test]
    fn a_pin_table_that_is_not_yaml_is_gb_329_and_names_the_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("fleet-pins.yaml");
        std::fs::write(&p, "components:\n  portal: [unclosed\n").unwrap();
        let e = read_yaml(&p).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-329"), "{e:#}");
        assert!(format!("{e:#}").contains("fleet-pins.yaml"), "{e:#}");
        assert_eq!(exit_of("GB-329"), Some(1));
    }

    #[test]
    fn a_missing_pin_table_is_still_gb_254() {
        let d = tempfile::tempdir().unwrap();
        let e = read_yaml(&d.path().join("fleet-pins.yaml")).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-254"), "{e:#}");
    }

    fn checkout_with_schema(text: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path()).intent_schema();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        d
    }

    #[test]
    fn an_intent_schema_that_does_not_compile_is_gb_329() {
        let d = checkout_with_schema("{\"type\": 7}\n");
        let e = validate_schema(&Paths::new(d.path()), &Map::new(), "rendered intent for 'zagros'").unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-329"), "{e:#}");
        assert!(format!("{e:#}").contains("intent-v1.schema.json"), "{e:#}");
    }

    #[test]
    fn an_intent_schema_that_is_not_json_is_gb_329() {
        let d = checkout_with_schema("{ not json\n");
        let e = validate_schema(&Paths::new(d.path()), &Map::new(), "rendered intent for 'zagros'").unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-329"), "{e:#}");
        assert!(format!("{e:#}").contains("intent-v1.schema.json"), "{e:#}");
    }
}
