//! Target types and platform families (ADR 1257, docs/design/goatbuilder-build-targets.md
//! sections 3.1 and 3.2): the roles a machine can have, and the kinds of
//! hardware, each a data file `join-node` and `standup` read
//! (ops/goatbuilder/targets/README.md is the format).
//!
//! The files this build ships are carried in the binary, so a checkout from
//! before they existed plans as it did. A checkout whose directory has files
//! reads its own and none of the binary's, and a private overlay adds files of
//! its own in `ops/goatbuilder/overlay/`. An overlay replaces none: in an assembled tree
//! the files here must be goatcore's own ([`Provenance`]). Everything is checked when it is read, except
//! that the plays exist: a play may come from another repository, so that is
//! checked by a run ([`missing_play_refusal`]), never by a plan.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use jsonschema::{Draft, JSONSchema};
use rust_embed::RustEmbed;
use serde::Deserialize;

const TYPE_SCHEMA: &str = include_str!("../../../ops/goatbuilder/schema/target-type-v1.schema.json");
const FAMILY_SCHEMA: &str = include_str!("../../../ops/goatbuilder/schema/platform-family-v1.schema.json");

const TYPES_DIR: &str = "ops/goatbuilder/targets";
const FAMILIES_DIR: &str = "ops/goatbuilder/families";
const OVERLAY_TYPES_DIR: &str = "ops/goatbuilder/overlay/targets";
const OVERLAY_FAMILIES_DIR: &str = "ops/goatbuilder/overlay/families";

#[derive(RustEmbed)]
#[folder = "../../ops/goatbuilder/targets/"]
struct ShippedTypes;

#[derive(RustEmbed)]
#[folder = "../../ops/goatbuilder/families/"]
struct ShippedFamilies;

/// The built-in steps a type file may name (joinnode.rs runs them). Each one
/// but the last works on the site's core node, so only a type whose group is
/// the site's core group may name it.
pub(crate) const BUILTINS: [&str; 9] = ["seed-and-replicate", "signal-federation", "site-leaves", "front-door", "health-publishers", "mesh-dns", "single-host-obs", "founder-bootstrap", "utility-play-present"];

const CORE_GROUP: &str = "{site_us}_hosts";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum When {
    Founder,
    Joiner,
}

/// Where standup builds a site's machine of the type.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Standup {
    #[serde(rename = "phase-14")]
    Phase14,
    ObsLegs,
}

/// The site's declaration of a machine of the type (its group_vars).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SiteKeys {
    pub declared_by: String,
    #[serde(default)]
    pub absent_reason: Option<String>,
    pub standup: Standup,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Play {
    pub play: String,
    pub args: String,
    /// Where the play lives when it is not in this repository: `overlay`,
    /// `source` (the type's own `source:`, at its commit), or `owner/repository`.
    #[serde(default)]
    pub from: Option<String>,
}

impl Play {
    /// Whether the play is in the type's source repository.
    pub(crate) fn is_from_source(&self) -> bool {
        self.from.as_deref() == Some(SOURCE)
    }
}

/// `from:` of a play in the type's source repository.
const SOURCE: &str = "source";

/// A type's plays in another repository, at a full commit or a signed tag
/// (gate J26).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypeSource {
    pub repository: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub signer: Option<String>,
}

impl TypeSource {
    /// The mapping it was read from.
    pub(crate) fn as_json(&self) -> serde_json::Value {
        let mut m = serde_json::json!({"repository": self.repository});
        for (k, v) in [("commit", &self.commit), ("tag", &self.tag), ("signer", &self.signer)] {
            if let Some(v) = v {
                m[k] = serde_json::json!(v);
            }
        }
        m
    }

    /// What pins it.
    pub(crate) fn reference(&self) -> crate::source::Ref {
        crate::source::Ref::of(&self.as_json())
    }

    /// A digest of the whole pin (repository, commit or tag, signer): what a
    /// phase's check holds the type file to when its plan could not name a commit.
    pub(crate) fn pin_digest(&self) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(self.as_json().to_string()).iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    /// How a person reads what pins it.
    pub(crate) fn shown(&self) -> String {
        match (&self.commit, &self.tag) {
            (Some(c), _) => format!("commit {c}"),
            (None, Some(t)) => format!("the tag {t}, signed by its declared key"),
            _ => String::new(),
        }
    }
}

/// One entry of a type's phases: exactly one of `plays`, `builtin`, `component`,
/// `skip`. A component has a `skip` beside it: why, when no pin for it exists.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub when: Option<When>,
    /// The standup obs leg the entry is (obs alone).
    #[serde(default)]
    pub leg: Option<String>,
    #[serde(default)]
    pub plays: Vec<Play>,
    #[serde(default)]
    pub builtin: Option<String>,
    /// A release component of the pins, installed at its pin.
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub skip: Option<String>,
}

impl Entry {
    /// Whether the entry is for this kind of site.
    pub(crate) fn applies(&self, joiner: bool) -> bool {
        self.when.is_none_or(|w| (w == When::Joiner) == joiner)
    }

    /// The label with its tag in front, as join-node prints it.
    pub(crate) fn tagged(&self, type_name: &str, v: &Values) -> String {
        format!("[{}] {}", self.tag.as_deref().unwrap_or(type_name), self.text(v))
    }

    /// The label alone, as standup prints it.
    pub(crate) fn text(&self, v: &Values) -> String {
        v.render(self.label.as_deref().unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetType {
    #[allow(dead_code)]
    schema: String,
    #[serde(rename = "type")]
    pub name: String,
    pub families: Vec<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    preflight: Option<String>,
    #[serde(default)]
    pub inventory: Option<String>,
    #[serde(default)]
    pub site: Option<SiteKeys>,
    #[serde(default)]
    converges_as: Option<String>,
    /// The minimum encryption scope a machine of the type requires.
    #[serde(default)]
    pub encryption: Option<String>,
    /// What a machine of the type declares in its host_vars (`tap`).
    #[serde(default)]
    pub declares: Vec<String>,
    /// The site's values a machine of the type needs before it is changed.
    #[serde(default)]
    pub inputs: Vec<Input>,
    /// The components a machine of the type runs, deployed after its phases.
    #[serde(default)]
    pub services: Vec<String>,
    /// What `doctor --target --live` reads on each machine of the type, beyond
    /// a tap's end state (`automatic-updates`).
    #[serde(default)]
    pub checks: Vec<String>,
    /// Where its plays `from: source` are: a repository at a full commit.
    #[serde(default)]
    pub source: Option<TypeSource>,
    #[serde(default)]
    pub phases: Vec<Entry>,
}

impl TargetType {
    pub(crate) fn group_template(&self) -> String {
        self.group.clone().unwrap_or_else(|| format!("{{site_us}}_{}_hosts", self.name_us()))
    }

    /// The inventory group a machine of the type sits in at a site.
    pub(crate) fn group(&self, site: &str) -> String {
        self.group_template().replace("{site_us}", &site.replace('-', "_")).replace("{site}", site)
    }

    pub(crate) fn name_us(&self) -> String {
        self.name.replace('-', "_")
    }

    /// What the machine converges as: its own type, or the one it names.
    pub(crate) fn converges_as(&self) -> &str {
        self.converges_as.as_deref().unwrap_or(&self.name)
    }

    /// The config preflight's target for a machine of the type.
    pub(crate) fn preflight(&self, v: &Values) -> String {
        v.render(self.preflight.as_deref().unwrap_or("{site_group}"))
    }

    /// The type's own inventory at a site, relative to ops/ansible.
    pub(crate) fn inventory(&self, site: &str) -> Option<String> {
        self.inventory.as_ref().map(|i| i.replace("{site_us}", &site.replace('-', "_")).replace("{site}", site))
    }

    /// ` -i <the type's inventory>` when the checkout has that file, else empty.
    pub(crate) fn inventory_arg(&self, repo: &Path, site: &str) -> String {
        self.inventory(site).filter(|i| repo.join("ops/ansible").join(i).is_file()).map(|i| format!(" -i {i}")).unwrap_or_default()
    }

    /// Its plays from its source repository, as paths inside it.
    pub(crate) fn source_plays(&self) -> Vec<&str> {
        let v: std::collections::BTreeSet<&str> = self.phases.iter().flat_map(|e| &e.plays).filter(|p| p.is_from_source()).map(|p| p.play.as_str()).collect();
        v.into_iter().collect()
    }

    fn texts(&self) -> Vec<&str> {
        let own = [&self.group, &self.preflight, &self.inventory].into_iter().flatten().map(String::as_str);
        let entries = self.phases.iter().flat_map(|e| [&e.label, &e.skip].into_iter().flatten().map(String::as_str).chain(e.plays.iter().map(|p| p.args.as_str())));
        own.chain(entries).collect()
    }
}

/// Where a run keeps a type's source, for the checkout it runs from.
pub(crate) struct Kept {
    /// The pin the plan was made with, as [`TypeSource::pin_digest`].
    pin: String,
    /// Where the copy is, when its pin says before a run: a commit's.
    at: Option<std::path::PathBuf>,
    repo: std::path::PathBuf,
    type_name: String,
}

/// What a type file's text may name.
pub(crate) struct Values {
    pub site: String,
    pub group: String,
    /// `the founder <site>`, or `the founder` when the site names none.
    pub founder: String,
    /// Where a run keeps the type's source, and the checkout it runs from: a
    /// play `from: source` is run from there.
    pub kept: Option<Kept>,
}

impl Values {
    const NAMES: [&'static str; 5] = ["site", "site_us", "site_group", "group", "founder"];

    pub(crate) fn new(t: &TargetType, site: &str, founder_site: &str) -> Self {
        let founder = if founder_site.is_empty() { "the founder".to_string() } else { format!("the founder {founder_site}") };
        Values { site: site.to_string(), group: t.group(site), founder, kept: None }
    }

    /// The values, with the type's plays `from: source` run from where a run
    /// keeps its source for this checkout. A type with no source is as before.
    pub(crate) fn sourced(mut self, repo: &Path, t: &TargetType) -> Self {
        self.kept = t.source.as_ref().map(|s| Kept { pin: s.pin_digest(), at: crate::source::type_kept(repo, &t.name, &s.reference()), repo: crate::source::absolute(repo), type_name: t.name.clone() });
        self
    }

    /// The first `{...}` of the text that is not one of `allowed`, or a brace
    /// outside one: the same scan as [`Values::render`], so what passes renders.
    fn unknown_name(text: &str, allowed: &[&str]) -> Option<String> {
        let mut rest = text;
        while let Some(at) = rest.find(['{', '}']) {
            if rest[at..].starts_with('}') {
                return Some("}".into());
            }
            let Some(end) = rest[at..].find('}') else { return Some(rest[at..].to_string()) };
            let name = &rest[at + 1..at + end];
            if !allowed.contains(&name) {
                return Some(format!("{{{name}}}"));
            }
            rest = &rest[at + end + 1..];
        }
        None
    }

    /// The text with its names filled in. One pass: a value is never read for names.
    pub(crate) fn render(&self, text: &str) -> String {
        let us = self.site.replace('-', "_");
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find('{') {
            let Some(end) = rest[at..].find('}') else { break };
            out.push_str(&rest[..at]);
            match &rest[at + 1..at + end] {
                "site" => out.push_str(&self.site),
                "site_us" => out.push_str(&us),
                "site_group" => out.push_str(&format!("{us}_hosts")),
                "group" => out.push_str(&self.group),
                "founder" => out.push_str(&self.founder),
                // Refused when the file was read; left as it is.
                _ => out.push_str(&rest[at..=at + end]),
            }
            rest = &rest[at + end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// The plays' command, from ops/ansible: each play with the same driver and
    /// inventories, `limit` after each one's own words.
    pub(crate) fn command(&self, plays: &[Play], r: &str, pre: &str, inv: &str, limit: &str) -> String {
        let runs: Vec<String> = plays
            .iter()
            .map(|p| match (&self.kept, p.is_from_source()) {
                // A play kept outside ops/ansible: the copy held to its commit
                // again just before it runs, goatcore's roles first, and an
                // inventory beside the checkout's group_vars and host_vars.
                (Some(k), true) => {
                    let check = format!("\"${{GOATBUILDER_BIN:-goatbuilder}}\" -C {} type-source {} {}", crate::standup::sh_quote(&k.repo.display().to_string()), k.type_name, p.play);
                    match &k.at {
                        Some(at) => {
                            // The commit its plan named, which the check holds the type file to.
                            let commit = at.file_name().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
                            let check = format!("{check} --commit {commit}");
                            let play = at.join(&p.play);
                            let roles = crate::source::type_roles(&k.repo, at, &play);
                            format!("{check} && ANSIBLE_ROLES_PATH={} {pre} {inv} -i {} {} {}{limit}", crate::standup::sh_quote(&roles), crate::source::VARS_HERE, crate::standup::sh_quote(&play.display().to_string()), self.render(&p.args))
                        }
                        // A tag's copy is the one its check names: the commit
                        // the tag names when the play runs.
                        None => {
                            let goatcore = crate::standup::sh_quote(&k.repo.join("ops/ansible/roles").display().to_string());
                            let beside = p.play.rsplit_once('/').map(|(d, _)| format!(":$root/{d}/roles")).unwrap_or_default();
                            let check = format!("{check} --pin {}", k.pin);
                            format!("root=$({check}) && [ -d \"$root\" ] && ANSIBLE_ROLES_PATH={goatcore}\"{beside}:$root/roles\" {pre} {inv} -i {} \"$root/{}\" {}{limit}", crate::source::VARS_HERE, p.play, self.render(&p.args))
                        }
                    }
                }
                _ => format!("{pre} {inv} {} {}{limit}", p.play, self.render(&p.args)),
            })
            .collect();
        format!("cd '{r}/ops/ansible' && {}", runs.join(" && "))
    }
}

/// A value of the site's configuration a machine of a type needs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The Ansible variable.
    pub var: String,
    /// Values that are placeholders, not values (a role's sentinel).
    #[serde(default)]
    pub refuse: Vec<String>,
    /// What to set, and why the type needs it.
    pub why: String,
}

/// Every type and family this checkout has.
#[derive(Debug, Clone)]
pub(crate) struct Targets {
    types: BTreeMap<String, TargetType>,
    families: Vec<String>,
    /// Each family's models, by name, and what each one's entry says.
    models: BTreeMap<String, Models>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    #[allow(dead_code)]
    schema: String,
    family: String,
    #[allow(dead_code)]
    title: String,
    models: Models,
}

/// A family's models: a list of names, which says nothing about any of them,
/// or each name with its entry (design section 3.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum Models {
    Listed(Vec<String>),
    Described(BTreeMap<String, Model>),
}

impl Models {
    #[cfg(test)]
    fn names(&self) -> Vec<String> {
        match self {
            Models::Listed(names) => names.clone(),
            Models::Described(entries) => entries.keys().cloned().collect(),
        }
    }

    /// What each model's preparation encrypts: `unknown` for a model that is
    /// only listed.
    pub(crate) fn scopes(&self) -> Vec<(String, String)> {
        match self {
            Models::Listed(names) => names.iter().map(|n| (n.clone(), "unknown".to_string())).collect(),
            Models::Described(entries) => entries.iter().map(|(n, m)| (n.clone(), m.encryption.scope.clone())).collect(),
        }
    }
}

/// One model's entry: how a machine of it is prepared, up to the reachable
/// Ubuntu host goatbuilder builds on (design sections 3.1 and 3.5).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub(crate) struct Model {
    pub name: String,
    pub arch: String,
    pub firmware: String,
    pub boot_media: String,
    pub os: ModelOs,
    pub encryption: ModelEncryption,
    pub extras: Vec<String>,
    /// A path in the tree, or `none: ` and the reason.
    pub builder: String,
    #[serde(default)]
    pub directions: Option<String>,
    /// What `doctor --target --live` holds a machine of the model to, beyond
    /// the fields above (small_computer.rs). Only a small computer's has one.
    #[serde(default)]
    pub end_state: Option<ModelEndState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelEndState {
    /// What `/proc/device-tree/model` starts with on the board.
    pub board: String,
    /// The kind of disk the root's partition is on.
    pub root_media: String,
    /// Where FIPS mode holds: `userspace` (the OpenSSL FIPS provider alone) or `kernel` (the kernel's as well).
    pub fips: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub(crate) struct ModelOs {
    pub image: String,
    /// Each Ubuntu release and its state on the model.
    pub ubuntu: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub(crate) struct ModelEncryption {
    /// root, data, none or unknown.
    pub scope: String,
    pub unlock: Vec<String>,
    pub recovery: Vec<String>,
}

/// GB-425 when a type requires an encryption scope that no model of a family
/// it accepts delivers. A family whose models differ passes when one of them
/// delivers it; `doctor --target` names the ones that do not.
fn encryption_problem(t: &TargetType, models: &BTreeMap<String, Models>) -> Option<String> {
    let scope = t.encryption.as_ref()?;
    for f in &t.families {
        let scopes = models.get(f).map(Models::scopes).unwrap_or_default();
        if scopes.iter().any(|(_, s)| s == scope) {
            continue;
        }
        let what = if scopes.is_empty() { "it has no model".to_string() } else { scopes.iter().map(|(m, s)| format!("{m}: {s}")).collect::<Vec<_>>().join(", ") };
        return Some(format!("requires an encrypted {scope}, and no model of the platform family '{f}' delivers it ({what}). Accept only families a model of which delivers it, or correct the model's entry in the family's file"));
    }
    None
}

fn invalid(file: &str, why: impl std::fmt::Display) -> anyhow::Error {
    goat_errors::coded("GB-374", format!("the target file {file} is not valid: {why}")).into()
}

/// Every `.yaml` file of a directory, by name; a directory that is not there
/// has none.
fn files(dir: &Path) -> Result<Vec<(String, String)>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let shown = |p: &Path| p.display().to_string();
    for e in std::fs::read_dir(dir).map_err(|e| invalid(&shown(dir), e))? {
        let p = e.map_err(|e| invalid(&shown(dir), e))?.path();
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.ends_with(".yml") {
            return Err(invalid(&shown(&p), "its name ends in .yml; type and family files end in .yaml"));
        }
        if name.ends_with(".yaml") {
            out.push((name, std::fs::read_to_string(&p).map_err(|e| invalid(&shown(&p), e))?));
        }
    }
    out.sort();
    Ok(out)
}

fn shipped<E: RustEmbed>() -> Vec<(String, String)> {
    E::iter().filter(|n| n.ends_with(".yaml")).filter_map(|n| Some((n.to_string(), String::from_utf8_lossy(&E::get(&n)?.data).into_owned()))).collect()
}

/// The files of one kind, by name, as (where it is, text): the tree's own when
/// its directory has any, else this build's; then the overlay's, whose names
/// are new. A tree's files are read as a whole set, so a type its goatcore
/// removed stays removed whatever goatbuilder reads it.
fn collect(repo: &Path, from: &Provenance, ships: Vec<(String, String)>, dir: &str, overlay: &str) -> Result<BTreeMap<String, (String, String)>> {
    let mut all: BTreeMap<String, (String, String)> = BTreeMap::new();
    let own = files(&repo.join(dir))?;
    if !own.is_empty() {
        for (n, t) in own {
            from.check(repo, &format!("{dir}/{n}"), t.as_bytes())?;
            all.insert(n.clone(), (format!("{dir}/{n}"), t));
        }
    } else {
        all.extend(ships.into_iter().map(|(n, t)| (n.clone(), (format!("{dir}/{n} (this build's)"), t))));
    }
    for (n, t) in files(&repo.join(overlay))? {
        let at = format!("{overlay}/{n}");
        if all.contains_key(&n) {
            return Err(invalid(&at, format!("{dir}/{n} already has that name; an overlay adds names and replaces none")));
        }
        all.insert(n, (at, t));
    }
    Ok(all)
}

/// Where the checkout's own type and family files come from.
///
/// An assembled tree is a git checkout of goatcore at the commit its marker
/// names, with the overlay's files copied over it (the contract of
/// ops/scripts/lib/source_of_record.py). So the tree itself says what goatcore's
/// file is, and goatbuilder's release plays no part: a file is goatcore's when
/// its bytes are that commit's, or a recorded pre-disclosure patch made them.
enum Provenance {
    /// No marker: a checkout, whose files are whatever is being worked on.
    Checkout,
    /// Assembled from local directories (`goatcore_ref=local`): no commit to
    /// compare with. It plans; [`unverified_refusal`] refuses a run.
    Local,
    Assembled { commit: String, patched: Vec<String> },
}

fn overlaid(what: String) -> anyhow::Error {
    goat_errors::coded("GB-378", what).into()
}

impl Provenance {
    fn of(repo: &Path) -> Result<Self> {
        let Some(m) = crate::fresh::marker(repo)? else {
            return Ok(Provenance::Checkout);
        };
        if m.goatcore_ref == "local" {
            return Ok(Provenance::Local);
        }
        // A full commit id, as source_of_record.py requires: a name such as HEAD
        // would hold the files to whatever the tree's own git last committed.
        if m.goatcore_ref.len() != 40 || !m.goatcore_ref.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(overlaid(format!("{} names goatcore_ref={}, which is not a commit id of 40 hexadecimal characters, so its type and family files cannot be told from an overlay's. Re-assemble the tree from the overlay", crate::fresh::MARKER, m.goatcore_ref)));
        }
        let readable = git(repo).args(["cat-file", "-e", &format!("{}^{{commit}}", m.goatcore_ref)]).output().is_ok_and(|o| o.status.success());
        if !readable {
            return Err(overlaid(format!("{} names goatcore {}, which this tree's git does not have, so its type and family files cannot be told from an overlay's. Re-assemble the tree from the overlay", crate::fresh::MARKER, m.goatcore_ref)));
        }
        let patched = if m.embargo.is_empty() { Vec::new() } else { patched_paths(repo) };
        Ok(Provenance::Assembled { commit: m.goatcore_ref, patched })
    }

    /// GB-378 for a file that is not goatcore's.
    fn check(&self, repo: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
        let Provenance::Assembled { commit, patched } = self else {
            return Ok(());
        };
        if patched.iter().any(|p| p == rel) {
            return Ok(());
        }
        let blob = git(repo).args(["cat-file", "blob", &format!("{commit}:{rel}")]).output().map_err(|e| overlaid(format!("{rel} cannot be compared with goatcore {commit} ({e}), so type and family files cannot be told from an overlay's")))?;
        if blob.status.success() && blob.stdout == bytes {
            return Ok(());
        }
        let how = if blob.status.success() { "differs from" } else { "is not in" };
        Err(overlaid(format!("{rel} {how} goatcore {commit}, which this tree was assembled from: an overlay adds types and families, and replaces none. Put the overlay's own in ops/goatbuilder/overlay/, under a name of its own, and re-assemble")))
    }
}

/// git on the tree, reading its objects as they are (no replacement refs).
fn git(repo: &Path) -> crate::runner::Cmd {
    let mut c = crate::runner::Cmd::new("git");
    c.arg("--no-replace-objects").arg("-C").arg(repo);
    c
}

/// The goatcore paths the recorded pre-disclosure patches change, each one
/// held in the tree exactly as they make it: source_of_record.py's own answer.
/// A tree whose script cannot say has none.
fn patched_paths(repo: &Path) -> Vec<String> {
    let out = crate::runner::Cmd::new("python3").arg(repo.join("ops/scripts/lib/source_of_record.py")).args(["patched-paths", "--repo"]).arg(repo).output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect(),
        _ => Vec::new(),
    }
}

/// GB-378 for a run from a tree assembled from local directories: which of its
/// files are goatcore's is not known, and no deploy accepts such a tree.
pub(crate) fn unverified_refusal(repo: &Path) -> Result<()> {
    match Provenance::of(repo)? {
        Provenance::Local => Err(overlaid("this tree was assembled from local directories (goatcore_ref=local), so its type and family files cannot be told from an overlay's. Re-assemble it from the pinned refs; --dry-run plans from it".into())),
        _ => Ok(()),
    }
}

fn read<T: serde::de::DeserializeOwned>(schema: &JSONSchema, file: &str, text: &str) -> Result<T> {
    let v: serde_json::Value = serde_yaml::from_str(text).map_err(|e| invalid(file, e))?;
    let problems: Vec<String> = match schema.validate(&v) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.map(|e| format!("{}: {e}", if e.instance_path.to_string().is_empty() { "(root)".to_string() } else { e.instance_path.to_string() })).collect(),
    };
    if !problems.is_empty() {
        return Err(invalid(file, problems.join("; ")));
    }
    serde_json::from_value(v).map_err(|e| invalid(file, e))
}

fn schema(text: &str) -> Result<JSONSchema> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| goat_errors::coded("GB-025", format!("embedded schema does not compile: {e}")))?;
    JSONSchema::options().with_draft(Draft::Draft7).compile(&v).map_err(|e| goat_errors::coded("GB-025", format!("embedded schema does not compile: {e}")).into())
}

/// Every type and family of the checkout, each one checked.
pub(crate) fn load(repo: &Path) -> Result<Targets> {
    let (types_schema, families_schema) = (schema(TYPE_SCHEMA)?, schema(FAMILY_SCHEMA)?);
    let from = Provenance::of(repo)?;
    let mut families = Vec::new();
    let mut models = BTreeMap::new();
    for (name, (at, text)) in collect(repo, &from, shipped::<ShippedFamilies>(), FAMILIES_DIR, OVERLAY_FAMILIES_DIR)? {
        let f: Family = read(&families_schema, &at, &text)?;
        if format!("{}.yaml", f.family) != name {
            return Err(invalid(&at, format!("it names the family '{}'; the file's name is the family's", f.family)));
        }
        models.insert(f.family.clone(), f.models);
        families.push(f.family);
    }
    let mut types = BTreeMap::new();
    let mut places = BTreeMap::new();
    for (name, (at, text)) in collect(repo, &from, shipped::<ShippedTypes>(), TYPES_DIR, OVERLAY_TYPES_DIR)? {
        let t: TargetType = read(&types_schema, &at, &text)?;
        if format!("{}.yaml", t.name) != name {
            return Err(invalid(&at, format!("it names the type '{}'; the file's name is the type's", t.name)));
        }
        if let Some(f) = t.families.iter().find(|f| !families.contains(f)) {
            return Err(goat_errors::coded("GB-375", format!("{at} accepts the platform family '{f}', which has no file; known: {}", families.join(", "))).into());
        }
        if let Some(why) = shape_problem(&t) {
            return Err(invalid(&at, why));
        }
        if let Some(why) = encryption_problem(&t, &models) {
            return Err(goat_errors::coded("GB-425", format!("{at} {why}")).into());
        }
        places.insert(t.name.clone(), at);
        types.insert(t.name.clone(), t);
    }
    for t in types.values() {
        let Some(other) = &t.converges_as else { continue };
        let why = match types.get(other) {
            None => format!("converges_as names '{other}', which has no type file"),
            Some(o) if o.converges_as.is_some() => format!("converges_as names '{other}', which itself converges as another type"),
            Some(o) if o.group_template() != t.group_template() => format!("it converges as '{other}' and sits in another group, where {other}'s plays would match no host"),
            Some(_) => continue,
        };
        return Err(invalid(&places[&t.name], why));
    }
    Ok(Targets { types, families, models })
}

/// Why a play's words do not limit it to the type's group, if they do not.
/// standup runs a phase-14 play with no `-l`, so its one `-e target=` is all
/// that keeps it to the machines of the type. The words are `-e key=value`
/// pairs and nothing else: ansible-playbook takes abbreviated and combined
/// options (`--extra=@file`, `-vlx`), so any other word could widen or empty
/// the play. Exactly one sets target, to `target={group}`; a type in the core
/// group may write `target={site_group}` (the same group by another name).
fn target_problem(t: &TargetType, args: &str) -> Option<String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    let key_value = regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=[^@\s]+$").unwrap();
    let mut setting = Vec::new();
    for pair in words.chunks(2) {
        match pair {
            ["-e", kv] if key_value.is_match(kv) => {
                if kv.starts_with("target=") {
                    setting.push(*kv);
                }
            }
            _ => return Some(format!("with \"{}\": a play's args are -e key=value pairs and nothing else (no other option, no @ file)", pair.join(" "))),
        }
    }
    let mut allowed = vec!["target={group}"];
    if t.group_template() == CORE_GROUP {
        allowed.push("target={site_group}");
    }
    match setting.as_slice() {
        [one] if allowed.contains(one) => None,
        [] => Some("without -e target={group}".into()),
        ["target={site_group}"] => Some(format!("with -e target={{site_group}}, the site's core group, and the type's machines are in {}: join-node's -l would match no host", t.group_template())),
        [one] => Some(format!("with {one}: the one word that sets target is target={{group}}")),
        _ => Some("setting target more than once: the last one wins, so exactly one word sets it, target={group}".into()),
    }
}

/// What a type file says that its schema cannot refuse.
fn shape_problem(t: &TargetType) -> Option<String> {
    for text in t.texts() {
        if let Some(name) = Values::unknown_name(text, &Values::NAMES) {
            return Some(format!("\"{text}\" names a value it may not ({name}); known: {}", Values::NAMES.iter().map(|n| format!("{{{n}}}")).collect::<Vec<_>>().join(", ")));
        }
    }
    // These say where the machine is, so they take the site's name alone.
    for text in [&t.group, &t.inventory].into_iter().flatten() {
        if Values::unknown_name(text, &["site", "site_us"]).is_some() {
            return Some(format!("\"{text}\" may name {{site}} and {{site_us}} only"));
        }
    }
    // A command's words: {founder} is several words of text, so only a label
    // or a skip reason names it.
    let commands = t.preflight.iter().map(String::as_str).chain(t.phases.iter().flat_map(|e| e.plays.iter().map(|p| p.args.as_str())));
    for text in commands {
        if Values::unknown_name(text, &["site", "site_us", "site_group", "group"]).is_some() {
            return Some(format!("\"{text}\" is part of a command, which may not name {{founder}}: it is text of several words. A label or a skip reason may"));
        }
    }
    if t.converges_as.is_some() != t.phases.is_empty() {
        return Some("it has either phases or converges_as, and one of them".into());
    }
    for (i, e) in t.phases.iter().enumerate() {
        let n = i + 1;
        let kinds = [!e.plays.is_empty(), e.builtin.is_some(), e.component.is_some() || e.skip.is_some()].iter().filter(|k| **k).count();
        if kinds != 1 {
            return Some(format!("phase entry {n} has exactly one of plays, builtin, component and skip"));
        }
        if e.component.is_some() && e.skip.is_none() {
            return Some(format!("phase entry {n} names a component and no skip: the reason when no pin for it exists"));
        }
        if e.builtin.is_none() && e.label.is_none() {
            return Some(format!("phase entry {n} has no label"));
        }
        if e.leg.is_some() && (e.plays.is_empty() || t.site.as_ref().map(|s| s.standup) != Some(Standup::ObsLegs)) {
            return Some(format!("phase entry {n} names a leg: only plays of a type standup builds in obs-legs have one"));
        }
        if let Some(b) = &e.builtin {
            if !BUILTINS.contains(&b.as_str()) {
                return Some(format!("phase entry {n} names the built-in step '{b}', which this goatbuilder does not have; known: {}", BUILTINS.join(", ")));
            }
            if e.label.is_some() || e.tag.is_some() {
                return Some(format!("phase entry {n} is a built-in step, which has its own label"));
            }
            if b != "utility-play-present" && t.group_template() != CORE_GROUP {
                return Some(format!("phase entry {n} names the built-in step '{b}', which works on the site's core node; this type's group is {}", t.group_template()));
            }
        }
        for p in &e.plays {
            if let Some(why) = target_problem(t, &p.args) {
                return Some(format!("phase entry {n} runs {} {why}", p.play));
            }
        }
    }
    if let Some(why) = source_problem(t) {
        return Some(why);
    }
    if !t.services.is_empty() && t.site.as_ref().is_some_and(|s| s.standup == Standup::ObsLegs) {
        return Some("it lists services and standup builds it in obs-legs, which deploys none".into());
    }
    match t.site.as_ref().map(|s| s.standup) {
        Some(Standup::Phase14) if t.phases.is_empty() || t.phases.iter().any(|e| e.builtin.is_some() || e.component.is_some()) => Some("standup builds it in phase 14, which runs plays and declared skips only".into()),
        Some(Standup::ObsLegs) if t.name != "obs" => Some("obs-legs is how standup builds the type obs, and no other".into()),
        Some(Standup::ObsLegs) if ["baseline", "utility", "stack"].iter().any(|l| t.phases.iter().filter(|e| e.leg.as_deref() == Some(l)).count() != 1) => Some("a type standup builds in obs-legs has one entry for each leg: baseline, utility, stack".into()),
        _ => None,
    }
}

/// What a type's `source:` and its plays' `from:` say that cannot be so.
fn source_problem(t: &TargetType) -> Option<String> {
    if let Some(s) = &t.source {
        if !crate::source::repository_ok(&s.repository) {
            return Some("`source.repository:` is a git URL (https, ssh or file) with no credential in it".into());
        }
        if let Some(why) = crate::source::ref_problem(&s.as_json()) {
            return Some(why);
        }
    }
    for (i, e) in t.phases.iter().enumerate() {
        for p in &e.plays {
            if p.is_from_source() && t.source.is_none() {
                return Some(format!("phase entry {} runs {} from: source, and the type declares no source:", i + 1, p.play));
            }
            if !p.is_from_source() && p.play.contains('/') {
                return Some(format!("phase entry {} runs {}: a play of this tree is a file of ops/ansible, named alone; only a play from: source has a path inside its repository", i + 1, p.play));
            }
        }
    }
    if t.site.as_ref().is_some_and(|s| s.standup == Standup::ObsLegs) && !t.source_plays().is_empty() {
        return Some("standup builds it in obs-legs, whose plays are this tree's: no play of it is from: source".into());
    }
    if t.source.is_some() && t.source_plays().is_empty() {
        return Some("it declares source: and no play is from: source".into());
    }
    None
}

impl Targets {
    pub(crate) fn get(&self, name: &str) -> Option<&TargetType> {
        self.types.get(name)
    }

    pub(crate) fn names(&self) -> Vec<&str> {
        self.types.keys().map(String::as_str).collect()
    }

    /// A family's models, if the family has a file.
    pub(crate) fn models(&self, family: &str) -> Option<&Models> {
        self.models.get(family)
    }

    /// The type, or GB-247 naming every type there is.
    pub(crate) fn require(&self, name: &str) -> Result<&TargetType> {
        self.get(name).ok_or_else(|| goat_errors::coded("GB-247", format!("class '{name}' is not one this verb converges; known: {}", self.names().join(", "))).into())
    }

    /// The type whose phases a machine of this type runs.
    pub(crate) fn converged(&self, t: &TargetType) -> Result<&TargetType> {
        self.require(t.converges_as())
    }

    /// The types standup builds in phase 14, in the order it builds them.
    pub(crate) fn phase_14(&self) -> impl Iterator<Item = (&TargetType, &SiteKeys)> {
        self.types.values().filter_map(|t| t.site.as_ref().filter(|s| s.standup == Standup::Phase14).map(|s| (t, s)))
    }

    /// The plays of one of the obs type's legs.
    pub(crate) fn obs_leg(&self, leg: &str) -> Result<&Entry> {
        self.require("obs")?.phases.iter().find(|e| e.leg.as_deref() == Some(leg)).ok_or_else(|| invalid(&format!("{TYPES_DIR}/obs.yaml"), format!("no entry is standup's {leg} leg")))
    }

    /// The family the machine declares, and the file or line it is in.
    pub(crate) fn declared_family(&self, repo: &Path, t: &TargetType, site: &str, node: &str) -> Result<Option<(String, String)>> {
        declared_family(repo, t, site, node)
    }

    /// What the machine must declare besides its family, checked: for a type
    /// that `declares: [tap]`, the tap's line (GB-380 when it has none or it
    /// is not valid). Empty for any other type.
    pub(crate) fn declarations(&self, repo: &Path, t: &TargetType, site: &str, node: &str) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        if t.declares.iter().any(|d| d == crate::tap::KEY) {
            let (tap, at) = crate::tap::required(repo, &t.name, node)?;
            out.push((tap.line(), at));
        }
        for i in &t.inputs {
            out.push(input(repo, t, site, node, i)?);
        }
        Ok(out)
    }

    /// GB-376 when the machine declares a family its type does not accept. One
    /// that declares none is not checked.
    pub(crate) fn check_family(&self, repo: &Path, t: &TargetType, site: &str, node: &str) -> Result<()> {
        self.check_family_of(repo, t, site, node, false)
    }

    /// As [`Targets::check_family`], for a machine standup finds in the type's
    /// group at a joining site, where a machine of a type that converges as
    /// this one shares it (a phone sits in the core's group): it passes when
    /// one of them accepts its family. Which of them the machine is, only
    /// join-node's --class says. A founder has no such machine (a phone joins),
    /// so there the type alone is asked.
    pub(crate) fn check_family_in_group(&self, repo: &Path, t: &TargetType, site: &str, node: &str, joiner: bool) -> Result<()> {
        self.check_family_of(repo, t, site, node, joiner)
    }

    fn check_family_of(&self, repo: &Path, t: &TargetType, site: &str, node: &str, shared: bool) -> Result<()> {
        let Some((family, at)) = declared_family(repo, t, site, node)? else {
            return Ok(());
        };
        if !self.families.contains(&family) {
            return Err(goat_errors::coded("GB-375", format!("{node} declares the platform family '{family}' ({at}), which has no file; known: {}", self.families.join(", "))).into());
        }
        if shared && self.types.values().any(|o| o.converges_as() == t.name && o.group_template() == t.group_template() && o.families.contains(&family)) {
            return Ok(());
        }
        if !t.families.contains(&family) {
            return Err(goat_errors::coded("GB-376", format!("{node} is a {family} machine ({at}), and the type {} accepts only: {}. Build this role on a machine of one of those families, or correct platform_family", t.name, t.families.join(", "))).into());
        }
        Ok(())
    }
}

/// The variable a machine declares its family in.
const FAMILY_KEY: &str = "platform_family";

/// GB-414 when a machine of the type lacks one of its inputs, or has the
/// role's placeholder there; else the line the run record keeps, and where.
pub(crate) fn input(repo: &Path, t: &TargetType, site: &str, node: &str, i: &Input) -> Result<(String, String)> {
    let refused = |what: String| -> anyhow::Error { goat_errors::coded("GB-414", format!("{node}, a {} machine, needs {}: {what}. {}. Nothing on the machine was changed", t.name, i.var, i.why)).into() };
    match declared_var(repo, t, site, node, &i.var, true)? {
        None => Err(refused(format!("it is not set (host_vars/{node}, its inventory line, group_vars/{} or group_vars/all)", t.group(site)))),
        Some((v, at)) if i.refuse.contains(&v) || v.trim().is_empty() => Err(refused(format!("{at} sets it to '{v}', which is a placeholder, not a value"))),
        Some((v, at)) if v.contains("{{") => Ok((format!("{}: templated ({v}); the play reads it", i.var), at)),
        Some((v, at)) => Ok((format!("{}: {v}", i.var), at)),
    }
}

/// Whether anything in the checkout's inventory declares a family: when
/// nothing does, standup asks the inventory nothing more.
pub(crate) fn any_family_declared(repo: &Path) -> bool {
    fn holds(p: &Path) -> bool {
        if p.is_dir() {
            return std::fs::read_dir(p).map(|d| d.flatten().any(|e| holds(&e.path()))).unwrap_or(false);
        }
        std::fs::read(p).map(|b| String::from_utf8_lossy(&b).contains(FAMILY_KEY)).unwrap_or(false)
    }
    ["host_vars", "group_vars", "inventory"].iter().any(|d| holds(&repo.join("ops/ansible").join(d)))
}

/// The family a machine declares, and where, in Ansible's order, highest
/// first: its host_vars; its line in the inventories, mgmt-hosts-mesh.ini (or
/// the door's override) before the type's own before mgmt-hosts.ini (the verbs
/// pass them the other way round, and a later inventory wins); its group's group_vars, then
/// group_vars/all; its group's `[group:vars]` in those inventories, then
/// `[all:vars]`. In a vars directory the last file wins. A parent group's
/// variables are not read, and a templated value is not a family to check.
fn declared_family(repo: &Path, t: &TargetType, site: &str, node: &str) -> Result<Option<(String, String)>> {
    declared_var(repo, t, site, node, FAMILY_KEY, false)
}

/// [`declared_family`]'s reading for any variable. `templated` keeps a
/// templated value (as set); otherwise it is not a value to check.
fn declared_var(repo: &Path, t: &TargetType, site: &str, node: &str, key: &str, templated: bool) -> Result<Option<(String, String)>> {
    let ansible = repo.join("ops/ansible");
    let found = |hit: Option<(String, String)>| hit.filter(|(v, _)| templated || !v.contains("{{"));
    if let Some(hit) = in_vars(&ansible, "host_vars", node, key)? {
        return Ok(found(Some(hit)));
    }
    let inventories: Vec<String> = std::iter::once("inventory/mgmt-hosts-mesh.ini".to_string()).chain(t.inventory(site)).chain(["inventory/mgmt-hosts.ini".to_string()]).collect();
    let texts: Vec<(String, String)> = inventories.iter().map(|i| (i.clone(), std::fs::read_to_string(ansible.join(i)).unwrap_or_default())).collect();
    for (i, text) in &texts {
        // A host listed twice in one inventory: its later line wins.
        let last = text.lines().filter_map(|line| {
            let mut words = line.split_whitespace();
            if words.next() != Some(node) {
                return None;
            }
            words.take_while(|w| !w.starts_with(['#', ';'])).find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
        }).next_back();
        if let Some(v) = last {
            return Ok(found(Some((v.trim_matches(['"', '\'']).to_string(), format!("ops/ansible/{i}")))));
        }
    }
    let group = t.group(site);
    for g in [group.as_str(), "all"] {
        if let Some(hit) = in_vars(&ansible, "group_vars", g, key)? {
            return Ok(found(Some(hit)));
        }
    }
    for g in [group.as_str(), "all"] {
        for (i, text) in &texts {
            if let Some(v) = in_ini_vars(text, g, key) {
                return Ok(found(Some((v, format!("ops/ansible/{i} [{g}:vars]")))));
            }
        }
    }
    Ok(None)
}

/// `key` in an inventory's `[<group>:vars]` section; the last one wins.
fn in_ini_vars(text: &str, group: &str, key: &str) -> Option<String> {
    let head = format!("[{group}:vars]");
    let mut inside = false;
    let mut value = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            // A section header may carry a comment after it.
            inside = line.split(['#', ';']).next().unwrap_or_default().trim() == head;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                let v = v.split([' ', '\t']).find(|w| !w.is_empty()).unwrap_or_default();
                value = Some(v.trim_matches(['"', '\'']).to_string());
            }
        }
    }
    value
}

/// `key` in `<kind>/<name>.yml|.yaml` or the files of `<kind>/<name>/`,
/// read in Ansible's order: the last that sets it wins.
fn in_vars(ansible: &Path, kind: &str, name: &str, key: &str) -> Result<Option<(String, String)>> {
    let mut paths: Vec<std::path::PathBuf> = ["yml", "yaml"].iter().map(|x| ansible.join(kind).join(format!("{name}.{x}"))).collect();
    let mut inside: Vec<std::path::PathBuf> = std::fs::read_dir(ansible.join(kind).join(name)).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml")).collect()).unwrap_or_default();
    inside.sort();
    paths.extend(inside);
    let mut hit = None;
    for p in paths.iter().filter(|p| p.is_file()) {
        let text = std::fs::read_to_string(p).unwrap_or_default();
        if !text.contains(key) {
            continue;
        }
        let at = format!("ops/ansible/{}", p.strip_prefix(ansible).unwrap_or(p).display());
        let unreadable = |why: String| -> anyhow::Error { goat_errors::coded("GB-374", format!("{at}, where a machine's {key} is read from, is not valid: {why}")).into() };
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| unreadable(format!("it is not readable YAML: {e}")))?;
        match doc.get(key) {
            Some(serde_yaml::Value::String(v)) => hit = Some((v.clone(), at)),
            Some(_) if key == FAMILY_KEY => return Err(unreadable(format!("{FAMILY_KEY} is one word, the name of a platform family"))),
            Some(_) => return Err(unreadable(format!("{key} is a string"))),
            None => {}
        }
    }
    Ok(hit)
}

/// GB-377 for the first of these plays the checkout does not have. Only a run
/// asks: a plan and the loader never require a play.
pub(crate) fn missing_play_refusal<'a>(repo: &Path, type_name: &str, plays: impl IntoIterator<Item = &'a Play>, skip_hint: &str) -> Option<anyhow::Error> {
    let p = plays.into_iter().filter(|p| !p.is_from_source()).find(|p| !repo.join("ops/ansible").join(&p.play).is_file())?;
    let fix = match p.from.as_deref() {
        Some("overlay") => "It comes from a private overlay: run from a tree assembled with the overlay that has it".to_string(),
        Some(source) => format!("It comes from {source}: run from a tree assembled with that repository"),
        None => "It is this repository's own: restore it, or bring the checkout to a commit that has it".to_string(),
    };
    Some(goat_errors::coded("GB-377", format!("ops/ansible/{}, which a machine of the type {type_name} converges with, is not in this tree, so no phase has run. {fix}{skip_hint}", p.play)).into())
}

/// GB-420 when a run cannot take the type's plays `from: source` from its
/// source repository at its commit: fetched with this person's own access, or
/// the kept copy shown to be exactly that commit. Only a run asks.
pub(crate) fn source_refusal<'a>(repo: &Path, t: &TargetType, plays: impl IntoIterator<Item = &'a Play>) -> Option<anyhow::Error> {
    // Only a run asks, and it goes on to run the plays.
    crate::source::record_tags();
    source_refused(repo, t, plays, "any phase")
}

fn source_refused<'a>(repo: &Path, t: &TargetType, plays: impl IntoIterator<Item = &'a Play>, before: &str) -> Option<anyhow::Error> {
    source_taken(repo, t, plays, before).err()
}

/// The kept copy `plays` are taken from, checked; none when none of them is
/// from the type's source.
fn source_taken<'a>(repo: &Path, t: &TargetType, plays: impl IntoIterator<Item = &'a Play>, before: &str) -> Result<Option<std::path::PathBuf>> {
    let Some(s) = t.source.as_ref() else { return Ok(None) };
    let plays: std::collections::BTreeSet<&str> = plays.into_iter().filter(|p| p.is_from_source()).map(|p| p.play.as_str()).collect();
    if plays.is_empty() {
        return Ok(None);
    }
    crate::source::fetch_type(repo, &t.name, &s.as_json(), &plays.into_iter().collect::<Vec<_>>(), before).map(Some)
}

/// The hidden `type-source` verb a phase runs just before a play from its
/// type's source: the kept copy held to the commit again, and the play in it.
pub fn source_check(repo: &Path, type_name: &str, play: &str, planned: Option<&str>, pin: Option<&str>) -> i32 {
    // A phase's check is the run's own, just before its play.
    crate::source::record_tags();
    let checked = load(repo).and_then(|t| {
        let ty = t.require(type_name)?.clone();
        if let Some(p) = pin {
            if ty.source.as_ref().map(|s| s.pin_digest()).as_deref() != Some(p) {
                return Err(goat_errors::coded(crate::source::TYPE_CODE, format!("refused before its play ran: the type {type_name}'s plays in its source repository: the type file's source: is not the one this run was planned with; run again from phase 1 (check: declaration)")).into());
            }
        }
        // The copy the phase runs is the one its plan named: a type file
        // changed during the run is not taken halfway through it.
        let now = ty.source.as_ref().map(|s| s.shown()).unwrap_or_default();
        if let Some(p) = planned {
            if ty.source.as_ref().and_then(|s| s.commit.as_deref()) != Some(p) {
                return Err(goat_errors::coded(crate::source::TYPE_CODE, format!("refused before its play ran: the type {type_name}'s plays at {} in its source repository: this run was planned with commit {p}, and the type file now pins {now}; run again from phase 1 (check: declaration)", &p[..p.len().min(12)])).into());
            }
        }
        let p = ty.phases.iter().flat_map(|e| &e.plays).find(|p| p.is_from_source() && p.play == play).cloned();
        match p {
            Some(p) => source_taken(repo, &ty, [&p], "its play ran"),
            None => Err(goat_errors::coded(crate::source::TYPE_CODE, format!("refused before the play: the type {type_name} has no play {play} from: source in its source repository (check: declaration)")).into()),
        }
    });
    match checked {
        Ok(at) => {
            if let Some(at) = at {
                println!("{}", at.display());
            }
            0
        }
        Err(e) => {
            let code = e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone()).unwrap_or_default();
            eprintln!("goatbuilder: {code} {e:#}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn code(e: &anyhow::Error) -> &str {
        e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()).unwrap_or("")
    }

    /// A type that is valid as it stands; each refusal below changes one thing.
    const ACME: &str = "schema: goatbuilder-target/v1\ntype: acme\nfamilies: [host-class]\nphases:\n  - label: an appliance at {site}\n    plays:\n      - {play: acme-deploy.yml, args: \"-e target={group}\", from: overlay}\n";

    /// What loading `text` as the overlay's acme type says.
    fn refused(text: &str) -> (String, String) {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "ops/goatbuilder/overlay/targets/acme.yaml", text);
        let e = load(t.path()).expect_err(text);
        (code(&e).to_string(), e.to_string())
    }

    /// A directory with no type file in it (a README alone) is a tree with
    /// none: the binary's are read, and no stray file empties the set.
    #[test]
    fn a_directory_with_no_type_file_reads_the_binarys() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "ops/goatbuilder/targets/README.md", "notes\n");
        assert_eq!(load(t.path()).unwrap().names(), load(&repo()).unwrap().names());
    }

    #[test]
    fn the_shipped_types_load_with_and_without_a_checkout_that_has_them() {
        let want = ["breakglass", "core", "federation-boundary", "first-bite", "goat-processors", "network-jump", "network-tap", "obs", "octo-pod", "phone", "simple-boundary"];
        assert_eq!(load(&repo()).unwrap().names(), want);
        let empty = tempfile::tempdir().unwrap();
        let t = load(empty.path()).unwrap();
        assert_eq!(t.names(), want);
        assert_eq!(t.require("core").unwrap().group("zagros-b"), "zagros_b_hosts");
        assert_eq!(t.require("phone").unwrap().converges_as(), "core");
        assert_eq!(t.require("obs").unwrap().group("zagros-b"), "zagros_b_obs_hosts");
        assert_eq!(t.require("network-jump").unwrap().group("zagros-prod"), "zagros_prod_network_jump_hosts");
        assert_eq!(code(&t.require("toaster").unwrap_err()), "GB-247");
    }

    /// Every built-in a file names exists, and every built-in is named by a file.
    #[test]
    fn the_built_in_steps_are_the_ones_the_types_name() {
        let t = load(&repo()).unwrap();
        let named: std::collections::BTreeSet<&str> = t.types.values().flat_map(|t| t.phases.iter()).filter_map(|e| e.builtin.as_deref()).collect();
        assert_eq!(named, BUILTINS.into_iter().collect());
    }

    /// Each platform of hardware-classes.yaml is a model of exactly one family.
    #[test]
    fn every_platform_is_in_one_family() {
        let v: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(repo().join("ops/bootstrap/hardware-classes.yaml")).unwrap()).unwrap();
        let mut platforms: Vec<String> = v["platforms"].as_mapping().unwrap().keys().map(|k| k.as_str().unwrap().to_string()).collect();
        let s = schema(FAMILY_SCHEMA).unwrap();
        let mut models: Vec<String> = files(&repo().join(FAMILIES_DIR)).unwrap().iter().flat_map(|(n, t)| read::<Family>(&s, n, t).unwrap().models.names()).collect();
        platforms.sort();
        models.sort();
        assert_eq!(models, platforms);
    }

    /// Every shipped model has its entry, and each entry's builder and
    /// directions are in this tree; the design's family table names each model.
    #[test]
    fn every_shipped_model_has_an_entry_whose_paths_exist() {
        let s = schema(FAMILY_SCHEMA).unwrap();
        let design = std::fs::read_to_string(repo().join("docs/design/goatbuilder-build-targets.md")).unwrap();
        for (n, t) in files(&repo().join(FAMILIES_DIR)).unwrap() {
            let f: Family = read(&s, &n, &t).unwrap();
            let entries = match f.models {
                Models::Described(entries) => entries,
                Models::Listed(names) if names.is_empty() => continue,
                Models::Listed(_) => panic!("{n} lists its models with no entry for each"),
            };
            for (key, m) in entries {
                for path in [Some(&m.builder), m.directions.as_ref()].into_iter().flatten().filter(|p| !p.starts_with("none: ")) {
                    assert!(repo().join(path).exists(), "{n}: {key}'s {path} is not in the tree");
                }
                assert!(design.contains(&format!("`{key}`")), "the design's family table does not name {key} ({n})");
                for release in ["24.04", "26.04"] {
                    assert!(m.os.ubuntu.contains_key(release), "{n}: {key} states nothing for Ubuntu {release}");
                }
            }
        }
    }

    /// A type that requires an encrypted root accepts only a family one model
    /// of which delivers it; a family whose models only list their names
    /// delivers nothing that is known.
    #[test]
    fn a_type_accepts_only_families_whose_models_deliver_its_encryption() {
        let rack = |scope: &str| format!("schema: goatbuilder-family/v1\nfamily: rack\ntitle: A rack server\nmodels:\n  rack-1:\n    name: Rack 1\n    arch: amd64\n    firmware: unknown\n    boot_media: unknown\n    os: {{image: unknown, ubuntu: {{\"24.04\": unknown, \"26.04\": unknown}}}}\n    encryption: {{scope: {scope}, unlock: [unknown], recovery: []}}\n    extras: []\n    builder: \"none: directions only\"\n");
        let typed = ACME.replace("[host-class]", "[rack]").replace("phases:", "encryption: root\nphases:");
        for (family, refused) in [(rack("data"), true), (rack("unknown"), true), ("schema: goatbuilder-family/v1\nfamily: rack\ntitle: A rack server\nmodels: [rack-1]\n".to_string(), true), (rack("root"), false)] {
            let t = tempfile::tempdir().unwrap();
            put(t.path(), "ops/goatbuilder/overlay/families/rack.yaml", &family);
            put(t.path(), "ops/goatbuilder/overlay/targets/acme.yaml", &typed);
            match load(t.path()) {
                Err(e) => assert!(refused && code(&e) == "GB-425" && e.to_string().contains("no model of the platform family 'rack' delivers it (rack-1: "), "{family}: {e}"),
                Ok(_) => assert!(!refused, "{family} was accepted"),
            }
        }
        // A family whose models differ passes on the one that delivers it.
        let t = load(&repo()).unwrap();
        let host: Vec<(String, String)> = t.models("host-class").unwrap().scopes();
        assert!(host.contains(&("karbon-801".into(), "root".into())) && host.contains(&("thor".into(), "data".into())), "{host:?}");
        assert_eq!(t.require("network-tap").unwrap().encryption.as_deref(), Some("root"));
    }

    /// A model's entry is refused when a field is missing or unknown.
    #[test]
    fn a_model_entry_is_checked() {
        let s = schema(FAMILY_SCHEMA).unwrap();
        let good = std::fs::read_to_string(repo().join(FAMILIES_DIR).join("cloud.yaml")).unwrap();
        assert!(read::<Family>(&s, "cloud.yaml", &good).is_ok());
        for bad in [good.replace("    builder: ops/bootstrap/build-image-ec2.sh\n", ""), good.replace("\"26.04\": not built", "\"26.04\": maybe"), good.replace("extras: []", "extras: []\n    colour: grey"), good.replace("scope: root", "scope: most")] {
            let e = read::<Family>(&s, "cloud.yaml", &bad).expect_err(&bad);
            assert_eq!(code(&e), "GB-374", "{e}");
        }
    }

    /// A private type needs no public change: its file and its family's are in
    /// the overlay directory, and its play in no tree until it is run.
    #[test]
    fn an_overlay_adds_a_type_and_a_family() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "ops/goatbuilder/overlay/families/rack.yaml", "schema: goatbuilder-family/v1\nfamily: rack\ntitle: A rack server\nmodels: [rack-1]\n");
        put(t.path(), "ops/goatbuilder/overlay/targets/acme.yaml", &ACME.replace("[host-class]", "[rack]"));
        let targets = load(t.path()).unwrap();
        let acme = targets.require("acme").unwrap();
        assert_eq!(acme.group("zagros-b"), "zagros_b_acme_hosts");
        let v = Values::new(acme, "zagros-b", "");
        let e = &acme.phases[0];
        assert_eq!(e.tagged("acme", &v), "[acme] an appliance at zagros-b");
        assert_eq!(v.command(&e.plays, "/r", "PRE", "INV", " -l n"), "cd '/r/ops/ansible' && PRE INV acme-deploy.yml -e target=zagros_b_acme_hosts -l n");
        let e = missing_play_refusal(t.path(), "acme", &e.plays, "").expect("the play is nowhere");
        assert!(code(&e) == "GB-377" && e.to_string().contains("ops/ansible/acme-deploy.yml") && e.to_string().contains("overlay"), "{e}");
        put(t.path(), "ops/ansible/acme-deploy.yml", "---\n");
        assert!(missing_play_refusal(t.path(), "acme", &acme.phases[0].plays, "").is_none());
    }

    /// A tree that has the directory reads its own files and none of the
    /// binary's: a type its goatcore removed does not come back.
    #[test]
    fn a_checkout_replaces_a_shipped_type_and_an_overlay_may_not() {
        let t = tempfile::tempdir().unwrap();
        let text = std::fs::read_to_string(repo().join("ops/goatbuilder/targets/breakglass.yaml")).unwrap();
        put(t.path(), "ops/goatbuilder/targets/breakglass.yaml", &text.replace("[small-computer, host-class]", "[cloud]"));
        let loaded = load(t.path()).unwrap();
        assert_eq!(loaded.names(), ["breakglass"], "the tree's files are read as a whole set");
        assert_eq!(loaded.require("breakglass").unwrap().families, ["cloud"]);
        put(t.path(), "ops/goatbuilder/overlay/targets/breakglass.yaml", &text);
        let e = load(t.path()).unwrap_err();
        assert!(code(&e) == "GB-374" && e.to_string().contains("overlay adds names"), "{e}");
    }

    fn sh(dir: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// A tree assembled as the public contract says: goatcore's checkout, with
    /// `files` in that commit, and the marker naming it.
    fn assembled(core: &[(&str, &str)]) -> (tempfile::TempDir, String) {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "ops/goatbuilder/README.md", "goatcore\n");
        for (rel, text) in core {
            put(t.path(), rel, text);
        }
        sh(t.path(), &["init", "-q"]);
        sh(t.path(), &["add", "-A"]);
        sh(t.path(), &["commit", "-q", "-m", "goatcore"]);
        let head = sh(t.path(), &["rev-parse", "HEAD"]);
        put(t.path(), ".goat-assembled", &format!("goatcore_ref={head}\noverlay_commit={}\n", "b".repeat(40)));
        (t, head)
    }

    const BG: &str = "ops/goatbuilder/targets/breakglass.yaml";

    /// In an assembled tree an overlay copied over goatcore replaces no type and
    /// no family. A plain checkout's own file is what is being worked on.
    #[test]
    fn an_overlay_copied_over_goatcore_replaces_nothing() {
        let text = std::fs::read_to_string(repo().join(BG)).unwrap();
        let changed = text.replace("[small-computer, host-class]", "[cloud]");
        let (t, _) = assembled(&[(BG, &text)]);
        load(t.path()).expect("goatcore's own file");
        put(t.path(), BG, &changed);
        let e = load(t.path()).unwrap_err();
        assert!(code(&e) == "GB-378" && e.to_string().contains("differs from goatcore"), "{e}");
        // A new name in goatcore's directory: an older goatcore without the
        // file, or a name no goatcore has. Both belong in overlay/.
        let (t, _) = assembled(&[]);
        put(t.path(), BG, &text);
        let e = load(t.path()).unwrap_err();
        assert!(code(&e) == "GB-378" && e.to_string().contains("is not in goatcore"), "{e}");
        let (t, _) = assembled(&[]);
        put(t.path(), "ops/goatbuilder/targets/acme.yaml", ACME);
        assert_eq!(code(&load(t.path()).unwrap_err()), "GB-378");
        let (t, _) = assembled(&[]);
        put(t.path(), "ops/goatbuilder/families/cloud.yaml", "schema: goatbuilder-family/v1\nfamily: cloud\ntitle: Any\nmodels: []\n");
        assert_eq!(code(&load(t.path()).unwrap_err()), "GB-378");
        // Where the overlay's own files go.
        let (t, _) = assembled(&[(BG, &text)]);
        put(t.path(), "ops/goatbuilder/overlay/targets/acme.yaml", ACME);
        load(t.path()).unwrap().require("acme").expect("an overlay adds a type");
        put(t.path(), "ops/goatbuilder/overlay/targets/breakglass.yaml", &text);
        assert_eq!(code(&load(t.path()).unwrap_err()), "GB-374");
        // No marker: the same change is a person at work on goatcore.
        std::fs::remove_file(t.path().join("ops/goatbuilder/overlay/targets/breakglass.yaml")).unwrap();
        std::fs::remove_file(t.path().join(".goat-assembled")).unwrap();
        put(t.path(), BG, &changed);
        assert_eq!(load(t.path()).unwrap().require("breakglass").unwrap().families, ["cloud"]);
    }

    /// The tree's own goatcore commit is what its files are held to, never the
    /// files in the binary: a goatbuilder older or newer than the tree's goatcore
    /// reads a goatcore whose files differ from its own, or that has none.
    #[test]
    fn a_binary_of_another_release_reads_an_assembled_tree() {
        let other = std::fs::read_to_string(repo().join(BG)).unwrap().replace("[small-computer, host-class]", "[cloud]");
        let (newer, _) = assembled(&[(BG, &other), ("ops/goatbuilder/targets/acme.yaml", ACME)]);
        let t = load(newer.path()).unwrap();
        assert_eq!(t.require("breakglass").unwrap().families, ["cloud"]);
        t.require("acme").expect("a type this binary does not ship");
        let (older, _) = assembled(&[]);
        assert_eq!(load(older.path()).unwrap().names(), load(&repo()).unwrap().names());
    }

    /// A marker whose commit is not in the tree's git proves nothing; one of
    /// local directories plans and does not run.
    #[test]
    fn a_tree_that_cannot_be_judged_does_not_run() {
        let (t, _) = assembled(&[]);
        put(t.path(), ".goat-assembled", &format!("goatcore_ref={}\noverlay_commit=x\n", "a".repeat(40)));
        let e = load(t.path()).unwrap_err();
        assert!(code(&e) == "GB-378" && e.to_string().contains("does not have"), "{e}");
        // A name of a commit is not a commit: HEAD would hold the files to the
        // tree's own last commit, an edited type file among them.
        put(t.path(), ".goat-assembled", "goatcore_ref=HEAD\noverlay_commit=x\n");
        let e = load(t.path()).unwrap_err();
        assert!(code(&e) == "GB-378" && e.to_string().contains("40 hexadecimal"), "{e}");
        put(t.path(), ".goat-assembled", "goatcore_ref=local\noverlay_commit=x\n");
        load(t.path()).expect("it plans");
        assert_eq!(code(&unverified_refusal(t.path()).unwrap_err()), "GB-378");
        std::fs::remove_file(t.path().join(".goat-assembled")).unwrap();
        unverified_refusal(t.path()).expect("a checkout runs");
    }

    /// A recorded pre-disclosure patch to a type file is goatcore's; an edit
    /// beside it is not.
    #[test]
    fn a_recorded_pre_disclosure_patch_is_goatcores() {
        let text = std::fs::read_to_string(repo().join(BG)).unwrap();
        let script = std::fs::read_to_string(repo().join("ops/scripts/lib/source_of_record.py")).unwrap();
        let (t, head) = assembled(&[(BG, &text), ("ops/scripts/lib/source_of_record.py", &script)]);
        put(t.path(), BG, &text.replace("[small-computer, host-class]", "[cloud]"));
        let patch = std::process::Command::new("git").arg("-C").arg(t.path()).args(["diff", "--", BG]).output().unwrap().stdout;
        put(t.path(), "embargo/one/core.patch", &String::from_utf8(patch.clone()).unwrap());
        use sha2::Digest;
        let digest: String = sha2::Sha256::digest(&patch).iter().map(|b| format!("{b:02x}")).collect();
        put(t.path(), ".goat-assembled", &format!("goatcore_ref={head}\noverlay_commit={}\nembargo=one/core.patch:{digest}\n", "b".repeat(40)));
        assert_eq!(load(t.path()).unwrap().require("breakglass").unwrap().families, ["cloud"]);
        put(t.path(), BG, &text.replace("[small-computer, host-class]", "[phone]"));
        assert_eq!(code(&load(t.path()).unwrap_err()), "GB-378");
    }

    #[test]
    fn a_file_that_says_something_unknown_is_refused() {
        for (from, to, want, says) in [
            ("families:", "colour: red\nfamilies:", "GB-374", "'colour' was unexpected"),
            ("label:", "shape: round\n    label:", "GB-374", "'shape' was unexpected"),
            ("from: overlay", "repo: overlay", "GB-374", "'repo' was unexpected"),
            ("target/v1", "target/v2", "GB-374", "goatbuilder-target/v1"),
            ("type: acme", "type: other", "GB-374", "the file's name is the type's"),
            ("host-class", "mainframe", "GB-375", "'mainframe'"),
            ("{site}", "{region}", "GB-374", "{region}"),
            ("target={group}", "target=all", "GB-374", "the one word that sets target"),
            ("-e target={group}", "-e site_id={site}", "GB-374", "without -e target="),
            ("-e target={group}", "-e target={group} -e target=all", "GB-374", "more than once"),
            ("-e target={group}", "-e target={group},all", "GB-374", "the one word that sets target"),
            ("-e target={group}", "-e target={group} -l all", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} --limit=all", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} -e @more.yml", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} --extra=@more.yml", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} -vlnothing", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} -i inventory/other.ini", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} -e", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={group} -e note=@x", "GB-374", "-e key=value pairs and nothing else"),
            ("-e target={group}", "-e target={site_group}", "GB-374", "would match no host"),
            ("-e target={group}", "-e target={group} -e peer={founder}", "GB-374", "may not name {founder}"),
            ("an appliance at {site}", "an appliance at {gr{site}oup}", "GB-374", "({gr{site})"),
            ("an appliance at {site}", "an appliance at site}", "GB-374", "names a value it may not"),
            ("an appliance at {site}", "an appliance for {founder}", "", ""),
            ("-e target={group}", "-e target={group}; reboot", "GB-374", "does not match"),
            ("acme-deploy.yml", "../acme-deploy.yml", "GB-374", "does not match"),
            ("families: [host-class]", "families: [host-class]\nconverges_as: core", "GB-374", "either phases or converges_as"),
            ("phases:", "site: {declared_by: has_acme, standup: obs-legs}\nphases:", "GB-374", "and no other"),
            ("    plays:", "    skip: not here\n    plays:", "GB-374", "exactly one of"),
            ("    plays:", "    leg: stack\n    plays:", "GB-374", "names a leg"),
        ] {
            let text = ACME.replace(from, to);
            assert_ne!(text, ACME, "{from}");
            if want.is_empty() {
                let t = tempfile::tempdir().unwrap();
                put(t.path(), "ops/goatbuilder/overlay/targets/acme.yaml", &text);
                load(t.path()).unwrap_or_else(|e| panic!("{to}: {e}"));
                continue;
            }
            let (c, e) = refused(&text);
            assert!(c == want && e.contains(says), "{to}: {c} {e}");
        }
        let head = "schema: goatbuilder-target/v1\ntype: acme\nfamilies: [host-class]\n";
        for (body, says) in [
            ("phases:\n  - builtin: make-tea\n", "'make-tea'"),
            ("phases:\n  - builtin: mesh-dns\n", "works on the site's core node"),
            ("site: {declared_by: has_acme, standup: phase-14}\nphases:\n  - builtin: utility-play-present\n", "plays and declared skips only"),
            ("converges_as: ghost\n", "'ghost'"),
            ("converges_as: core\n", "another group"),
            ("converges_as: phone\n", "itself converges as"),
            ("phases: [\n", "not valid"),
        ] {
            let (c, e) = refused(&format!("{head}{body}"));
            assert!(c == "GB-374" && e.contains(says), "{body}: {c} {e}");
        }
    }

    #[test]
    fn a_machine_of_a_family_its_type_does_not_accept_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let targets = load(t.path()).unwrap();
        let core = targets.require("core").unwrap();
        let check = || targets.check_family(t.path(), core, "zagros", "node-x");
        assert!(!any_family_declared(t.path()));
        check().expect("a machine that declares none is not checked");
        put(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "platform_family: phone\n");
        assert!(any_family_declared(t.path()));
        let e = check().unwrap_err();
        assert!(code(&e) == "GB-376" && e.to_string().contains("group_vars/zagros_hosts/vars.yml"), "{e}");
        put(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nnode-x ansible_host=192.0.2.1 platform_family=small-computer # a Pi\n");
        let e = check().unwrap_err();
        assert!(code(&e) == "GB-376" && e.to_string().contains("small-computer") && e.to_string().contains("mgmt-hosts.ini"), "{e}");
        put(t.path(), "ops/ansible/host_vars/node-x.yml", "platform_family: host-class\n");
        check().expect("host_vars come first");
        put(t.path(), "ops/ansible/host_vars/node-x.yml", "platform_family: mainframe\n");
        assert_eq!(code(&check().unwrap_err()), "GB-375");
        put(t.path(), "ops/ansible/host_vars/node-x.yml", "platform_family: [a\n");
        let e = check().unwrap_err();
        assert!(code(&e) == "GB-374" && e.to_string().contains("where a machine's platform_family is read from"), "{e}");
    }

    /// The family is read as Ansible resolves the variable: the last file of a
    /// vars directory wins, the type's own inventory over the base and the
    /// transport's override over both,
    /// group_vars/all and `[group:vars]` below the group's own files.
    #[test]
    fn the_family_is_read_in_ansibles_order() {
        let t = tempfile::tempdir().unwrap();
        let targets = load(t.path()).unwrap();
        let bg = targets.require("breakglass").unwrap();
        let core = targets.require("core").unwrap();
        let found = |ty: &TargetType| declared_family(t.path(), ty, "zagros", "node-x").unwrap();
        put(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[all:vars]\nplatform_family=cloud\n[zagros_hosts]\nnode-x\n");
        assert_eq!(found(core).unwrap().0, "cloud", "[all:vars]");
        put(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts:vars] # the site\nplatform_family=host-class\n[all:vars]\nplatform_family=cloud\n[zagros_hosts]\nnode-x\n");
        assert_eq!(found(core).unwrap().0, "host-class", "a group's own vars over all's, its header with a comment");
        put(t.path(), "ops/ansible/inventory/mgmt-hosts-mesh.ini", "[zagros_hosts]\nnode-x platform_family=host-class\nnode-x platform_family=phone\n");
        assert_eq!(found(core).unwrap().0, "phone", "a host's later line in one inventory wins");
        std::fs::remove_file(t.path().join("ops/ansible/inventory/mgmt-hosts-mesh.ini")).unwrap();
        put(t.path(), "ops/ansible/group_vars/all.yml", "platform_family: phone\n");
        assert_eq!(found(core).unwrap(), ("phone".to_string(), "ops/ansible/group_vars/all.yml".to_string()), "group_vars/all over the inventory's group vars");
        put(t.path(), "ops/ansible/group_vars/zagros_hosts/00-defaults.yml", "platform_family: phone\n");
        put(t.path(), "ops/ansible/group_vars/zagros_hosts/zz-site.yml", "platform_family: host-class\n");
        assert_eq!(found(core).unwrap().1, "ops/ansible/group_vars/zagros_hosts/zz-site.yml", "the last file of a directory wins");
        put(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_breakglass_hosts]\nnode-x platform_family=host-class\n");
        put(t.path(), "ops/ansible/inventory/zagros-breakglass-nodes.ini", "[zagros_breakglass_hosts]\nnode-x platform_family=small-computer\n");
        assert_eq!(found(bg).unwrap().0, "small-computer", "the type's own inventory is passed after the base, so it wins over it");
        put(t.path(), "ops/ansible/inventory/mgmt-hosts-mesh.ini", "[zagros_breakglass_hosts]\nnode-x platform_family=host-class\n");
        assert_eq!(found(bg).unwrap().0, "host-class", "the transport's override is passed last, so it wins over the type's own");
        std::fs::remove_file(t.path().join("ops/ansible/inventory/mgmt-hosts-mesh.ini")).unwrap();
        put(t.path(), "ops/ansible/host_vars/node-x.yml", "platform_family: \"{{ hw }}\"\n");
        assert_eq!(found(bg), None, "a templated value is not a family to check");
    }

    /// standup finds a phone in the core's group: it passes as a phone there,
    /// and join-node, which knows the class, holds it to its own type.
    #[test]
    fn a_machine_in_a_shared_group_is_checked_against_the_types_that_share_it() {
        let t = tempfile::tempdir().unwrap();
        let targets = load(t.path()).unwrap();
        let core = targets.require("core").unwrap();
        put(t.path(), "ops/ansible/host_vars/phone-1.yml", "platform_family: phone\n");
        targets.check_family_in_group(t.path(), core, "zagros", "phone-1", true).expect("a phone in a joining site's core group");
        assert_eq!(code(&targets.check_family_in_group(t.path(), core, "zagros", "phone-1", false).unwrap_err()), "GB-376", "a founder has no phone");
        assert_eq!(code(&targets.check_family(t.path(), core, "zagros", "phone-1").unwrap_err()), "GB-376");
        put(t.path(), "ops/ansible/host_vars/pi-1.yml", "platform_family: small-computer\n");
        assert_eq!(code(&targets.check_family_in_group(t.path(), core, "zagros", "pi-1", true).unwrap_err()), "GB-376");
        // A type that sits in the core's group but does not converge as core
        // is no reason to pass a machine as core.
        put(t.path(), "ops/goatbuilder/overlay/targets/kiosk.yaml", "schema: goatbuilder-target/v1\ntype: kiosk\nfamilies: [small-computer]\ngroup: \"{site_us}_hosts\"\nphases:\n  - label: a kiosk\n    plays:\n      - {play: kiosk.yml, args: \"-e target={group}\", from: overlay}\n");
        let targets = load(t.path()).unwrap();
        let core = targets.require("core").unwrap();
        assert_eq!(code(&targets.check_family_in_group(t.path(), core, "zagros", "pi-1", true).unwrap_err()), "GB-376");
    }
}
