// SPDX-License-Identifier: Apache-2.0
//
// `goat warrant apply --membership` — warrant owns peer membership
// (docs/design/mesh-change-control.md §4, ADR 1189).
//
// The declaration is the directory `ops/warrant/membership/<scope>/`: a
// `_scope.yaml` header, fragment files of entries, and a generated per-person
// baseline (`_baseline.generated.yaml`). It says, per peer, which MANAGED
// groups it should hold. `_scope.yaml` saying `converge: enforce` is the
// switch: such a scope is governed by warrant, and the tools that used to
// write membership live (`goat admin members move`, `goat admin
// platform-admin`) print or write an entry instead (§4.5). A `converge: report`
// scope is an adoption draft: the converger reports against it and those tools
// still write live. This module is both halves: the converger, and the helpers
// those tools use to find and edit the declaration.
//
// Invariants (§4.4 — do not weaken):
//   - REPORT BY DEFAULT. `--mode enforce` is required to change anything, and
//     the file's `converge:` is the ceiling: the CLI can lower enforce to
//     report, never raise it (exit 2).
//   - UNKNOWN IS NEVER CHANGED. A peer with no baseline (declared or key) and
//     no `exact` or `hold` entry is reported `unknown` and left alone.
//   - ONLY MANAGED GROUPS. A managed group is one whose name starts with the
//     scope's prefix + `-`. `All` and every other prefix are never touched.
//   - BOUNDED. More than `--max-changes` peer-group operations → nothing is
//     changed, exit 3. A broken declaration or an empty activity log cannot
//     mass-revert a goatnet.
//   - MIRROR-FIRST. Every add lands and is re-read before any removal, and an
//     add into a group no enabled policy covers is refused (the members-move
//     coverage gate).
//   - AUDITED. One OperatorActionV1 `warrant.membership.apply` per enforce run
//     that changed something.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use panoscope_core::aclcheck::Declaration;
use panoscope_core::netbird::{NbEvent, NbGroup, NbPeer, NbUser, NetbirdClient, SetupKeyMeta};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};


/// Where the declarations live, relative to the repo root.
pub const MEMBERSHIP_DIR: &str = "ops/warrant/membership";
/// The operator doc for everything in this module.
pub const RUNBOOK: &str = "docs/operations/warrant-runbook.md §\"Membership\"";

/// Exit code: `--mode enforce` asked for on a file whose `converge:` is report.
pub const EXIT_MODE_CEILING: i32 = 2;
/// Exit code: the run would exceed `--max-changes`; nothing was changed.
pub const EXIT_MAX_CHANGES: i32 = 3;
/// Exit code: a security-quarantine transition was refused because the subject
/// is privileged, or because a person's ownership/identity privilege could not
/// be proved before changing membership. Other rows were still processed.
pub const EXIT_PRIVILEGED_QUARANTINE: i32 = 4;

/// A refusal that carries its own exit code. `main.rs` prints the message and
/// exits with the code instead of the generic 1, so a timer wrapper can tell
/// "refused by design" from "broke".
#[derive(Debug)]
pub struct WarrantExit {
    pub code: i32,
    pub msg: String,
}

impl std::fmt::Display for WarrantExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for WarrantExit {}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Report,
    Enforce,
}

#[derive(Args, Debug)]
pub struct ApplyArgs {
    /// Converge peer membership (the only kind `apply` does today).
    #[arg(long)]
    pub membership: bool,
    /// Taxonomy scope, e.g. `zagros-dev`. Must equal `_scope.yaml`'s `scope:`.
    #[arg(long)]
    pub scope: String,
    /// The declaration DIRECTORY, e.g. `ops/warrant/membership/zagros-dev`
    /// (`_scope.yaml`, fragments, `_baseline.generated.yaml`). `--dir` is an
    /// alias.
    #[arg(long, alias = "dir")]
    pub file: PathBuf,
    /// Directory holding the goatnet's netbird_acl_taxonomy.yml and
    /// netbird_acl_matrix.yml, read to check the scope's prefix. Omitted:
    /// GOAT_ACL_DECLARATION_DIR, the active context, then the checkout.
    #[arg(long, value_name = "DIR")]
    pub acl_declaration: Option<PathBuf>,
    /// `report` changes nothing. `enforce` converges, and is refused when
    /// `_scope.yaml` says `converge: report`.
    #[arg(long, value_enum, default_value = "report")]
    pub mode: Mode,
    /// Refuse the whole run, changing nothing, if it would make more
    /// peer-group changes than this.
    #[arg(long, default_value_t = 10)]
    pub max_changes: usize,
    /// Print one stable JSON object instead of the table.
    #[arg(long)]
    pub json: bool,
    /// Write `goat_warrant_membership.prom` here (node_exporter textfile dir).
    #[arg(long)]
    pub textfile_dir: Option<PathBuf>,
    /// NetBird management URL (else $GOAT_NB_MGMT_URL).
    #[arg(long)]
    pub mgmt_url: Option<String>,
    /// Where `goat warrant pause` writes `pause-<scope>.json`. While a pause
    /// is in force the run is report-only, whatever `--mode` says.
    #[arg(long, default_value = DEFAULT_STATE_DIR)]
    pub state_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// The declaration — a directory per scope (mesh-change-control.md §4.1)
// ---------------------------------------------------------------------------
//
//   ops/warrant/membership/<scope>/          (on a host /etc/goat/warrant/membership/<scope>/)
//     _scope.yaml                 version, scope, converge — required; the only
//                                 file allowed `scope:` / `converge:`
//     _baseline.generated.yaml    GENERATED per-person baseline (never hand-edited)
//     **/*.yaml                   fragments: `version: 1` + `entries: [...]`
//
// The merged view is every fragment's entries, one entry per peer across all
// fragments. Any other `_`-prefixed file is refused, as is the retired
// single-file `<scope>.yaml` layout.

/// The scope header file inside a scope directory.
pub const SCOPE_FILE: &str = "_scope.yaml";
/// The generated per-person baseline inside a scope directory.
pub const BASELINE_FILE: &str = "_baseline.generated.yaml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Converge {
    Report,
    Enforce,
}

/// `_scope.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeFile {
    version: u32,
    scope: String,
    converge: Converge,
}

/// A fragment. `deny_unknown_fields` is what refuses `scope:` / `converge:`
/// anywhere but `_scope.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FragmentFile {
    version: u32,
    #[serde(default)]
    entries: Option<Vec<MembershipEntry>>,
}

/// One person's declared baseline in `_baseline.generated.yaml`: the full
/// managed group names a device peer and a handset peer of theirs should hold.
/// `email_hash` is populated from the top-level compatibility map after decode;
/// keeping it out of the person row lets the previous strict reader accept the
/// new file during a rolling upgrade.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonBaseline {
    /// SHA-256 of the person's lower-cased login email. This is the same
    /// privacy-preserving join key as the sanitized roster. It lets the
    /// deployed converger join a peer owner to the live Ory identity without
    /// placing an email address in the declaration.
    #[serde(skip)]
    pub email_hash: String,
    #[serde(default)]
    pub device: Vec<String>,
    #[serde(default)]
    pub handset: Vec<String>,
}

const AMBIGUOUS_EMAIL_HASH: &str = "ambiguous-baseline-manifest-owner";

#[derive(Debug, Default, Deserialize)]
struct OperatorManifest {
    #[serde(default)]
    identity: OperatorManifestIdentity,
}

#[derive(Debug, Default, Deserialize)]
struct OperatorManifestIdentity {
    #[serde(default)]
    handle: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    status: String,
}

/// Load active person-owner joins from local, gitignored operator manifests.
/// These entries prove ownership only and never provide a group baseline;
/// group intent remains solely in the generated baseline or setup key.
pub(crate) fn people_from_manifests(
    dir: &Path,
) -> anyhow::Result<BTreeMap<String, PersonBaseline>> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("read operator manifest directory {}: {e}", dir.display()))?;
    let mut paths = entries
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut people = BTreeMap::new();
    for path in paths {
        if path.extension().and_then(|v| v.to_str()) != Some("yml") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read operator manifest {}: {e}", path.display()))?;
        let manifest: OperatorManifest = serde_yaml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parse operator manifest {}: {e}", path.display()))?;
        if manifest.identity.status != "active" {
            continue;
        }
        let handle = manifest.identity.handle.trim();
        if handle.is_empty() {
            anyhow::bail!(
                "active operator manifest {} has no identity.handle",
                path.display()
            );
        }
        let email = manifest.identity.email.trim();
        if email.is_empty() {
            anyhow::bail!(
                "active operator manifest {} has no identity.email",
                path.display()
            );
        }
        if people
            .insert(
                handle.to_string(),
                PersonBaseline {
                    email_hash: email_hash(email),
                    device: Vec::new(),
                    handset: Vec::new(),
                },
            )
            .is_some()
        {
            anyhow::bail!(
                "active operator manifest {} duplicates identity.handle `{handle}`",
                path.display()
            );
        }
    }
    Ok(people)
}

/// The generated baseline owns group intent. Local manifests are ownership
/// joins only and never group baselines: they fill handles absent from the
/// generated file, while a disagreement about an existing owner is made
/// explicitly ambiguous so the quarantine guard fails closed.
pub(crate) fn merge_people(
    mut baseline: BTreeMap<String, PersonBaseline>,
    manifests: BTreeMap<String, PersonBaseline>,
) -> BTreeMap<String, PersonBaseline> {
    for (handle, manifest) in manifests {
        match baseline.get_mut(&handle) {
            Some(person)
                if !person.email_hash.is_empty() && person.email_hash != manifest.email_hash =>
            {
                person.email_hash = AMBIGUOUS_EMAIL_HASH.to_string();
            }
            Some(_) => {}
            None => {
                baseline.insert(handle, manifest);
            }
        }
    }
    baseline
}

/// Live Ory privilege indexed by the privacy-preserving email hash carried in
/// the generated baseline. More than one row for a hash is kept deliberately:
/// a duplicate identity is ambiguous and must fail closed.
#[derive(Debug, Clone, Default)]
pub(crate) struct OryPrivilegeIndex {
    by_email_hash: BTreeMap<String, Vec<BTreeSet<String>>>,
}

impl OryPrivilegeIndex {
    pub(crate) fn groups_for(&self, email_hash: &str) -> anyhow::Result<&BTreeSet<String>> {
        match self.by_email_hash.get(email_hash).map(Vec::as_slice) {
            Some([groups]) => Ok(groups),
            Some(rows) => anyhow::bail!(
                "the owner join matched {} Ory identities; ownership is ambiguous",
                rows.len()
            ),
            None => anyhow::bail!("the owner join matched no live Ory identity"),
        }
    }

    #[cfg(test)]
    pub(crate) fn one(email: &str, groups: &[&str]) -> Self {
        Self {
            by_email_hash: BTreeMap::from([(
                email_hash(email),
                vec![groups.iter().map(|g| (*g).to_string()).collect()],
            )]),
        }
    }
}

#[derive(Default, Deserialize)]
struct OryIdentityTraits {
    email: Option<String>,
    groups: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
struct OryIdentityMetadata {
    groups: Option<Vec<String>>,
}

/// Deliberately narrow Kratos projection. Recovery addresses and credential
/// metadata are discarded by serde during decoding and never enter the model.
#[derive(Deserialize)]
struct OryIdentityProjection {
    #[serde(default)]
    traits: OryIdentityTraits,
    #[serde(default)]
    metadata_public: Option<OryIdentityMetadata>,
}

pub(crate) fn email_hash(email: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(email.trim().to_lowercase().as_bytes())
    )
}

fn absorb_ory_page(index: &mut OryPrivilegeIndex, body: &str) -> anyhow::Result<usize> {
    let identities: Vec<OryIdentityProjection> = serde_json::from_str(body)
        .map_err(|e| anyhow::anyhow!("decode Ory identity lookup: {e}"))?;
    let count = identities.len();
    for identity in identities {
        let Some(email) = identity
            .traits
            .email
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        else {
            continue;
        };
        let groups = identity
            .metadata_public
            .and_then(|m| m.groups)
            .or(identity.traits.groups)
            .unwrap_or_default()
            .into_iter()
            .collect();
        index
            .by_email_hash
            .entry(email_hash(email))
            .or_default()
            .push(groups);
    }
    Ok(count)
}

/// Read every Ory identity once and retain only the email hash and login-group
/// set needed by the quarantine gate. Raw identity data never reaches output.
pub(crate) async fn read_ory_privileges(
    cfg: &super::super::admin::OryEnvConfig,
) -> anyhow::Result<OryPrivilegeIndex> {
    const PAGE_SIZE: usize = 250;
    const SAFETY_CAP: usize = 5000;
    let mut out = OryPrivilegeIndex::default();
    let mut total = 0usize;
    for page in 0.. {
        let query = format!(
            "curl -sS --max-time 60 -w '\\n%{{http_code}}' \
             '{}/admin/identities?page={page}&per_page={PAGE_SIZE}'",
            cfg.kratos_admin_url
        );
        let (code, body) = super::super::admin::run_ssh_curl(cfg, &query, None).await?;
        if code != 200 {
            anyhow::bail!("Ory identity lookup page {page} returned HTTP {code}");
        }
        let count = absorb_ory_page(&mut out, &body)?;
        total += count;
        if count < PAGE_SIZE {
            return Ok(out);
        }
        if total >= SAFETY_CAP {
            anyhow::bail!(
                "Ory identity lookup reached its {SAFETY_CAP}-row safety limit; completeness \
                 cannot be proved"
            );
        }
    }
    unreachable!()
}

/// `_baseline.generated.yaml`. `identity_hashes` is deliberately top-level:
/// the previous reader did not deny unknown fields here, so old binaries accept
/// a new declaration while new binaries also accept the old declaration.
#[derive(Debug, Clone, Deserialize)]
struct BaselineFile {
    version: u32,
    scope: String,
    #[serde(default)]
    identity_hashes: BTreeMap<String, String>,
    #[serde(default)]
    people: Option<BTreeMap<String, PersonBaseline>>,
}

/// A scope's merged declaration.
#[derive(Debug, Clone)]
pub struct MembershipDecl {
    pub scope: String,
    pub converge: Converge,
    pub entries: Option<Vec<MembershipEntry>>,
    /// Handle → declared baseline, from `_baseline.generated.yaml` (empty when
    /// the file is absent).
    pub people: BTreeMap<String, PersonBaseline>,
    /// Peer → the fragment (relative to the scope directory) holding its entry.
    pub sources: BTreeMap<String, PathBuf>,
}

impl MembershipDecl {
    pub fn entries(&self) -> &[MembershipEntry] {
        self.entries.as_deref().unwrap_or(&[])
    }
}

/// One `entries:` row. Unknown fields are refused: a typo such as `grants:`
/// would otherwise parse as an entry with no form and silently mean "baseline".
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipEntry {
    pub peer: String,
    #[serde(default)]
    pub exact: Option<Vec<String>>,
    #[serde(default)]
    pub grant: Option<Vec<String>>,
    #[serde(default)]
    pub absent: Option<Vec<String>>,
    #[serde(default)]
    pub hold: Option<String>,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub expires: Option<String>,
}

/// The three shapes an entry can take (§4.1: exactly one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryForm {
    Hold,
    Exact(BTreeSet<String>),
    GrantAbsent {
        grant: BTreeSet<String>,
        absent: BTreeSet<String>,
    },
}

fn set_of(v: &Option<Vec<String>>) -> BTreeSet<String> {
    v.iter().flatten().cloned().collect()
}

fn vec_or_none(s: BTreeSet<String>) -> Option<Vec<String>> {
    if s.is_empty() {
        None
    } else {
        Some(s.into_iter().collect())
    }
}

impl MembershipEntry {
    pub fn form(&self) -> anyhow::Result<EntryForm> {
        let has_ga = self.grant.is_some() || self.absent.is_some();
        match (self.hold.as_deref(), self.exact.is_some(), has_ga) {
            (Some("quarantine"), false, false) => Ok(EntryForm::Hold),
            (Some(other), false, false) => anyhow::bail!(
                "entry `{}`: hold `{other}` is not a hold this converger knows (only `quarantine`)",
                self.peer
            ),
            (None, true, false) => Ok(EntryForm::Exact(set_of(&self.exact))),
            (None, false, true) => Ok(EntryForm::GrantAbsent {
                grant: set_of(&self.grant),
                absent: set_of(&self.absent),
            }),
            _ => anyhow::bail!(
                "entry `{}` must carry exactly one of: grant/absent, exact, hold",
                self.peer
            ),
        }
    }

    /// Build an entry from a form, filling the list fields accordingly.
    pub fn with_form(
        peer: &str,
        form: EntryForm,
        owner: &str,
        reason: &str,
        expires: Option<String>,
    ) -> Self {
        let mut e = MembershipEntry {
            peer: peer.to_string(),
            owner: owner.to_string(),
            reason: reason.to_string(),
            expires,
            ..Default::default()
        };
        match form {
            EntryForm::Hold => e.hold = Some("quarantine".into()),
            EntryForm::Exact(g) => e.exact = Some(g.into_iter().collect()),
            EntryForm::GrantAbsent { grant, absent } => {
                e.grant = vec_or_none(grant);
                e.absent = vec_or_none(absent);
            }
        }
        e
    }

    fn groups_named(&self) -> impl Iterator<Item = &String> {
        self.exact
            .iter()
            .flatten()
            .chain(self.grant.iter().flatten())
            .chain(self.absent.iter().flatten())
    }
}

/// The managed-group prefix for a scope.
///
/// ASSUMPTION: a scope's NetBird `site_prefix` is its scope id upper-cased
/// (`zagros-dev` → `ZAGROS-DEV`). That holds for every scope in
/// `netbird_acl_taxonomy.yml` today. `run_apply` cross-checks it against the
/// compiled-in taxonomy and refuses to run if the two disagree, so a scope
/// that breaks the convention fails loudly instead of managing the wrong
/// groups.
pub fn scope_prefix(scope: &str) -> String {
    scope.trim().to_uppercase()
}

/// Per-entry checks: non-empty peer, owner and reason, one form, every group
/// carrying the scope's prefix. `seen` collects peers so a duplicate inside
/// one list is refused.
fn check_entries(entries: &[MembershipEntry], scope: &str) -> anyhow::Result<()> {
    let prefix = format!("{}-", scope_prefix(scope));
    let mut seen = BTreeSet::new();
    for e in entries {
        if e.peer.trim().is_empty() {
            anyhow::bail!("an entry has an empty `peer:`");
        }
        if !seen.insert(e.peer.clone()) {
            anyhow::bail!("peer `{}` has more than one entry", e.peer);
        }
        if e.owner.trim().is_empty() || e.reason.trim().is_empty() {
            anyhow::bail!("entry `{}` needs both `owner:` and `reason:`", e.peer);
        }
        e.form()?;
        if let Some(g) = e.groups_named().find(|g| !g.starts_with(&prefix)) {
            anyhow::bail!(
                "entry `{}` names `{g}`, which is not a managed group of scope `{scope}` \
                 (managed groups start with `{prefix}`)",
                e.peer
            );
        }
    }
    Ok(())
}

/// Parse and check one fragment: `version: 1`, `entries:`, nothing else. The
/// full CI validation (taxonomy membership, expiry rules, EXTERNAL-INFRA and
/// PLATFORM-ADMIN placement) is `ops/warrant/membership.py --check`; this is
/// what the converger needs to not misbehave.
pub fn parse_fragment(text: &str, scope: &str) -> anyhow::Result<Vec<MembershipEntry>> {
    let f: FragmentFile = serde_yaml::from_str(text).map_err(|e| {
        anyhow::anyhow!(
            "does not parse as a fragment (`version: 1` + `entries:`; only {SCOPE_FILE} may \
             carry `scope:`/`converge:`): {e}"
        )
    })?;
    if f.version != 1 {
        anyhow::bail!("fragment version {} — this goat reads version 1", f.version);
    }
    let entries = f.entries.unwrap_or_default();
    check_entries(&entries, scope)?;
    Ok(entries)
}

fn parse_scope_file(text: &str, scope: &str) -> anyhow::Result<ScopeFile> {
    let s: ScopeFile = serde_yaml::from_str(text).map_err(|e| {
        anyhow::anyhow!("{SCOPE_FILE} does not parse (version, scope, converge only): {e}")
    })?;
    if s.version != 1 {
        anyhow::bail!("{SCOPE_FILE} version {} — this goat reads version 1", s.version);
    }
    if s.scope != scope {
        anyhow::bail!(
            "membership declaration is for scope `{}`, not `{scope}` — refusing to apply it here",
            s.scope
        );
    }
    Ok(s)
}

fn parse_baseline_file(
    text: &str,
    scope: &str,
) -> anyhow::Result<BTreeMap<String, PersonBaseline>> {
    let b: BaselineFile = serde_yaml::from_str(text)
        .map_err(|e| anyhow::anyhow!("{BASELINE_FILE} does not parse: {e}"))?;
    if b.version != 1 {
        anyhow::bail!("{BASELINE_FILE} version {} — this goat reads version 1", b.version);
    }
    if b.scope != scope {
        anyhow::bail!(
            "{BASELINE_FILE} is for scope `{}`, not `{scope}`",
            b.scope
        );
    }
    let prefix = format!("{}-", scope_prefix(scope));
    let mut people = b.people.unwrap_or_default();
    for (h, hash) in &b.identity_hashes {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            anyhow::bail!(
                "{BASELINE_FILE}: identity_hashes.{h} must be a 64-character lowercase SHA-256 hash"
            );
        }
        let Some(person) = people.get_mut(h) else {
            anyhow::bail!(
                "{BASELINE_FILE}: identity_hashes names `{h}`, which has no people entry"
            );
        };
        person.email_hash.clone_from(hash);
    }
    for (h, p) in &people {
        if let Some(g) = p
            .device
            .iter()
            .chain(p.handset.iter())
            .find(|g| !g.starts_with(&prefix))
        {
            anyhow::bail!(
                "{BASELINE_FILE}: person `{h}` names `{g}`, which is not a managed group of \
                 scope `{scope}` (managed groups start with `{prefix}`)"
            );
        }
    }
    Ok(people)
}

/// Every regular file under `dir`, recursively, as paths relative to `dir`,
/// sorted. Dot-files (editor swap files, a writer's temp file) are skipped.
fn walk(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let here = dir.join(rel);
    let rd =
        std::fs::read_dir(&here).map_err(|e| anyhow::anyhow!("read {}: {e}", here.display()))?;
    let mut items: Vec<_> = rd.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for e in items {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let r = rel.join(name);
        let p = dir.join(&r);
        if p.is_dir() {
            walk(dir, &r, out)?;
        } else if p.is_file() {
            out.push(r);
        }
    }
    Ok(())
}

fn retired_layout(file: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "{} is the retired single-file layout. A scope's declaration is now the directory \
         ops/warrant/membership/<scope>/ (on a host /etc/goat/warrant/membership/<scope>/): \
         {SCOPE_FILE} (version, scope, converge), fragment files `*.yaml` with `entries:`, and \
         the generated {BASELINE_FILE} (mesh-change-control.md §4.1). Pass the directory.",
        file.display()
    )
}

/// Load and check a scope directory: `_scope.yaml`, the optional generated
/// baseline, and every fragment, merged. One entry per peer ACROSS fragments;
/// a duplicate names both files.
pub fn load_decl(dir: &Path, scope: &str) -> anyhow::Result<MembershipDecl> {
    if dir.is_file() {
        return Err(retired_layout(dir));
    }
    if !dir.is_dir() {
        let legacy = dir.with_extension("yaml");
        if legacy.is_file() {
            return Err(retired_layout(&legacy));
        }
        anyhow::bail!(
            "membership declaration directory {} does not exist",
            dir.display()
        );
    }
    let scope_path = dir.join(SCOPE_FILE);
    let text = std::fs::read_to_string(&scope_path).map_err(|e| {
        anyhow::anyhow!(
            "read {}: {e} — every scope directory needs {SCOPE_FILE}",
            scope_path.display()
        )
    })?;
    let head = parse_scope_file(&text, scope)?;

    let mut files = Vec::new();
    walk(dir, Path::new(""), &mut files)?;
    let mut people = BTreeMap::new();
    let mut entries: Vec<MembershipEntry> = Vec::new();
    let mut sources: BTreeMap<String, PathBuf> = BTreeMap::new();
    for rel in files {
        let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let top = rel.parent().map(|p| p.as_os_str().is_empty()).unwrap_or(true);
        if top && name == SCOPE_FILE {
            continue;
        }
        if top && name == BASELINE_FILE {
            let p = dir.join(&rel);
            let t = std::fs::read_to_string(&p)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
            people = parse_baseline_file(&t, scope)?;
            continue;
        }
        if name.starts_with('_') {
            anyhow::bail!(
                "{}: `_`-prefixed files other than {SCOPE_FILE} and {BASELINE_FILE} (at the top of \
                 the scope directory) are refused — rename it or remove it",
                dir.join(&rel).display()
            );
        }
        if name.ends_with(".yml") {
            anyhow::bail!(
                "{}: fragments are `*.yaml`; a `.yml` file would be silently skipped — rename it",
                dir.join(&rel).display()
            );
        }
        if !name.ends_with(".yaml") {
            continue;
        }
        let p = dir.join(&rel);
        let t =
            std::fs::read_to_string(&p).map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
        let got = parse_fragment(&t, scope).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
        for e in got {
            if let Some(first) = sources.get(&e.peer) {
                anyhow::bail!(
                    "peer `{}` has an entry in both {} and {} — one entry per peer across the \
                     scope directory",
                    e.peer,
                    first.display(),
                    rel.display()
                );
            }
            sources.insert(e.peer.clone(), rel.clone());
            entries.push(e);
        }
    }
    Ok(MembershipDecl {
        scope: head.scope,
        converge: head.converge,
        entries: Some(entries),
        people,
        sources,
    })
}

// ---------------------------------------------------------------------------
// Governed scopes, and editing the declaration
// ---------------------------------------------------------------------------

/// `<repo root>/ops/warrant/membership/<scope>` (the scope DIRECTORY), repo
/// root found by walking up from the cwd (the same discovery operator
/// manifests use). None outside a checkout.
pub fn membership_path(scope: &str) -> Option<PathBuf> {
    membership_root().map(|r| r.join(scope))
}

/// `<repo root>/ops/warrant/membership`, whether or not it exists. None
/// outside a checkout.
fn membership_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let root = super::super::admin::discover_repo_root(&cwd)?;
    Some(root.join(MEMBERSHIP_DIR))
}

/// What a scope's `converge:` says, as far as governance goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileConverge {
    /// An adoption draft: the converger reports, the old tools still write.
    Report,
    /// Governed: warrant owns membership on the scope.
    Enforce,
    /// A declaration exists but its `converge:` cannot be read (no
    /// `_scope.yaml`, a broken one, or the retired single file). Treated as
    /// governed — refusing a live write is the safe side of a broken file.
    Unreadable,
}

fn converge_word(v: &str) -> FileConverge {
    match v.trim() {
        "report" => FileConverge::Report,
        "enforce" => FileConverge::Enforce,
        _ => FileConverge::Unreadable,
    }
}

fn converge_of_text(text: &str) -> FileConverge {
    serde_yaml::from_str::<serde_yaml::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("converge")
                .and_then(|c| c.as_str())
                .map(converge_word)
        })
        .unwrap_or(FileConverge::Unreadable)
}

/// The `converge:` of the scope directory at `dir` (its `_scope.yaml`); None
/// when there is no declaration there at all. A retired single file
/// `<scope>.yaml` beside it counts as a declaration that cannot be read.
fn repo_converge(dir: &Path) -> Option<FileConverge> {
    if dir.is_dir() {
        return Some(
            std::fs::read_to_string(dir.join(SCOPE_FILE))
                .map(|t| converge_of_text(&t))
                .unwrap_or(FileConverge::Unreadable),
        );
    }
    if dir.is_file() || dir.with_extension("yaml").is_file() {
        return Some(FileConverge::Unreadable);
    }
    None
}

/// One source's answer to "is this scope's membership declared?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeAnswer {
    /// The source knows this goatnet and it has no membership declaration:
    /// the audited tools write live.
    Undeclared,
    /// A declaration, with what its `converge:` says.
    Declared(FileConverge),
}

/// Where the governance answer came from, for the message an operator reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernanceSource {
    /// `ops/warrant/membership/` in the current checkout.
    Checkout,
    /// `warrant_membership_converge` in the active context's profile.toml.
    Context,
    /// The goatnet's NetBird account: a warrant service user, or none.
    Live,
}

/// Is this scope's membership governed by warrant, and which source said so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Governance {
    pub governed: bool,
    pub source: GovernanceSource,
}

impl Governance {
    /// One line naming the source, for a refusal or a dry-run.
    pub fn because(&self) -> &'static str {
        match self.source {
            GovernanceSource::Checkout => {
                "decided by this checkout's ops/warrant/membership/<scope>/_scope.yaml \
                 (no directory = no declaration)"
            }
            GovernanceSource::Context => {
                "decided by `warrant_membership_converge` in the context's profile.toml"
            }
            GovernanceSource::Live => {
                "decided by the goatnet's NetBird account: a warrant service user \
                 (svc-<goatnet>-warrant) = governed, none = not; run from a checkout that \
                 carries ops/warrant/membership/ or set `warrant_membership_converge` in the \
                 context to decide it from the declaration"
            }
        }
    }
}

/// The checkout's answer: only a checkout that carries `ops/warrant/membership/`
/// knows the goatnets' declarations, and then a scope with no directory there
/// has none. Any other checkout (the public tree, an unrelated repository)
/// does not answer.
fn checkout_answer(membership_root: Option<&Path>, scope: &str) -> Option<ScopeAnswer> {
    let root = membership_root.filter(|r| r.is_dir())?;
    Some(match repo_converge(&root.join(scope)) {
        None => ScopeAnswer::Undeclared,
        Some(c) => ScopeAnswer::Declared(c),
    })
}

/// The profile.toml key that carries a goatnet's `converge:` with its context.
pub const CONTEXT_CONVERGE_KEY: &str = "warrant_membership_converge";

/// The context's answer, from `warrant_membership_converge`: `enforce` and
/// `report` as in `_scope.yaml`, `none` for a goatnet with no declaration.
/// Any other value is a declaration that cannot be read (governed). It answers
/// only for the goatnet the context is bound to: its `goatnet` field, or, with
/// none, the context's own name (the binding `require_env_binding` checks).
fn context_answer(
    value: Option<&str>,
    bound_goatnet: Option<&str>,
    context_name: &str,
    scope: &str,
) -> Option<ScopeAnswer> {
    let bound = match bound_goatnet.map(str::trim).filter(|g| !g.is_empty()) {
        Some(g) => g == scope,
        None => context_name == scope,
    };
    let v = value.map(str::trim).filter(|v| !v.is_empty())?;
    if !bound {
        return None;
    }
    Some(match v {
        "none" => ScopeAnswer::Undeclared,
        v => ScopeAnswer::Declared(converge_word(v)),
    })
}

/// A warrant converger's NetBird service user: `svc-<goatnet>-warrant`
/// (ops/netbird/service-identities.yaml `warrant-membership`). One NetBird
/// account is one goatnet, so any such user means warrant runs here.
fn is_warrant_service_user(u: &NbUser) -> bool {
    let n = u.name.trim();
    n.len() > "svc--warrant".len() && n.starts_with("svc-") && n.ends_with("-warrant")
}

/// The goatnet's own answer, when no checkout and no context knew: an account
/// with a warrant service user runs a converger whose mode nothing here can
/// read, so it is governed (fail closed); an account without one has no
/// converger, so the tools write live as they always have; an account that
/// could not be read is governed.
fn live_answer<E>(users: Result<&[NbUser], E>) -> ScopeAnswer {
    match users {
        Ok(us) if !us.iter().any(is_warrant_service_user) => ScopeAnswer::Undeclared,
        _ => ScopeAnswer::Declared(FileConverge::Unreadable),
    }
}

fn governed_by(answer: ScopeAnswer) -> bool {
    match answer {
        ScopeAnswer::Undeclared | ScopeAnswer::Declared(FileConverge::Report) => false,
        ScopeAnswer::Declared(FileConverge::Enforce | FileConverge::Unreadable) => true,
    }
}

/// Pure half of [`governance`]: the first source that answers decides —
/// the checkout, then the context, then the goatnet's NetBird account (`live`
/// is read only when neither of the others answered; None there = not read,
/// which is governed).
fn decide(
    checkout: Option<ScopeAnswer>,
    context: Option<ScopeAnswer>,
    live: Option<ScopeAnswer>,
) -> Governance {
    let (answer, source) = checkout
        .map(|a| (a, GovernanceSource::Checkout))
        .or_else(|| context.map(|a| (a, GovernanceSource::Context)))
        .unwrap_or((
            live.unwrap_or(ScopeAnswer::Declared(FileConverge::Unreadable)),
            GovernanceSource::Live,
        ));
    Governance {
        governed: governed_by(answer),
        source,
    }
}

/// Is this scope's membership governed by warrant (mesh-change-control.md
/// §4.1)? Governed when its declaration says `converge: enforce`; a
/// declaration that cannot be read counts as governed. A `converge: report`
/// scope is an adoption draft: the converger reports against it, and
/// `members move --apply` / `platform-admin` still write live, as on an
/// ungoverned scope.
///
/// Read at run time, never compiled in, so the binary carries no goatnet's
/// data. The first source that answers decides:
///   1. a checkout that carries `ops/warrant/membership/` — the scope's
///      `_scope.yaml`, or no directory = no declaration;
///   2. the active context's `warrant_membership_converge`
///      (`enforce` | `report` | `none`), for the goatnet it is bound to;
///   3. `nb`, the NetBird account the command is about to write: a warrant
///      service user there = governed, none = not governed, unreadable =
///      governed.
pub async fn governance(
    scope: &str,
    profile: &crate::profile::Profile,
    nb: &NetbirdClient,
) -> Governance {
    let checkout = checkout_answer(membership_root().as_deref(), scope);
    let context = context_answer(
        profile.profile_field(CONTEXT_CONVERGE_KEY).as_deref(),
        profile.profile_field("goatnet").as_deref(),
        &profile.context,
        scope,
    );
    let live = if checkout.is_none() && context.is_none() {
        Some(live_answer(nb.list_users().await.as_deref()))
    } else {
        None
    };
    decide(checkout, context, live)
}

fn yaml_scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '/'))
        && !matches!(
            s,
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
        );
    if plain {
        s.to_string()
    } else {
        // A JSON string is a valid YAML double-quoted scalar.
        serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
    }
}

/// Render one entry as the YAML list item that goes under `entries:`.
pub fn entry_yaml(e: &MembershipEntry, indent: &str) -> String {
    let list = |v: &[String]| {
        format!(
            "[{}]",
            v.iter()
                .map(|g| yaml_scalar(g))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut out = format!("{indent}- peer: {}\n", yaml_scalar(&e.peer));
    let sub = format!("{indent}  ");
    if let Some(h) = &e.hold {
        out.push_str(&format!("{sub}hold: {}\n", yaml_scalar(h)));
    }
    if let Some(x) = &e.exact {
        out.push_str(&format!("{sub}exact: {}\n", list(x)));
    }
    if let Some(g) = &e.grant {
        out.push_str(&format!("{sub}grant: {}\n", list(g)));
    }
    if let Some(a) = &e.absent {
        out.push_str(&format!("{sub}absent: {}\n", list(a)));
    }
    out.push_str(&format!("{sub}owner: {}\n", yaml_scalar(&e.owner)));
    out.push_str(&format!(
        "{sub}reason: {}\n",
        serde_json::to_string(&e.reason).unwrap_or_default()
    ));
    if let Some(x) = &e.expires {
        out.push_str(&format!("{sub}expires: {}\n", yaml_scalar(x)));
    }
    out
}

/// Fold a new entry into an existing one for the same peer.
///
/// - a `hold` or an `exact` replaces whatever was there;
/// - grant/absent onto grant/absent: the new grants join the old ones and
///   leave `absent`, the new absents join the old ones and leave `grant`;
/// - grant/absent onto `exact`: the exact list is edited, and stays exact;
/// - grant/absent onto a `hold`: replaces it (moving a peer out of quarantine
///   ends the hold).
pub fn merge_entry(
    existing: Option<&MembershipEntry>,
    new: MembershipEntry,
) -> anyhow::Result<MembershipEntry> {
    let Some(old) = existing else { return Ok(new) };
    let new_form = new.form()?;
    let old_form = old.form()?;
    let expires = new.expires.clone().or_else(|| old.expires.clone());
    let merged = match (old_form, new_form) {
        (_, f @ (EntryForm::Hold | EntryForm::Exact(_))) => f,
        (EntryForm::Hold, f) => f,
        (EntryForm::Exact(mut x), EntryForm::GrantAbsent { grant, absent }) => {
            x.extend(grant);
            x.retain(|g| !absent.contains(g));
            EntryForm::Exact(x)
        }
        (
            EntryForm::GrantAbsent {
                grant: og,
                absent: oa,
            },
            EntryForm::GrantAbsent {
                grant: ng,
                absent: na,
            },
        ) => {
            let grant: BTreeSet<String> = og
                .union(&ng)
                .filter(|g| !na.contains(*g))
                .cloned()
                .collect();
            let absent: BTreeSet<String> = oa
                .union(&na)
                .filter(|g| !ng.contains(*g))
                .cloned()
                .collect();
            EntryForm::GrantAbsent { grant, absent }
        }
    };
    Ok(MembershipEntry::with_form(
        &new.peer,
        merged,
        &new.owner,
        &new.reason,
        expires,
    ))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn peer_of_item(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("- peer:")?;
    let v = rest.split(" #").next().unwrap_or("").trim();
    Some(v.trim_matches('"').trim_matches('\'').to_string())
}

/// Text-level edit that keeps every comment and every other entry byte for
/// byte: replace this peer's list item if there is one, else append a new
/// item under `entries:`. Pure; the caller re-parses the result.
fn splice_entry(text: &str, entry: &MembershipEntry) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if let Some(i) = lines
        .iter()
        .position(|l| peer_of_item(l).as_deref() == Some(entry.peer.as_str()))
    {
        let ind = indent_of(lines[i]);
        let mut j = i + 1;
        while j < lines.len() {
            let l = lines[j];
            if !l.trim().is_empty() && indent_of(l) <= ind {
                break;
            }
            j += 1;
        }
        // Leave trailing blank lines to separate the next item.
        while j > i + 1 && lines[j - 1].trim().is_empty() {
            j -= 1;
        }
        let rendered = entry_yaml(entry, &" ".repeat(ind));
        let mut out: Vec<String> = lines[..i].iter().map(|s| s.to_string()).collect();
        out.extend(rendered.lines().map(String::from));
        out.extend(lines[j..].iter().map(|s| s.to_string()));
        return out.join("\n") + "\n";
    }
    let item_indent = lines
        .iter()
        .find(|l| peer_of_item(l).is_some())
        .map(|l| indent_of(l))
        .unwrap_or(2);
    let mut out: Vec<String> = lines
        .iter()
        .map(|l| {
            let t = l.trim_end();
            if t.starts_with("entries:")
                && t.trim_start_matches("entries:").trim().starts_with("[]")
            {
                "entries:".to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }
    out.extend(
        entry_yaml(entry, &" ".repeat(item_indent))
            .lines()
            .map(String::from),
    );
    out.join("\n") + "\n"
}

/// Designated writer target for hold entries (quarantine, probation).
pub const HOLDS_FILE: &str = "holds.yaml";
/// Designated writer target for ADR 1178 service hosts (exact EXTERNAL-INFRA).
pub const SERVICES_FILE: &str = "services.yaml";
/// Designated writer target for person grant/absent entries
/// (`members move --write-entry`, `platform-admin --write-entry`).
pub const PEOPLE_FILE: &str = "people.yaml";
/// Hand-maintained INFRA entries. No tool writes it; named so the four
/// targets live in one place.
#[allow(dead_code)]
pub const SUBSTRATE_FILE: &str = "substrate.yaml";

/// The fragment a NEW entry of this form goes into (§4.1 layout). A peer that
/// already has an entry is edited where it is, whatever this says.
pub fn target_fragment(form: &EntryForm, prefix: &str) -> &'static str {
    match form {
        EntryForm::Hold => HOLDS_FILE,
        EntryForm::Exact(g) if g.len() == 1 => {
            let only = g.iter().next().map(String::as_str).unwrap_or("");
            if only == format!("{prefix}-EXTERNAL-INFRA") {
                SERVICES_FILE
            } else if only.starts_with(&format!("{prefix}-PROBATION-")) {
                HOLDS_FILE
            } else {
                PEOPLE_FILE
            }
        }
        _ => PEOPLE_FILE,
    }
}

/// Add or merge `entry` into the scope directory `dir`.
///
/// The peer's entry is edited IN PLACE in whichever fragment already holds it
/// (so a writer never creates a second entry for a peer); a peer with no entry
/// goes into `target` (a path relative to `dir`, e.g. [`PEOPLE_FILE`]), which
/// is created with a `version: 1` / `entries: []` header when missing. The
/// whole directory is re-loaded and checked to hold exactly the merged entry
/// before the fragment is replaced (atomically). Returns the merged entry and
/// the file written.
pub fn write_entry(
    dir: &Path,
    scope: &str,
    target: &str,
    entry: MembershipEntry,
) -> anyhow::Result<(MembershipEntry, PathBuf)> {
    let decl = load_decl(dir, scope)?;
    let existing = decl.entries().iter().find(|e| e.peer == entry.peer);
    let rel = decl
        .sources
        .get(&entry.peer)
        .cloned()
        .unwrap_or_else(|| PathBuf::from(target));
    check_fragment_name(&rel)?;
    let path = dir.join(&rel);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => NEW_FRAGMENT.to_string(),
        Err(e) => anyhow::bail!("read {}: {e}", path.display()),
    };
    let merged = merge_entry(existing, entry)?;
    let next = splice_entry(&text, &merged);
    let check = parse_fragment(&next, scope).map_err(|e| {
        anyhow::anyhow!(
            "refusing to write {}: the edit would not parse ({e})",
            path.display()
        )
    })?;
    if check.iter().find(|e| e.peer == merged.peer) != Some(&merged) {
        anyhow::bail!(
            "refusing to write {}: after the edit the entry for `{}` does not read back as intended \
             (is `entries:` the last key in the file?)",
            path.display(),
            merged.peer
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("create {}: {e}", parent.display()))?;
    }
    write_atomic(&path, next.as_bytes())?;
    Ok((merged, path))
}

/// The header a writer gives a fragment it creates.
const NEW_FRAGMENT: &str = "version: 1\nentries: []\n";

/// A writer target must be a plain `*.yaml` fragment path inside the scope
/// directory: no `..`, not absolute, not one of the `_` files.
fn check_fragment_name(rel: &Path) -> anyhow::Result<()> {
    let ok_parts = rel
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !ok_parts || !name.ends_with(".yaml") || name.starts_with('_') || name.starts_with('.') {
        anyhow::bail!(
            "`{}` is not a fragment a writer may target (a relative `*.yaml` path, not `_`-prefixed)",
            rel.display()
        );
    }
    Ok(())
}

fn write_atomic(path: &Path, body: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("out");
    let tmp = dir.join(format!(".{name}.tmp.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| anyhow::anyhow!("create {}: {e}", tmp.display()))?;
        f.write_all(body)?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::anyhow!("rename {} -> {}: {e}", tmp.display(), path.display())
    })
}

// ---------------------------------------------------------------------------
// The plan — pure
// ---------------------------------------------------------------------------

/// A peer as the planner sees it: id, exact name, every group name it holds.
#[derive(Debug, Clone, Default)]
pub struct LivePeer {
    pub id: String,
    pub name: String,
    pub groups: Vec<String>,
}

impl From<&NbPeer> for LivePeer {
    fn from(p: &NbPeer) -> Self {
        LivePeer {
            id: p.id.clone(),
            name: p.name.clone(),
            groups: p.groups.iter().map(|g| g.name.clone()).collect(),
        }
    }
}

/// Live state of a group the plan may add into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupState {
    /// Referenced by at least one enabled rule of an enabled policy.
    pub covered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Match,
    Drift,
    /// Drift the converger will not apply: an add into a group that does not
    /// exist or that no enabled policy covers.
    Blocked,
    /// No baseline (declared or key) and no exact/hold entry — never changed.
    Unknown,
    /// An entry names a peer that does not exist.
    MissingPeer,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub peer: String,
    pub id: String,
    /// Managed groups held now.
    pub current: Vec<String>,
    /// Managed groups the peer should hold; null when unknown.
    pub desired: Option<Vec<String>>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub status: Status,
    /// Where the peer's baseline came from: `declared` (the person's row in
    /// `_baseline.generated.yaml`), `key` (the minting setup key's
    /// auto_groups), or `none`.
    pub baseline_source: BaselineSource,
    /// Groups held that this scope does not manage (`All`, other prefixes).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmanaged: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
}

impl Plan {
    fn count(&self, s: Status) -> usize {
        self.rows.iter().filter(|r| r.status == s).count()
    }
    pub fn matched(&self) -> usize {
        self.count(Status::Match)
    }
    /// Peers not holding what they should: applicable drift plus blocked.
    pub fn drift(&self) -> usize {
        self.count(Status::Drift) + self.count(Status::Blocked)
    }
    pub fn blocked(&self) -> usize {
        self.count(Status::Blocked)
    }
    pub fn unknown(&self) -> usize {
        self.count(Status::Unknown)
    }
    pub fn missing_peer(&self) -> usize {
        self.count(Status::MissingPeer)
    }
    /// Peers whose baseline came from `_baseline.generated.yaml`.
    pub fn baseline_declared(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.baseline_source == BaselineSource::Declared)
            .count()
    }
    pub fn peers_total(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.status != Status::MissingPeer)
            .count()
    }
    /// Peer-group add + remove operations the converger would make.
    pub fn changes_planned(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.status == Status::Drift)
            .map(|r| r.add.len() + r.remove.len())
            .sum()
    }
}

/// Is `name` a group this scope manages? It must carry `<prefix>-`, and must
/// not belong to another taxonomy scope whose prefix is LONGER and starts with
/// ours: on a server hosting `zagros` and `zagros-dev`, scope `zagros` must not
/// treat `ZAGROS-DEV-OBSERVER` as its own.
pub fn is_managed(name: &str, prefix: &str, excluded_prefixes: &[String]) -> bool {
    name.starts_with(&format!("{prefix}-"))
        && !excluded_prefixes
            .iter()
            .any(|x| name.starts_with(&format!("{x}-")))
}

/// Compute the plan (§4.2). `baselines` maps a peer id to its resolved
/// baseline ([`resolve_baselines`]); a peer absent from it has none.
pub fn plan_membership(
    decl: &MembershipDecl,
    prefix: &str,
    excluded_prefixes: &[String],
    peers: &[LivePeer],
    baselines: &BTreeMap<String, Baseline>,
    live_groups: &BTreeMap<String, GroupState>,
) -> anyhow::Result<Plan> {
    let managed = |g: &str| is_managed(g, prefix, excluded_prefixes);
    let by_peer: BTreeMap<&str, &MembershipEntry> = decl
        .entries()
        .iter()
        .map(|e| (e.peer.as_str(), e))
        .collect();
    let quarantined = format!("{prefix}-QUARANTINED");

    let mut rows = Vec::new();
    for p in peers {
        let current: BTreeSet<String> = p.groups.iter().filter(|g| managed(g)).cloned().collect();
        let unmanaged: Vec<String> = p.groups.iter().filter(|g| !managed(g)).cloned().collect();
        let entry = by_peer.get(p.name.as_str()).copied();
        let form = entry.map(|e| e.form()).transpose()?;
        let source = baselines
            .get(&p.id)
            .map(|b| b.source)
            .unwrap_or(BaselineSource::None);
        let baseline = baselines.get(&p.id).map(|b| {
            b.groups
                .iter()
                .filter(|g| managed(g))
                .cloned()
                .collect::<BTreeSet<_>>()
        });

        let desired: Option<BTreeSet<String>> = match (&form, baseline) {
            (Some(EntryForm::Hold), _) => Some(BTreeSet::from([quarantined.clone()])),
            (Some(EntryForm::Exact(x)), _) => Some(x.clone()),
            (Some(EntryForm::GrantAbsent { grant, absent }), Some(b)) => Some(
                b.union(grant)
                    .filter(|g| !absent.contains(*g))
                    .cloned()
                    .collect(),
            ),
            (None, Some(b)) => Some(b),
            (_, None) => None,
        };

        let mut row = Row {
            peer: p.name.clone(),
            id: p.id.clone(),
            current: current.iter().cloned().collect(),
            desired: None,
            add: Vec::new(),
            remove: Vec::new(),
            status: Status::Unknown,
            baseline_source: source,
            unmanaged,
            note: String::new(),
        };
        let Some(desired) = desired else {
            row.note = if entry.is_some() {
                "grant/absent needs a baseline; none found (not in the declared baseline, no \
                 minting key) — use exact or hold"
                    .into()
            } else {
                "no baseline: not in the declared baseline, and no minting key (key deleted, \
                 or interactive login)"
                    .into()
            };
            rows.push(row);
            continue;
        };
        row.add = desired.difference(&current).cloned().collect();
        row.remove = current.difference(&desired).cloned().collect();
        row.desired = Some(desired.into_iter().collect());
        row.status = if row.add.is_empty() && row.remove.is_empty() {
            Status::Match
        } else {
            Status::Drift
        };
        if row.status == Status::Drift {
            let bad: Vec<String> = row
                .add
                .iter()
                .filter_map(|g| match live_groups.get(g) {
                    None => Some(format!("{g} does not exist")),
                    Some(s) if !s.covered => Some(format!("{g} is covered by no enabled policy")),
                    _ => None,
                })
                .collect();
            if !bad.is_empty() {
                row.status = Status::Blocked;
                row.note = format!("refused (coverage gate): {}", bad.join("; "));
            }
        }
        rows.push(row);
    }

    let names: BTreeSet<&str> = peers.iter().map(|p| p.name.as_str()).collect();
    for e in decl.entries() {
        if !names.contains(e.peer.as_str()) {
            rows.push(Row {
                peer: e.peer.clone(),
                id: String::new(),
                current: Vec::new(),
                desired: None,
                add: Vec::new(),
                remove: Vec::new(),
                status: Status::MissingPeer,
                baseline_source: BaselineSource::None,
                unmanaged: Vec::new(),
                note: "entry names a peer that does not exist".into(),
            });
        }
    }
    Ok(Plan { rows })
}

/// Where a peer's baseline came from (§4.2 as amended: declared first, then
/// the minting key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BaselineSource {
    /// The person's `device` or `handset` list in `_baseline.generated.yaml`.
    Declared,
    /// The auto_groups of the setup key that minted the peer.
    Key,
    /// Neither: the peer is unknown unless an `exact` or `hold` entry names it.
    None,
}

/// A peer's resolved baseline: the group names it starts from, before its
/// entry (if any) is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    pub source: BaselineSource,
    pub groups: BTreeSet<String>,
}

/// What a peer's `Peer added` event says about the key that minted it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyJoin {
    /// The minting key's name: the resolved key's, else the event's
    /// `meta.setup_key_name` (kept even when the key is gone, so a handset
    /// peer is still recognised).
    pub key_name: Option<String>,
    /// The resolved key's auto_groups, by name. None when the key did not
    /// resolve (deleted, ambiguous name, or interactive login).
    pub groups: Option<BTreeSet<String>>,
}

/// Peer id → what its minting setup key says. The join is the peer's `Peer
/// added` event: its initiator is the setup key id (what
/// `check-membership-reconcile.py` uses), and its `meta.setup_key_name` names
/// the key, used when the id does not resolve and the name is unambiguous.
/// When a peer has several such events the oldest that resolves a key wins
/// (the feed is newest first), matching the Python check.
pub fn key_joins(
    events: &[NbEvent],
    keys: &[SetupKeyMeta],
    groups: &[NbGroup],
) -> BTreeMap<String, KeyJoin> {
    let gname: BTreeMap<&str, &str> = groups
        .iter()
        .map(|g| (g.id.as_str(), g.name.as_str()))
        .collect();
    let by_id: BTreeMap<&str, &SetupKeyMeta> = keys.iter().map(|k| (k.id.as_str(), k)).collect();
    let mut name_count: BTreeMap<&str, usize> = BTreeMap::new();
    for k in keys {
        *name_count.entry(k.name.as_str()).or_default() += 1;
    }
    let mut out: BTreeMap<String, KeyJoin> = BTreeMap::new();
    // Oldest first: the first event that resolves a key is the one kept.
    for e in events.iter().rev() {
        let peer_added = e.activity == "Peer added" || e.activity_code == "peer.setupkey.add";
        if !peer_added || e.target_id.is_empty() {
            continue;
        }
        let meta_name = e
            .meta
            .get("setup_key_name")
            .and_then(|v| v.as_str())
            .filter(|n| !n.is_empty());
        let key = by_id.get(e.initiator_id.as_str()).copied().or_else(|| {
            let n = meta_name?;
            if name_count.get(n) == Some(&1) {
                keys.iter().find(|k| k.name == n)
            } else {
                None
            }
        });
        let slot = out.entry(e.target_id.clone()).or_default();
        if slot.groups.is_some() {
            continue;
        }
        match key {
            Some(k) => {
                slot.key_name = Some(k.name.clone());
                slot.groups = Some(
                    k.auto_groups
                        .iter()
                        .map(|id| {
                            gname
                                .get(id.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| id.clone())
                        })
                        .collect(),
                );
            }
            None => {
                if slot.key_name.is_none() {
                    slot.key_name = meta_name.map(str::to_string);
                }
            }
        }
    }
    out.retain(|_, j| j.key_name.is_some() || j.groups.is_some());
    out
}

/// The name suffix of a person's handset setup key (`goat admin onboard`).
pub const HANDSET_KEY_SUFFIX: &str = "-handset";

/// Is this peer a handset? Its minting key's name ends `-handset`, or its own
/// name carries `-handset-`.
pub fn is_handset(peer_name: &str, join: Option<&KeyJoin>) -> bool {
    join.and_then(|j| j.key_name.as_deref())
        .map(|n| n.ends_with(HANDSET_KEY_SUFFIX))
        .unwrap_or(false)
        || peer_name.contains("-handset-")
}

/// `YYYY-MM-DD` at the start of `s`, followed by the end or `-…`.
fn starts_with_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && (b.len() == 10 || b[10] == b'-')
}

/// Does the remainder after `peer-<handle>` read as
/// `(-<org>)?-<scope>-<YYYY-MM-DD>(-…)?` (peer-name grammar v1.1.0; the org
/// short tag is one hyphen-free token)?
fn remainder_parses(rest: &str, scope: &str) -> bool {
    let at_scope = |r: &str| {
        r.strip_prefix('-')
            .and_then(|r| r.strip_prefix(scope))
            .and_then(|r| r.strip_prefix('-'))
            .map(starts_with_date)
            .unwrap_or(false)
    };
    if at_scope(rest) {
        return true;
    }
    let Some(r) = rest.strip_prefix('-') else {
        return false;
    };
    match r.find('-') {
        Some(n) if n > 0 => at_scope(&r[n..]),
        _ => false,
    }
}

/// Accept the pre-v1.1.0 `peer-H-H-<scope>-<date>` migration form.
/// This retires with the dual accept in `panoscope_core::peername::belongs_to`.
fn doubled_remainder_parses(rest: &str, handle: &str, scope: &str) -> bool {
    rest.strip_prefix('-')
        .and_then(|rest| rest.strip_prefix(handle))
        .map(|rest| remainder_parses(rest, scope))
        .unwrap_or(false)
}

/// The handle a person peer's name is keyed by, among the declared people,
/// ANCHORED on the scope being converged: the name resolves to handle `H`
/// only when it is `peer-H(-<org>)?-<scope>-<YYYY-MM-DD>(-…)?`. Among the
/// declared handles that satisfy that, the longest wins. So
/// `peer-jane-smith-acme-2030-01-05-x` resolves to `jane` only if `smith`
/// reads as the org (it does here) and `jane-smith` is not also declared;
/// and it never resolves when the remainder does not parse (a name minted for
/// another scope falls back to the key baseline).
pub fn declared_handle<'a>(
    peer_name: &str,
    scope: &str,
    people: &'a BTreeMap<String, PersonBaseline>,
) -> Option<&'a str> {
    let rest = panoscope_core::peername::rest(peer_name)?;
    people
        .keys()
        .map(String::as_str)
        .filter(|h| !h.is_empty())
        .filter(|h| {
            rest.strip_prefix(*h)
                .map(|r| remainder_parses(r, scope))
                .unwrap_or(false)
        })
        .max_by_key(|h| h.len())
}

fn declared_person_candidates<'a>(
    peer_name: &str,
    scope: &str,
    people: &'a BTreeMap<String, PersonBaseline>,
) -> Vec<(&'a str, &'a PersonBaseline)> {
    let Some(rest) = panoscope_core::peername::rest(peer_name) else {
        return Vec::new();
    };
    people
        .iter()
        .filter(|(handle, _)| {
            !handle.is_empty()
                && rest
                    .strip_prefix(handle.as_str())
                    .map(|tail| {
                        remainder_parses(tail, scope)
                            || doubled_remainder_parses(tail, handle, scope)
                    })
                    .unwrap_or(false)
        })
        .map(|(handle, baseline)| (handle.as_str(), baseline))
        .collect()
}

fn platform_admin_group(prefix: &str) -> String {
    format!("{prefix}-PLATFORM-ADMIN")
}

fn has_ory_admin(groups: &BTreeSet<String>) -> bool {
    groups
        .iter()
        .any(|g| matches!(g.as_str(), "PLATFORM-ADMIN" | "IAM-ADMIN" | "ADMINS"))
}

fn security_transition(
    current: &BTreeSet<String>,
    desired: &BTreeSet<String>,
    prefix: &str,
) -> bool {
    let quarantine = format!("{prefix}-QUARANTINED");
    current.contains(&quarantine) != desired.contains(&quarantine)
}

fn severance_refusal(reason: &str) -> String {
    format!(
        "REFUSED security-quarantine transition: {reason}. Privileged accounts do not use the \
         reversible quarantine workflow; sever the account and rotate its credentials under ADR \
         1091. Nothing was changed"
    )
}

pub(crate) fn unproven_owner_refusal(reason: &str) -> String {
    format!(
        "REFUSED security-quarantine transition: {reason}. Privilege cannot be ruled out, so the \
         reversible quarantine is refused. Fix the owner join (run from a repo checkout with the \
         goatnet's operator manifests, regenerate missing baseline data, or resolve conflicting \
         manifests or handle names) and retry; if the account is privileged, use ADR 1091 \
         severance and credential rotation instead. Nothing was changed"
    )
}

fn candidate_privilege_facts(
    person: &PersonBaseline,
    prefix: &str,
    ory: Result<&OryPrivilegeIndex, &str>,
) -> (bool, Option<&'static str>) {
    let declared_privileged = person
        .device
        .iter()
        .chain(person.handset.iter())
        .any(|g| g == &platform_admin_group(prefix));
    if person.email_hash == AMBIGUOUS_EMAIL_HASH {
        return (
            declared_privileged,
            Some(
                "the generated baseline and local operator manifest disagree about an owner; the join is ambiguous",
            ),
        );
    }
    if person.email_hash.is_empty() {
        return (
            declared_privileged,
            Some("a declared possible owner has no deployable Ory identity join"),
        );
    }
    let Ok(groups) = ory.and_then(|idx| {
        idx.groups_for(&person.email_hash)
            .map_err(|_| "the live Ory owner lookup was unavailable or ambiguous")
    }) else {
        return (
            declared_privileged,
            Some("the live Ory owner lookup was unavailable or ambiguous"),
        );
    };
    (declared_privileged || has_ory_admin(groups), None)
}

#[derive(Default)]
struct CandidateProof {
    ambiguous: bool,
    privileged: bool,
    unproven: Option<&'static str>,
}

impl CandidateProof {
    fn include(
        &mut self,
        candidates: &[(&str, &PersonBaseline)],
        prefix: &str,
        ory: Result<&OryPrivilegeIndex, &str>,
        mesh_privileged: bool,
    ) {
        self.ambiguous |= candidates.len() > 1;
        self.privileged |= mesh_privileged;
        for (_, person) in candidates {
            let (candidate_privileged, candidate_unproven) =
                candidate_privilege_facts(person, prefix, ory);
            self.privileged |= candidate_privileged;
            if self.unproven.is_none() {
                self.unproven = candidate_unproven;
            }
        }
    }

    fn refusal(self) -> Option<String> {
        if self.ambiguous && self.privileged {
            return Some(unproven_owner_refusal(
                "the person-to-peer ownership join is ambiguous and one possible owner is privileged",
            ));
        }
        if let Some(reason) = self.unproven {
            return Some(unproven_owner_refusal(reason));
        }
        self.privileged
            .then(|| severance_refusal("the subject is a privileged person"))
    }
}

fn candidate_set_quarantine_refusal(
    candidates: &[(&str, &PersonBaseline)],
    prefix: &str,
    ory: Result<&OryPrivilegeIndex, &str>,
    mesh_privileged: bool,
) -> Option<String> {
    let mut proof = CandidateProof::default();
    proof.include(candidates, prefix, ory, mesh_privileged);
    proof.refusal()
}

/// Evaluate a directly selected Ory identity even when no mesh mover exists.
/// This closes the partial-convergence case where the mesh is already at the
/// requested side of quarantine but the identity plane is not.
pub(crate) fn ory_identity_quarantine_refusal(
    email: &str,
    scope: &str,
    prefix: &str,
    people: &BTreeMap<String, PersonBaseline>,
    peers: &[LivePeer],
    baselines: &BTreeMap<String, Baseline>,
    ory: Result<&OryPrivilegeIndex, &str>,
) -> Option<String> {
    let hash = email_hash(email);
    let owners: Vec<(&str, &PersonBaseline)> = people
        .iter()
        .filter(|(_, person)| person.email_hash == hash)
        .map(|(handle, person)| (handle.as_str(), person))
        .collect();
    if owners.is_empty() {
        return Some(unproven_owner_refusal(
            "the person-to-peer ownership join was unavailable",
        ));
    }
    let mut proof = CandidateProof::default();
    proof.include(&owners, prefix, ory, false);
    let owner_handles: BTreeSet<&str> = owners.iter().map(|(handle, _)| *handle).collect();
    for peer in peers {
        let candidates = declared_person_candidates(&peer.name, scope, people);
        if candidates
            .iter()
            .any(|(candidate, _)| owner_handles.contains(candidate))
        {
            let mesh_privileged = peer
                .groups
                .iter()
                .any(|g| g == &platform_admin_group(prefix))
                || baselines
                    .get(&peer.id)
                    .map(|baseline| baseline.groups.contains(&platform_admin_group(prefix)))
                    .unwrap_or(false);
            proof.include(&candidates, prefix, ory, mesh_privileged);
        }
    }
    proof.refusal()
}

/// Return the reason a quarantine entry/exit must be refused. `None` means the
/// transition is allowed. Dormancy-only PROBATION transitions never enter this
/// function's policy path and remain releasable for administrators.
pub(crate) fn quarantine_refusal(
    peer: &LivePeer,
    desired: &BTreeSet<String>,
    scope: &str,
    prefix: &str,
    people: &BTreeMap<String, PersonBaseline>,
    resolved_baseline: Option<&Baseline>,
    ory: Result<&OryPrivilegeIndex, &str>,
) -> Option<String> {
    quarantine_refusal_with_owner_reason(
        peer,
        desired,
        scope,
        prefix,
        OwnerJoin {
            people,
            missing_reason: None,
        },
        resolved_baseline,
        ory,
    )
}

pub(crate) struct OwnerJoin<'a> {
    pub people: &'a BTreeMap<String, PersonBaseline>,
    pub missing_reason: Option<&'a str>,
}

pub(crate) fn quarantine_refusal_with_owner_reason(
    peer: &LivePeer,
    desired: &BTreeSet<String>,
    scope: &str,
    prefix: &str,
    owner_join: OwnerJoin<'_>,
    resolved_baseline: Option<&Baseline>,
    ory: Result<&OryPrivilegeIndex, &str>,
) -> Option<String> {
    let current: BTreeSet<String> = peer.groups.iter().cloned().collect();
    if !security_transition(&current, desired, prefix) {
        return None;
    }

    let mesh_privileged = current.contains(&platform_admin_group(prefix))
        || resolved_baseline
            .map(|b| b.groups.contains(&platform_admin_group(prefix)))
            .unwrap_or(false);
    let candidates = declared_person_candidates(&peer.name, scope, owner_join.people);
    if !candidates.is_empty() {
        return candidate_set_quarantine_refusal(&candidates, prefix, ory, mesh_privileged);
    }

    // `peer-…` is the person-device namespace. If no declared owner matches,
    // treating it as a machine would turn a missing roster into permission.
    if panoscope_core::peername::rest(&peer.name).is_some() {
        return Some(unproven_owner_refusal(owner_join.missing_reason.unwrap_or(
            "no unique declared person owns this person-shaped peer",
        )));
    }
    if mesh_privileged {
        return Some(severance_refusal(
            "the machine or system holds a PLATFORM-ADMIN mesh baseline",
        ));
    }
    None
}

fn guard_quarantine_plan(
    plan: &mut Plan,
    peers: &[LivePeer],
    scope: &str,
    prefix: &str,
    people: &BTreeMap<String, PersonBaseline>,
    baselines: &BTreeMap<String, Baseline>,
    ory: Result<&OryPrivilegeIndex, &str>,
) -> usize {
    let quarantine = format!("{prefix}-QUARANTINED");
    let mut blocked = 0usize;
    for row in &mut plan.rows {
        let Some(desired) = row
            .desired
            .as_ref()
            .map(|v| v.iter().cloned().collect::<BTreeSet<_>>())
        else {
            continue;
        };
        if row.current.contains(&quarantine) == desired.contains(&quarantine) {
            continue;
        }
        let Some(peer) = peers.iter().find(|p| p.id == row.id) else {
            continue;
        };
        if let Some(reason) = quarantine_refusal(
            peer,
            &desired,
            scope,
            prefix,
            people,
            baselines.get(&peer.id),
            ory,
        ) {
            row.status = Status::Blocked;
            row.note = reason;
            blocked += 1;
        }
    }
    blocked
}

fn privilege_exit(mode: Mode, blocked: usize) -> Option<WarrantExit> {
    (mode == Mode::Enforce && blocked > 0).then(|| WarrantExit {
        code: EXIT_PRIVILEGED_QUARANTINE,
        msg: format!(
            "refused: {blocked} security-quarantine transition(s) were refused by the \
             privileged-account gate; other rows converged normally"
        ),
    })
}

/// One peer's baseline: (1) a person peer whose handle is in the declared
/// baseline (anchored on `scope`, see [`declared_handle`]) gets that
/// person's `handset` list when it is a handset, else their `device` list;
/// (2) otherwise its minting key's auto_groups; (3) otherwise none.
pub fn resolve_baseline(
    peer_name: &str,
    scope: &str,
    people: &BTreeMap<String, PersonBaseline>,
    join: Option<&KeyJoin>,
) -> Option<Baseline> {
    if let Some(h) = declared_handle(peer_name, scope, people) {
        let p = &people[h];
        let list = if is_handset(peer_name, join) {
            &p.handset
        } else {
            &p.device
        };
        return Some(Baseline {
            source: BaselineSource::Declared,
            groups: list.iter().cloned().collect(),
        });
    }
    join.and_then(|j| j.groups.clone()).map(|groups| Baseline {
        source: BaselineSource::Key,
        groups,
    })
}

/// Peer id → resolved baseline, for every peer that has one.
pub fn resolve_baselines(
    scope: &str,
    people: &BTreeMap<String, PersonBaseline>,
    peers: &[LivePeer],
    joins: &BTreeMap<String, KeyJoin>,
) -> BTreeMap<String, Baseline> {
    peers
        .iter()
        .filter_map(|p| {
            resolve_baseline(&p.name, scope, people, joins.get(&p.id))
                .map(|b| (p.id.clone(), b))
        })
        .collect()
}

/// The mode a run actually uses, or the refusal when the CLI asks for more
/// than the file allows.
pub fn effective_mode(requested: Mode, ceiling: Converge) -> Result<Mode, WarrantExit> {
    match (requested, ceiling) {
        (Mode::Enforce, Converge::Report) => Err(WarrantExit {
            code: EXIT_MODE_CEILING,
            msg: "refused: --mode enforce on a declaration that says `converge: report`. \
                  The file is the ceiling — the CLI can lower enforce to report, never raise it. \
                  Change `converge:` in a reviewed PR first."
                .into(),
        }),
        (m, _) => Ok(m),
    }
}

/// The `--max-changes` gate.
pub fn check_max_changes(planned: usize, max: usize) -> Result<(), WarrantExit> {
    if planned > max {
        Err(WarrantExit {
            code: EXIT_MAX_CHANGES,
            msg: format!(
                "refused: this run would make {planned} peer-group changes, over --max-changes \
                 {max}. Nothing was changed. Read the report (--mode report) — a broken \
                 declaration or an empty activity log looks exactly like this. If the changes are \
                 intended, re-run once with a higher --max-changes."
            ),
        })
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Pause — the audited, expiring stop (mesh-change-control.md §2, §4.4)
// ---------------------------------------------------------------------------
//
// An emergency membership edit in the admin dashboard would be reverted by
// the next converger run. `goat warrant pause` writes
// `<state-dir>/pause-<scope>.json`; while it is in force every
// `apply --membership` run on that scope is a report run. It expires on its
// own (24 hours at most) and is recorded as an operator action, which
// `systemctl stop warrant-membership.timer` was not.
//
// A pause file the converger cannot read is IGNORED with a warning: a broken
// pause must not disable the converger forever. The same holds for an `until`
// further out than the 24-hour maximum (a hand-edited file).

/// Where the systemd unit keeps its state (`StateDirectory=goat-warrant`).
pub const DEFAULT_STATE_DIR: &str = "/var/lib/goat-warrant";
/// The longest pause `goat warrant pause` accepts.
pub const MAX_PAUSE_SECS: i64 = 24 * 3600;

#[derive(Args, Debug)]
pub struct PauseArgs {
    /// Taxonomy scope whose converger to pause, e.g. `zagros-dev`.
    #[arg(long)]
    pub scope: String,
    /// How long: `30m`, `1h`, `90s`, `2h30m`. At most 24h.
    #[arg(long = "for", value_name = "DURATION")]
    pub duration: String,
    /// Why — recorded in the pause file and the operator-action event.
    #[arg(long)]
    pub reason: String,
    /// The converger's state directory.
    #[arg(long, default_value = DEFAULT_STATE_DIR)]
    pub state_dir: PathBuf,
}

#[derive(Args, Debug)]
pub struct ResumeArgs {
    /// Taxonomy scope whose pause to end, e.g. `zagros-dev`.
    #[arg(long)]
    pub scope: String,
    /// The converger's state directory.
    #[arg(long, default_value = DEFAULT_STATE_DIR)]
    pub state_dir: PathBuf,
}

/// The pause file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseRecord {
    pub scope: String,
    /// RFC 3339, UTC.
    pub until: String,
    pub reason: String,
    pub actor: String,
    /// RFC 3339, UTC.
    pub created: String,
}

/// What the converger found in the pause file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseState {
    None,
    Active(PauseRecord),
    Expired(PauseRecord),
    /// Unreadable or out of bounds; ignored. The string says why.
    Malformed(String),
}

/// The subset of a pause the report carries.
#[derive(Debug, Clone, Serialize)]
struct PauseView {
    until: String,
    actor: String,
    reason: String,
}

impl From<&PauseRecord> for PauseView {
    fn from(p: &PauseRecord) -> Self {
        PauseView {
            until: p.until.clone(),
            actor: p.actor.clone(),
            reason: p.reason.clone(),
        }
    }
}

fn rfc3339(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// A scope is used in a file name, so it must be a plain taxonomy id.
fn check_scope_name(scope: &str) -> anyhow::Result<()> {
    let ok = !scope.is_empty()
        && scope
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !scope.starts_with('-');
    if !ok {
        anyhow::bail!(
            "scope `{scope}` is not a taxonomy scope id (lowercase letters, digits, `-`)"
        );
    }
    Ok(())
}

pub fn pause_path(state_dir: &Path, scope: &str) -> PathBuf {
    state_dir.join(format!("pause-{scope}.json"))
}

/// `90s`, `30m`, `1h`, `2h30m` → seconds. Refuses zero and anything over 24h.
pub fn parse_pause_duration(s: &str) -> anyhow::Result<i64> {
    let t = s.trim();
    let bad = || anyhow::anyhow!("--for `{s}` is not a duration — use e.g. 30m, 1h, 2h30m");
    if t.is_empty() {
        return Err(bad());
    }
    let mut total: i64 = 0;
    let mut num = String::new();
    for c in t.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let unit = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            _ => return Err(bad()),
        };
        let n: i64 = num.parse().map_err(|_| bad())?;
        total = total.saturating_add(n.saturating_mul(unit));
        num.clear();
    }
    if !num.is_empty() {
        return Err(bad());
    }
    if total <= 0 {
        anyhow::bail!("--for must be longer than zero");
    }
    if total > MAX_PAUSE_SECS {
        anyhow::bail!(
            "refused: --for {s} is over the 24h maximum. A pause is for an emergency edit; \
             anything longer belongs in the membership file as a reviewed change."
        );
    }
    Ok(total)
}

/// Build the record `goat warrant pause` writes. Pure.
pub fn build_pause(
    scope: &str,
    duration: &str,
    reason: &str,
    actor: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<PauseRecord> {
    check_scope_name(scope)?;
    if reason.trim().is_empty() {
        anyhow::bail!("refused: --reason is required and must not be empty");
    }
    let secs = parse_pause_duration(duration)?;
    Ok(PauseRecord {
        scope: scope.to_string(),
        until: rfc3339(now + chrono::Duration::seconds(secs)),
        reason: reason.trim().to_string(),
        actor: actor.to_string(),
        created: rfc3339(now),
    })
}

/// Classify a pause file's contents (None = no file). Pure.
pub fn pause_state(
    text: Option<&str>,
    scope: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> PauseState {
    let Some(text) = text else {
        return PauseState::None;
    };
    let rec: PauseRecord = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => return PauseState::Malformed(format!("does not parse: {e}")),
    };
    if rec.scope != scope {
        return PauseState::Malformed(format!("is for scope `{}`, not `{scope}`", rec.scope));
    }
    let Some(until) = parse_utc(&rec.until) else {
        return PauseState::Malformed(format!("`until` `{}` is not an RFC 3339 time", rec.until));
    };
    if until > now + chrono::Duration::seconds(MAX_PAUSE_SECS + 60) {
        return PauseState::Malformed(format!(
            "`until` {} is more than 24h away — no pause may run that long",
            rec.until
        ));
    }
    if now < until {
        PauseState::Active(rec)
    } else {
        PauseState::Expired(rec)
    }
}

fn read_pause(state_dir: &Path, scope: &str, now: chrono::DateTime<chrono::Utc>) -> PauseState {
    let path = pause_path(state_dir, scope);
    match std::fs::read_to_string(&path) {
        Ok(t) => pause_state(Some(&t), scope, now),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PauseState::None,
        Err(e) => PauseState::Malformed(format!("cannot be read: {e}")),
    }
}

/// Who is pausing: $GOAT_OPERATOR_HANDLE, else $SUDO_USER, else $USER.
fn pause_actor() -> String {
    ["GOAT_OPERATOR_HANDLE", "SUDO_USER", "USER"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// The audit actor: the pause actor when it is a valid handle, else the
/// profile's handle.
fn audit_actor(ctx: &crate::cmd::Ctx, actor: &str) -> String {
    if crate::cmd::audit::is_kebab_handle(actor) {
        actor.to_string()
    } else {
        crate::cmd::audit::actor_handle(&ctx.profile)
    }
}

fn require_state_dir(dir: &Path) -> anyhow::Result<()> {
    if !dir.is_dir() {
        anyhow::bail!(
            "state directory {} does not exist — is warrant-membership installed on this host? \
             A pause written anywhere else is not read by the converger.",
            dir.display()
        );
    }
    Ok(())
}

pub async fn run_pause(ctx: &crate::cmd::Ctx, a: PauseArgs) -> anyhow::Result<()> {
    let actor = pause_actor();
    let rec = build_pause(&a.scope, &a.duration, &a.reason, &actor, chrono::Utc::now())?;
    require_state_dir(&a.state_dir)?;
    let path = pause_path(&a.state_dir, &a.scope);
    if let PauseState::Active(old) = read_pause(&a.state_dir, &a.scope, chrono::Utc::now()) {
        println!(
            "replacing the pause until {} by {}: {}",
            old.until, old.actor, old.reason
        );
    }
    let body = serde_json::to_string_pretty(&rec)? + "\n";
    write_atomic(&path, body.as_bytes())?;
    println!(
        "paused warrant membership on `{}` until {} ({}) by {}: {}\n  wrote {}\n  \
         every converger run until then is report-only. End it early with \
         `goat warrant resume --scope {}`.",
        rec.scope,
        rec.until,
        a.duration,
        rec.actor,
        rec.reason,
        path.display(),
        rec.scope
    );
    let act = crate::cmd::audit::OperatorActionV1::new(
        audit_actor(ctx, &rec.actor),
        "goat-cli",
        "warrant.membership.pause",
        rec.scope.clone(),
        rec.scope.clone(),
        "ok",
    )
    .with_args_digest(&format!(
        "warrant membership pause scope={} until={} actor={} reason={}",
        rec.scope, rec.until, rec.actor, rec.reason
    ));
    crate::cmd::audit::emit(&ctx.profile, act).await;
    Ok(())
}

pub async fn run_resume(ctx: &crate::cmd::Ctx, a: ResumeArgs) -> anyhow::Result<()> {
    check_scope_name(&a.scope)?;
    let path = pause_path(&a.state_dir, &a.scope);
    let state = read_pause(&a.state_dir, &a.scope, chrono::Utc::now());
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "no pause on `{}` ({} does not exist) — nothing to resume.",
                a.scope,
                path.display()
            );
            return Ok(());
        }
        Err(e) => anyhow::bail!("remove {}: {e}", path.display()),
    }
    let what = match &state {
        PauseState::Active(p) => format!(
            "active pause until {} by {}: {}",
            p.until, p.actor, p.reason
        ),
        PauseState::Expired(p) => format!(
            "expired pause (until {}) by {}: {}",
            p.until, p.actor, p.reason
        ),
        PauseState::Malformed(why) => format!("unreadable pause file ({why})"),
        PauseState::None => "pause file".into(),
    };
    println!(
        "resumed warrant membership on `{}`: removed the {what}\n  removed {}\n  \
         the next converger run applies the declaration in its configured mode.",
        a.scope,
        path.display()
    );
    let actor = pause_actor();
    let act = crate::cmd::audit::OperatorActionV1::new(
        audit_actor(ctx, &actor),
        "goat-cli",
        "warrant.membership.resume",
        a.scope.clone(),
        a.scope.clone(),
        "ok",
    )
    .with_args_digest(&format!(
        "warrant membership resume scope={} actor={actor} removed={what}",
        a.scope
    ));
    crate::cmd::audit::emit(&ctx.profile, act).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

fn nb_client(scope: &str, mgmt_url: Option<&str>) -> anyhow::Result<NetbirdClient> {
    let scoped = format!("{}_NB_API_TOKEN", scope.to_uppercase().replace('-', "_"));
    let token = std::env::var("GOAT_NB_API_TOKEN")
        .ok()
        .or_else(|| std::env::var(&scoped).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("NetBird API token not set — export ${scoped} (or $GOAT_NB_API_TOKEN)")
        })?;
    let url = mgmt_url
        .map(str::to_string)
        .or_else(|| std::env::var("GOAT_NB_MGMT_URL").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "NetBird management URL not set — pass --mgmt-url or export $GOAT_NB_MGMT_URL"
            )
        })?;
    Ok(NetbirdClient::new(url, token))
}

/// Taxonomy prefixes longer than ours that start with ours (see [`is_managed`]).
fn longer_prefixes(decl: &Declaration, prefix: &str) -> Vec<String> {
    let mine = format!("{prefix}-");
    decl.scopes()
        .iter()
        .filter_map(|s| decl.model(s).ok().map(|m| m.prefix))
        .filter(|p| p.starts_with(&mine))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

struct Applied {
    ops: usize,
    error: Option<anyhow::Error>,
}

fn applicable_rows(plan: &Plan) -> impl Iterator<Item = &Row> {
    plan.rows.iter().filter(|row| row.status == Status::Drift)
}

/// Mirror-first: every add in every group lands and is re-read before any
/// removal is made. A failure stops the run where it is; peers are at worst
/// holding an extra group, never missing one.
async fn apply_plan(nb: &NetbirdClient, plan: &Plan) -> Applied {
    let mut adds: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut removes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in applicable_rows(plan) {
        for g in &r.add {
            adds.entry(g.clone()).or_default().insert(r.id.clone());
        }
        for g in &r.remove {
            removes.entry(g.clone()).or_default().insert(r.id.clone());
        }
    }
    let mut ops = 0;
    let res: anyhow::Result<()> = async {
        for (name, ids) in &adds {
            let groups = nb.list_groups().await.map_err(super::super::members::map_nb_err)?;
            let g = groups
                .iter()
                .find(|g| &g.name == name)
                .ok_or_else(|| anyhow::anyhow!("group {name} vanished before the add"))?;
            let mut set: BTreeSet<String> = g.peers.iter().map(|p| p.id.clone()).collect();
            set.extend(ids.iter().cloned());
            nb.update_group_peers(&g.id, &g.name, &set.into_iter().collect::<Vec<_>>())
                .await
                .map_err(super::super::members::map_nb_err)?;
            let verify = nb.list_groups().await.map_err(super::super::members::map_nb_err)?;
            let have: BTreeSet<&str> = verify
                .iter()
                .find(|v| &v.name == name)
                .map(|v| v.peers.iter().map(|p| p.id.as_str()).collect())
                .unwrap_or_default();
            if let Some(missing) = ids.iter().find(|i| !have.contains(i.as_str())) {
                anyhow::bail!("added peer {missing} to {name} but it is not there on re-read — nothing removed");
            }
            ops += ids.len();
        }
        for (name, ids) in &removes {
            let groups = nb.list_groups().await.map_err(super::super::members::map_nb_err)?;
            let Some(g) = groups.iter().find(|g| &g.name == name) else { continue };
            let keep: Vec<String> =
                g.peers.iter().map(|p| p.id.clone()).filter(|id| !ids.contains(id)).collect();
            nb.update_group_peers(&g.id, &g.name, &keep)
                .await
                .map_err(super::super::members::map_nb_err)?;
            ops += ids.len();
        }
        let after = nb.list_groups().await.map_err(super::super::members::map_nb_err)?;
        for (name, ids) in &removes {
            if let Some(g) = after.iter().find(|g| &g.name == name) {
                if let Some(left) = g.peers.iter().find(|p| ids.contains(&p.id)) {
                    anyhow::bail!("removed peer {} from {name} but it is still there on re-read", left.id);
                }
            }
        }
        Ok(())
    }
    .await;
    Applied {
        ops,
        error: res.err(),
    }
}

struct Live {
    peers: Vec<LivePeer>,
    joins: BTreeMap<String, KeyJoin>,
    groups: BTreeMap<String, GroupState>,
}

impl Live {
    fn baselines(&self, decl: &MembershipDecl) -> BTreeMap<String, Baseline> {
        resolve_baselines(&decl.scope, &decl.people, &self.peers, &self.joins)
    }
}

async fn read_live(nb: &NetbirdClient) -> anyhow::Result<Live> {
    use super::super::members::{covered_group_ids, map_nb_err};
    let peers = nb.list_peers().await.map_err(map_nb_err)?;
    let groups = nb.list_groups().await.map_err(map_nb_err)?;
    let policies = nb.list_policies().await.map_err(map_nb_err)?;
    let events = nb.list_events().await.map_err(map_nb_err)?;
    let keys = nb.list_setup_keys().await.map_err(map_nb_err)?;
    let covered = covered_group_ids(&policies);
    Ok(Live {
        peers: peers.iter().map(LivePeer::from).collect(),
        joins: key_joins(&events, &keys, &groups),
        groups: groups
            .iter()
            .map(|g| {
                (
                    g.name.clone(),
                    GroupState {
                        covered: covered.contains(&g.id),
                    },
                )
            })
            .collect(),
    })
}

pub async fn run_apply(ctx: &crate::cmd::Ctx, a: ApplyArgs) -> anyhow::Result<()> {
    if !a.membership {
        anyhow::bail!("`goat warrant apply` converges membership only today — pass --membership");
    }
    let decl = load_decl(&a.file, &a.scope)?;
    let prefix = scope_prefix(&a.scope);
    let tax = super::super::acl::declaration(
        &ctx.profile,
        a.acl_declaration.as_deref(),
        "warrant apply",
    )?;
    if let Ok(m) = tax.model(&a.scope) {
        if m.prefix != prefix {
            anyhow::bail!(
                "scope `{}` has site_prefix `{}` in the taxonomy, not `{prefix}` — this converger \
                 derives the managed prefix from the scope name and refuses to guess",
                a.scope,
                m.prefix
            );
        }
    }
    let excluded = longer_prefixes(&tax, &prefix);
    let requested = effective_mode(a.mode, decl.converge)?;
    let pause = read_pause(&a.state_dir, &a.scope, chrono::Utc::now());
    let mode = mode_under_pause(requested, &pause);
    announce_pause(&pause, &a);

    let nb = nb_client(&a.scope, a.mgmt_url.as_deref())?;
    let live = read_live(&nb).await?;
    let baselines = live.baselines(&decl);
    let mut plan = plan_membership(
        &decl,
        &prefix,
        &excluded,
        &live.peers,
        &baselines,
        &live.groups,
    )?;
    let quarantine = format!("{prefix}-QUARANTINED");
    let guarded_ids: BTreeSet<String> = plan
        .rows
        .iter()
        .filter(|r| {
            r.desired
                .as_ref()
                .map(|d| r.current.contains(&quarantine) != d.contains(&quarantine))
                .unwrap_or(false)
        })
        .map(|r| r.id.clone())
        .collect();
    let needs_ory = live
        .peers
        .iter()
        .any(|p| guarded_ids.contains(&p.id) && panoscope_core::peername::rest(&p.name).is_some());
    let ory = if needs_ory {
        Some(
            match super::super::admin::native_ory_config_for(Some(&a.scope), &ctx.profile) {
                Some(cfg) => read_ory_privileges(&cfg).await,
                None => Err(anyhow::anyhow!(
                    "no Ory admin configuration for scope `{}`",
                    a.scope
                )),
            },
        )
    } else {
        None
    };
    let ory_view = match ory.as_ref() {
        Some(Ok(index)) => Ok(index),
        _ => Err("the live Ory owner lookup was unavailable or ambiguous"),
    };
    let privilege_blocked = guard_quarantine_plan(
        &mut plan,
        &live.peers,
        &a.scope,
        &prefix,
        &decl.people,
        &baselines,
        ory_view,
    );
    let planned = plan.changes_planned();

    let mut report = Report {
        scope: decl.scope.clone(),
        mode: if mode == Mode::Enforce {
            "enforce"
        } else {
            "report"
        },
        peers_total: plan.peers_total(),
        r#match: plan.matched(),
        drift: plan.drift(),
        blocked: plan.blocked(),
        privileged_quarantine_blocked: privilege_blocked,
        unknown: plan.unknown(),
        missing_peer: plan.missing_peer(),
        baseline_declared: plan.baseline_declared(),
        changes_planned: planned,
        changes_applied: 0,
        drift_after: None,
        paused: match &pause {
            PauseState::Active(p) => Some(p.into()),
            _ => None,
        },
        pause_expired: match &pause {
            PauseState::Expired(p) => Some(p.into()),
            _ => None,
        },
        pause_warning: match &pause {
            PauseState::Malformed(why) => Some(format!(
                "pause file {} {why}; ignored",
                pause_path(&a.state_dir, &a.scope).display()
            )),
            _ => None,
        },
        rows: plan.rows.clone(),
    };

    if mode == Mode::Enforce {
        if let Err(refusal) = check_max_changes(planned, a.max_changes) {
            emit_report(&report, &a, &prefix);
            write_metrics(&a, &report);
            return Err(refusal.into());
        }
    }

    let mut failure: Option<anyhow::Error> = None;
    if mode == Mode::Enforce && planned > 0 {
        let applied = apply_plan(&nb, &plan).await;
        report.changes_applied = applied.ops;
        // Read back: the counts after the run are what the goatnet holds now.
        match read_live(&nb).await.and_then(|l| {
            plan_membership(
                &decl,
                &prefix,
                &excluded,
                &l.peers,
                &l.baselines(&decl),
                &l.groups,
            )
        }) {
            Ok(after) => report.drift_after = Some(after.drift()),
            Err(e) => eprintln!("warning: read-back after enforce failed: {e:#}"),
        }
        if applied.ops > 0 || applied.error.is_some() {
            let actor = crate::cmd::audit::actor_handle(&ctx.profile);
            let summary = format!(
                "warrant membership apply scope={} planned={planned} applied={} blocked={} file={}",
                a.scope,
                applied.ops,
                report.blocked,
                a.file.display()
            );
            let mut act = crate::cmd::audit::OperatorActionV1::new(
                &actor,
                "goat-cli",
                "warrant.membership.apply",
                a.scope.clone(),
                a.scope.clone(),
                if applied.error.is_none() {
                    "ok"
                } else {
                    "failed"
                },
            )
            .with_args_digest(&summary);
            if let Some(e) = &applied.error {
                act = act.with_error(format!("{e:#}"));
            }
            crate::cmd::audit::emit(&ctx.profile, act).await;
        }
        failure = applied.error;
    }

    emit_report(&report, &a, &prefix);
    write_metrics(&a, &report);

    if let Some(e) = failure {
        return Err(e.context(format!(
            "warrant membership enforce on `{}` stopped after {} change(s)",
            a.scope, report.changes_applied
        )));
    }
    if let Some(refusal) = privilege_exit(mode, privilege_blocked) {
        return Err(refusal.into());
    }
    if mode == Mode::Enforce && report.blocked > 0 {
        anyhow::bail!(
            "{} peer(s) blocked by the coverage gate on `{}` — see the rows marked blocked",
            report.blocked,
            a.scope
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct Report {
    scope: String,
    mode: &'static str,
    peers_total: usize,
    r#match: usize,
    drift: usize,
    blocked: usize,
    privileged_quarantine_blocked: usize,
    unknown: usize,
    missing_peer: usize,
    /// Peers whose baseline came from `_baseline.generated.yaml`.
    baseline_declared: usize,
    changes_planned: usize,
    changes_applied: usize,
    /// Drift re-read after an enforce run; absent on a report run.
    #[serde(skip_serializing_if = "Option::is_none")]
    drift_after: Option<usize>,
    /// Present while a `goat warrant pause` is in force; the run is report.
    #[serde(skip_serializing_if = "Option::is_none")]
    paused: Option<PauseView>,
    /// A pause file whose `until` has passed; ignored, not deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pause_expired: Option<PauseView>,
    /// A pause file that could not be used; ignored, the run converges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pause_warning: Option<String>,
    rows: Vec<Row>,
}

/// The mode a run uses once any pause is taken into account.
pub fn mode_under_pause(requested: Mode, pause: &PauseState) -> Mode {
    match pause {
        PauseState::Active(_) => Mode::Report,
        _ => requested,
    }
}

/// Expiry and malformed-file notices go to stderr so they reach the journal
/// on a `--json` run too. The active-pause line is part of the human report
/// (and on stderr under `--json`).
fn announce_pause(pause: &PauseState, a: &ApplyArgs) {
    let path = pause_path(&a.state_dir, &a.scope);
    match pause {
        PauseState::None => {}
        PauseState::Active(p) => {
            if a.json {
                eprintln!("paused until {} by {}: {}", p.until, p.actor, p.reason);
            }
        }
        PauseState::Expired(p) => eprintln!(
            "note: the pause in {} expired at {} (set by {}: {}); ignored — converging normally. \
             `goat warrant resume --scope {}` removes the file.",
            path.display(),
            p.until,
            p.actor,
            p.reason,
            a.scope
        ),
        PauseState::Malformed(why) => eprintln!(
            "warning: pause file {} {why}; ignored — converging normally. A pause that cannot \
             be read does not stop the converger. Fix it with `goat warrant pause` or remove it \
             with `goat warrant resume --scope {}`.",
            path.display(),
            a.scope
        ),
    }
}

fn emit_report(r: &Report, a: &ApplyArgs, prefix: &str) {
    if a.json {
        match serde_json::to_string_pretty(r) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("error: could not render JSON: {e}"),
        }
        return;
    }
    println!(
        "warrant membership — scope {} (managed prefix {prefix}-), mode {}",
        r.scope, r.mode
    );
    if let Some(p) = &r.paused {
        println!("paused until {} by {}: {}", p.until, p.actor, p.reason);
    }
    if let Some(p) = &r.pause_expired {
        println!(
            "pause expired at {} (set by {}: {}); ignored",
            p.until, p.actor, p.reason
        );
    }
    if let Some(w) = &r.pause_warning {
        println!("warning: {w}");
    }
    println!();
    let shown: Vec<&Row> = r
        .rows
        .iter()
        .filter(|x| x.status != Status::Match)
        .collect();
    if shown.is_empty() {
        println!("every peer holds what the declaration says.");
    } else {
        println!(
            "{:<44} {:<12} {:<9} {:<34} REMOVE",
            "PEER", "STATUS", "BASELINE", "ADD"
        );
        for x in shown {
            let word = |v: serde_json::Result<serde_json::Value>| {
                v.ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            let status = word(serde_json::to_value(x.status));
            let base = word(serde_json::to_value(x.baseline_source));
            println!(
                "{:<44} {:<12} {:<9} {:<34} {}",
                x.peer,
                status,
                base,
                if x.add.is_empty() {
                    "-".into()
                } else {
                    x.add.join(",")
                },
                if x.remove.is_empty() {
                    "-".into()
                } else {
                    x.remove.join(",")
                },
            );
            if !x.note.is_empty() {
                println!("{:<44} {}", "", x.note);
            }
        }
    }
    println!(
        "\npeers {}  match {}  drift {} (blocked {})  unknown {}  missing-peer {}  \
         baseline declared {}  changes planned {}  applied {}{}",
        r.peers_total,
        r.r#match,
        r.drift,
        r.blocked,
        r.unknown,
        r.missing_peer,
        r.baseline_declared,
        r.changes_planned,
        r.changes_applied,
        r.drift_after
            .map(|d| format!("  drift after {d}"))
            .unwrap_or_default()
    );
    if r.mode == "report" && r.changes_planned > 0 {
        println!("report mode — nothing changed.");
    }
}

/// The node_exporter textfile body.
fn metrics_body(r: &Report, now_secs: u64) -> String {
    let l = format!(
        "scope=\"{}\"",
        r.scope.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let drift = r.drift_after.unwrap_or(r.drift);
    let rows: [(&str, &str, String); 10] = [
        (
            "peers_total",
            "Peers read from the management API.",
            r.peers_total.to_string(),
        ),
        (
            "match_total",
            "Peers holding exactly their declared managed groups.",
            r.r#match.to_string(),
        ),
        (
            "drift_total",
            "Peers not holding their declared managed groups (after the run, on enforce).",
            drift.to_string(),
        ),
        (
            "unknown_baseline_total",
            "Peers with no baseline (declared or key) and no exact or hold entry; never changed.",
            r.unknown.to_string(),
        ),
        (
            "baseline_declared_total",
            "Peers whose baseline came from the declared per-person baseline, not the minting key.",
            r.baseline_declared.to_string(),
        ),
        (
            "privileged_quarantine_blocked",
            "Security-quarantine transitions blocked by the privileged-account gate.",
            r.privileged_quarantine_blocked.to_string(),
        ),
        (
            "changes_applied_total",
            "Peer-group changes this run made.",
            r.changes_applied.to_string(),
        ),
        (
            "last_run_timestamp_seconds",
            "Unix time the run finished.",
            now_secs.to_string(),
        ),
        (
            "enforce",
            "1 when the run was in enforce mode, 0 in report mode.",
            u8::from(r.mode == "enforce").to_string(),
        ),
        (
            "paused",
            "1 while a goat warrant pause is in force (the run is report-only), else 0.",
            u8::from(r.paused.is_some()).to_string(),
        ),
    ];
    let mut out = String::new();
    for (name, help, v) in rows {
        out.push_str(&format!(
            "# HELP goat_warrant_membership_{name} {help}\n# TYPE goat_warrant_membership_{name} gauge\n\
             goat_warrant_membership_{name}{{{l}}} {v}\n"
        ));
    }
    out
}

fn write_metrics(a: &ApplyArgs, r: &Report) {
    let Some(dir) = &a.textfile_dir else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join("goat_warrant_membership.prom");
    if let Err(e) = write_atomic(&path, metrics_body(r, now).as_bytes()) {
        // Never let the sink fail the run — but say so, rather than leave a
        // stale file to be read as a fresh result.
        eprintln!("warning: could not write {}: {e:#}", path.display());
    }
}

// ---------------------------------------------------------------------------
// Tests — the pure planner, the gates, and the file edit.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "ZAGROS-DEV";

    fn decl(converge: &str, entries: &str) -> MembershipDecl {
        let text = format!("version: 1\nentries:{entries}\n");
        MembershipDecl {
            scope: "zagros-dev".into(),
            converge: if converge == "enforce" {
                Converge::Enforce
            } else {
                Converge::Report
            },
            entries: Some(parse_fragment(&text, "zagros-dev").unwrap()),
            people: BTreeMap::new(),
            sources: BTreeMap::new(),
        }
    }

    /// A key baseline.
    fn key(g: &[&str]) -> Baseline {
        Baseline {
            source: BaselineSource::Key,
            groups: set(g),
        }
    }

    fn peer(id: &str, name: &str, groups: &[&str]) -> LivePeer {
        LivePeer {
            id: id.into(),
            name: name.into(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
        }
    }

    fn set(g: &[&str]) -> BTreeSet<String> {
        g.iter().map(|s| s.to_string()).collect()
    }

    fn live(names: &[&str]) -> BTreeMap<String, GroupState> {
        names
            .iter()
            .map(|n| (n.to_string(), GroupState { covered: true }))
            .collect()
    }

    fn all_groups() -> BTreeMap<String, GroupState> {
        live(&[
            "ZAGROS-DEV-DEVELOPER",
            "ZAGROS-DEV-OBSERVER",
            "ZAGROS-DEV-P2P",
            "ZAGROS-DEV-QUARANTINED",
            "ZAGROS-DEV-EXTERNAL-INFRA",
        ])
    }

    fn row<'a>(p: &'a Plan, name: &str) -> &'a Row {
        p.rows.iter().find(|r| r.peer == name).unwrap()
    }

    fn quarantine_row(peer: &LivePeer) -> Row {
        Row {
            peer: peer.name.clone(),
            id: peer.id.clone(),
            current: peer.groups.clone(),
            desired: Some(vec!["ZAGROS-DEV-QUARANTINED".into()]),
            add: vec!["ZAGROS-DEV-QUARANTINED".into()],
            remove: peer.groups.clone(),
            status: Status::Drift,
            baseline_source: BaselineSource::Declared,
            unmanaged: Vec::new(),
            note: String::new(),
        }
    }

    fn person_baseline(email: &str, groups: &[&str]) -> PersonBaseline {
        PersonBaseline {
            email_hash: email_hash(email),
            device: groups.iter().map(|g| g.to_string()).collect(),
            handset: vec![format!("{P}-OBSERVER")],
        }
    }

    fn ory_index(email: &str, groups: &[&str]) -> OryPrivilegeIndex {
        OryPrivilegeIndex::one(email, groups)
    }

    #[test]
    fn local_manifests_supply_only_active_owner_joins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("alice.yml"),
            "identity:\n  handle: alice\n  email: Alice@Example.Test\n  status: active\nnetbird:\n  auto_groups: [DEVELOPER]\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("inactive.yml"),
            "identity:\n  handle: inactive\n  email: inactive@example.test\n  status: inactive\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("revoked.yml"),
            "identity:\n  handle: revoked\n  email: revoked@example.test\n  status: revoked\n",
        )
        .unwrap();
        let people = people_from_manifests(dir.path()).unwrap();
        assert_eq!(people.len(), 1);
        let alice = &people["alice"];
        assert_eq!(alice.email_hash, email_hash("alice@example.test"));
        assert!(alice.device.is_empty() && alice.handset.is_empty());
    }

    #[test]
    fn local_manifest_missing_email_or_malformed_yaml_is_refused() {
        let missing = tempfile::tempdir().unwrap();
        std::fs::write(
            missing.path().join("alice.yml"),
            "identity:\n  handle: alice\n  status: active\n",
        )
        .unwrap();
        assert!(people_from_manifests(missing.path())
            .unwrap_err()
            .to_string()
            .contains("no identity.email"));

        let malformed = tempfile::tempdir().unwrap();
        std::fs::write(malformed.path().join("alice.yml"), "identity: [not: valid").unwrap();
        assert!(people_from_manifests(malformed.path())
            .unwrap_err()
            .to_string()
            .contains("parse operator manifest"));
    }

    #[test]
    fn generated_baseline_wins_and_conflicting_manifest_join_is_ambiguous() {
        let baseline = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let same = merge_people(
            baseline.clone(),
            BTreeMap::from([
                (
                    "alice".to_string(),
                    PersonBaseline {
                        email_hash: email_hash("alice@example.test"),
                        ..Default::default()
                    },
                ),
                (
                    "bob".to_string(),
                    PersonBaseline {
                        email_hash: email_hash("bob@example.test"),
                        ..Default::default()
                    },
                ),
            ]),
        );
        assert_eq!(same["alice"].device, vec!["ZAGROS-DEV-DEVELOPER"]);
        assert!(same["bob"].device.is_empty() && same["bob"].handset.is_empty());

        let conflicted = merge_people(
            baseline,
            BTreeMap::from([(
                "alice".to_string(),
                PersonBaseline {
                    email_hash: email_hash("other@example.test"),
                    ..Default::default()
                },
            )]),
        );
        let peer = peer(
            "p1",
            "peer-alice-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let refusal = quarantine_refusal(
            &peer,
            &set(&["ZAGROS-DEV-QUARANTINED"]),
            S,
            P,
            &conflicted,
            None,
            Ok(&ory_index("alice@example.test", &["DEVELOPER"])),
        )
        .unwrap();
        assert!(refusal.contains("join is ambiguous"));
    }

    #[test]
    fn malformed_ory_groups_fail_closed() {
        let mut index = OryPrivilegeIndex::default();
        assert!(absorb_ory_page(
            &mut index,
            r#"[{"traits":{"email":"alice@example.test"},"metadata_public":{"groups":"PLATFORM-ADMIN"}}]"#,
        )
        .is_err());
        assert!(absorb_ory_page(
            &mut index,
            r#"[{"traits":{"email":"alice@example.test"},"metadata_public":{"groups":["DEVELOPER",7]}}]"#,
        )
        .is_err());
    }

    #[test]
    fn ory_projection_discards_sensitive_fields_and_indexes_each_page() {
        let mut index = OryPrivilegeIndex::default();
        let body = r#"[
          {
            "traits":{"email":"Alice@Example.Test","groups":["ADMINS"]},
            "metadata_public":{"groups":["PLATFORM-ADMIN"]},
            "credentials":{"totp":{"config":{"secret":"must-not-survive"}}},
            "recovery_addresses":[{"value":"alice@example.test"}]
          }
        ]"#;
        assert_eq!(absorb_ory_page(&mut index, body).unwrap(), 1);
        assert_eq!(
            index.groups_for(&email_hash("alice@example.test")).unwrap(),
            &set(&["PLATFORM-ADMIN"])
        );
    }

    #[test]
    fn direct_ory_quarantine_checks_identity_when_mesh_has_no_mover() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let peers = vec![peer(
            "p1",
            "peer-alice-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-QUARANTINED"],
        )];
        let idx = ory_index("alice@example.test", &["IAM-ADMIN"]);
        assert!(ory_identity_quarantine_refusal(
            "alice@example.test",
            S,
            P,
            &people,
            &peers,
            &BTreeMap::new(),
            Ok(&idx),
        )
        .unwrap()
        .contains("privileged person"));
    }

    #[test]
    fn direct_ory_quarantine_checks_live_and_declared_mesh_privilege() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-PLATFORM-ADMIN"]),
        )]);
        let idx = ory_index("alice@example.test", &["DEVELOPER"]);
        assert!(ory_identity_quarantine_refusal(
            "alice@example.test",
            S,
            P,
            &people,
            &[],
            &BTreeMap::new(),
            Ok(&idx),
        )
        .is_some());

        let ordinary_people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let peers = vec![peer(
            "p1",
            "peer-alice-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-PLATFORM-ADMIN"],
        )];
        assert!(ory_identity_quarantine_refusal(
            "alice@example.test",
            S,
            P,
            &ordinary_people,
            &peers,
            &BTreeMap::new(),
            Ok(&idx),
        )
        .is_some());
    }

    #[test]
    fn runtime_convergence_marks_privileged_quarantine_drift_blocked() {
        let privileged = peer(
            "p1",
            "peer-alice-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let ordinary = peer("p2", "machine-sensor", &["ZAGROS-DEV-SENSOR"]);
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let baselines = BTreeMap::from([
            ("p1".to_string(), key(&["ZAGROS-DEV-DEVELOPER"])),
            ("p2".to_string(), key(&["ZAGROS-DEV-SENSOR"])),
        ]);
        let mut plan = Plan {
            rows: vec![
                Row {
                    peer: privileged.name.clone(),
                    id: privileged.id.clone(),
                    current: vec!["ZAGROS-DEV-DEVELOPER".into()],
                    desired: Some(vec!["ZAGROS-DEV-QUARANTINED".into()]),
                    add: vec!["ZAGROS-DEV-QUARANTINED".into()],
                    remove: vec!["ZAGROS-DEV-DEVELOPER".into()],
                    status: Status::Drift,
                    baseline_source: BaselineSource::Declared,
                    unmanaged: Vec::new(),
                    note: String::new(),
                },
                Row {
                    peer: ordinary.name.clone(),
                    id: ordinary.id.clone(),
                    current: vec!["ZAGROS-DEV-SENSOR".into()],
                    desired: Some(vec!["ZAGROS-DEV-QUARANTINED".into()]),
                    add: vec!["ZAGROS-DEV-QUARANTINED".into()],
                    remove: vec!["ZAGROS-DEV-SENSOR".into()],
                    status: Status::Drift,
                    baseline_source: BaselineSource::Key,
                    unmanaged: Vec::new(),
                    note: String::new(),
                },
            ],
        };
        let idx = ory_index("alice@example.test", &["PLATFORM-ADMIN"]);
        assert_eq!(
            guard_quarantine_plan(
                &mut plan,
                &[privileged, ordinary],
                S,
                P,
                &people,
                &baselines,
                Ok(&idx),
            ),
            1
        );
        assert_eq!(plan.rows[0].status, Status::Blocked);
        assert!(plan.rows[0].note.contains("Nothing was changed"));
        assert_eq!(plan.rows[1].status, Status::Drift);
        assert_eq!(plan.changes_planned(), 2);
        assert_eq!(
            applicable_rows(&plan)
                .map(|row| row.add.len() + row.remove.len())
                .sum::<usize>(),
            2
        );
        assert!(privilege_exit(Mode::Report, 1).is_none());
        let refusal = privilege_exit(Mode::Enforce, 1).unwrap();
        assert_eq!(refusal.code, EXIT_PRIVILEGED_QUARANTINE);
        assert!(refusal.msg.contains("other rows converged normally"));
    }

    #[test]
    fn every_ory_admin_alias_refuses_security_quarantine() {
        let name = "peer-alice-zagros-dev-2026-09-01-laptop";
        let p = peer("p1", name, &["ZAGROS-DEV-DEVELOPER"]);
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let desired = set(&["ZAGROS-DEV-QUARANTINED"]);
        for role in ["PLATFORM-ADMIN", "IAM-ADMIN", "ADMINS"] {
            let idx = ory_index("alice@example.test", &[role]);
            let got = quarantine_refusal(&p, &desired, S, P, &people, None, Ok(&idx));
            assert!(got.unwrap().contains("privileged person"), "role {role}");
        }

        let quarantined = peer("p1", name, &["ZAGROS-DEV-QUARANTINED"]);
        let restore = set(&["ZAGROS-DEV-DEVELOPER"]);
        let idx = ory_index("alice@example.test", &["IAM-ADMIN"]);
        assert!(
            quarantine_refusal(&quarantined, &restore, S, P, &people, None, Ok(&idx),)
                .unwrap()
                .contains("privileged person")
        );
    }

    #[test]
    fn mesh_or_declared_platform_admin_refuses_quarantine() {
        let name = "peer-alice-zagros-dev-2026-09-01-laptop";
        let desired = set(&["ZAGROS-DEV-QUARANTINED"]);
        let idx = ory_index("alice@example.test", &["DEVELOPER"]);
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let live_admin = peer("p1", name, &["ZAGROS-DEV-PLATFORM-ADMIN"]);
        assert!(quarantine_refusal(&live_admin, &desired, S, P, &people, None, Ok(&idx)).is_some());

        let ordinary = peer("p1", name, &["ZAGROS-DEV-DEVELOPER"]);
        let declared_admin = key(&["ZAGROS-DEV-PLATFORM-ADMIN"]);
        assert!(quarantine_refusal(
            &ordinary,
            &desired,
            S,
            P,
            &people,
            Some(&declared_admin),
            Ok(&idx),
        )
        .is_some());
    }

    #[test]
    fn doubled_handle_owner_allows_ordinary_quarantine() {
        let p = peer(
            "p1",
            "peer-user371x-user371x-zagros-2026-06-16-JL-2.local",
            &["ZAGROS-DEVELOPER"],
        );
        let people = BTreeMap::from([(
            "user371x".to_string(),
            PersonBaseline {
                email_hash: email_hash("jonas@example.test"),
                device: vec!["ZAGROS-DEVELOPER".into()],
                handset: vec!["ZAGROS-OBSERVER".into()],
            },
        )]);
        let idx = ory_index("jonas@example.test", &["DEVELOPER"]);
        assert!(quarantine_refusal(
            &p,
            &set(&["ZAGROS-QUARANTINED"]),
            "zagros",
            "ZAGROS",
            &people,
            None,
            Ok(&idx),
        )
        .is_none());
    }

    #[test]
    fn doubled_handle_owner_refuses_privileged_quarantine() {
        let p = peer(
            "p1",
            "peer-user371x-user371x-zagros-2026-06-16-JL-2.local",
            &["ZAGROS-DEVELOPER"],
        );
        let desired = set(&["ZAGROS-QUARANTINED"]);
        for role in ["PLATFORM-ADMIN", "IAM-ADMIN", "ADMINS"] {
            let people = BTreeMap::from([(
                "user371x".to_string(),
                PersonBaseline {
                    email_hash: email_hash("jonas@example.test"),
                    device: vec!["ZAGROS-DEVELOPER".into()],
                    handset: vec!["ZAGROS-OBSERVER".into()],
                },
            )]);
            let idx = ory_index("jonas@example.test", &[role]);
            let refusal =
                quarantine_refusal(&p, &desired, "zagros", "ZAGROS", &people, None, Ok(&idx)).unwrap();
            assert!(refusal.contains("sever the account"), "role {role}");
            assert!(
                !refusal.contains("Privilege cannot be ruled out"),
                "role {role}"
            );
        }

        let people = BTreeMap::from([(
            "user371x".to_string(),
            PersonBaseline {
                email_hash: email_hash("jonas@example.test"),
                device: vec!["ZAGROS-PLATFORM-ADMIN".into()],
                handset: vec!["ZAGROS-OBSERVER".into()],
            },
        )]);
        let idx = ory_index("jonas@example.test", &["DEVELOPER"]);
        let refusal =
            quarantine_refusal(&p, &desired, "zagros", "ZAGROS", &people, None, Ok(&idx)).unwrap();
        assert!(refusal.contains("sever the account"));
        assert!(!refusal.contains("Privilege cannot be ruled out"));
    }

    #[test]
    fn doubled_handle_does_not_accept_different_multitoken_org() {
        let people = BTreeMap::from([(
            "user371x".to_string(),
            person_baseline("jonas@example.test", &["ZAGROS-DEVELOPER"]),
        )]);
        assert_eq!(
            declared_person_candidates(
                "peer-user371x-user371x-zagros-2026-06-16-x",
                "zagros",
                &people,
            )
            .len(),
            1
        );
        let p = peer(
            "p1",
            "peer-user371x-acme-corp-zagros-2026-06-16-x",
            &["ZAGROS-DEVELOPER"],
        );
        let idx = ory_index("jonas@example.test", &["DEVELOPER"]);
        let refusal = quarantine_refusal(
            &p,
            &set(&["ZAGROS-QUARANTINED"]),
            "zagros",
            "ZAGROS",
            &people,
            None,
            Ok(&idx),
        )
        .unwrap();
        assert!(refusal.contains("no unique declared person owns this person-shaped peer"));
        assert!(refusal.contains("Privilege cannot be ruled out"));
    }

    #[test]
    fn owner_lookup_unavailable_fails_closed_but_proven_ambiguity_is_allowed() {
        let name = "peer-maintainer-smith-zagros-dev-2026-09-01-laptop";
        let p = peer("p1", name, &["ZAGROS-DEV-DEVELOPER"]);
        let desired = set(&["ZAGROS-DEV-QUARANTINED"]);
        let one = BTreeMap::from([(
            "maintainer-smith".to_string(),
            person_baseline("owner@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let unavailable =
            quarantine_refusal(&p, &desired, S, P, &one, None, Err("lookup unavailable")).unwrap();
        assert!(unavailable.contains("unavailable"));
        assert!(unavailable.contains("Privilege cannot be ruled out"));
        assert!(!unavailable.contains("sever the account"));

        let mut two = one;
        two.insert(
            "maintainer".into(),
            person_baseline("other@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        );
        let mut idx = ory_index("owner@example.test", &["DEVELOPER"]);
        idx.by_email_hash
            .insert(email_hash("other@example.test"), vec![set(&["DEVELOPER"])]);
        assert!(quarantine_refusal(&p, &desired, S, P, &two, None, Ok(&idx)).is_none());

        let outside_checkout = quarantine_refusal_with_owner_reason(
            &p,
            &desired,
            S,
            P,
            OwnerJoin {
                people: &BTreeMap::new(),
                missing_reason: Some(
                    "no owner join is available because the command is not running from a repo checkout",
                ),
            },
            None,
            Err("not queried"),
        )
        .unwrap();
        assert!(outside_checkout.contains("not running from a repo checkout"));
        assert!(!outside_checkout.contains("sever the account"));
    }

    #[test]
    fn ambiguous_owner_candidates_fail_closed_unless_all_are_proven_ordinary() {
        let p = peer(
            "p1",
            "peer-maintainer-smith-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let desired = set(&["ZAGROS-DEV-QUARANTINED"]);
        let ordinary = person_baseline("maintainer@example.test", &["ZAGROS-DEV-DEVELOPER"]);
        let privileged = person_baseline("maintainer-smith@example.test", &["ZAGROS-DEV-PLATFORM-ADMIN"]);
        let people = BTreeMap::from([
            ("maintainer".to_string(), ordinary.clone()),
            ("maintainer-smith".to_string(), privileged),
        ]);
        let mut idx = ory_index("maintainer@example.test", &["DEVELOPER"]);
        idx.by_email_hash.insert(
            email_hash("maintainer-smith@example.test"),
            vec![set(&["DEVELOPER"])],
        );
        let refusal = quarantine_refusal(&p, &desired, S, P, &people, None, Ok(&idx)).unwrap();
        assert!(refusal.contains("one possible owner is privileged"));
        assert!(refusal.contains("Privilege cannot be ruled out"));
        assert!(!refusal.contains("sever the account"));

        let mut unproven_before_privileged = people.clone();
        unproven_before_privileged
            .get_mut("maintainer")
            .unwrap()
            .email_hash = String::new();
        let refusal = quarantine_refusal(
            &p,
            &desired,
            S,
            P,
            &unproven_before_privileged,
            None,
            Ok(&idx),
        )
        .unwrap();
        assert!(refusal.contains("one possible owner is privileged"));

        let mut unavailable = people.clone();
        unavailable.insert(
            "maintainer-smith".into(),
            person_baseline("missing@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        );
        assert!(
            quarantine_refusal(&p, &desired, S, P, &unavailable, None, Ok(&idx))
                .unwrap()
                .contains("lookup was unavailable or ambiguous")
        );

        for (bad_hash, expected) in [
            (String::new(), "no deployable Ory identity join"),
            (
                AMBIGUOUS_EMAIL_HASH.to_string(),
                "baseline and local operator manifest disagree",
            ),
        ] {
            let mut unproven = unavailable.clone();
            unproven.get_mut("maintainer-smith").unwrap().email_hash = bad_hash;
            let refusal =
                quarantine_refusal(&p, &desired, S, P, &unproven, None, Ok(&idx)).unwrap();
            assert!(refusal.contains("Privilege cannot be ruled out"));
            assert!(refusal.contains(expected));
        }
    }

    #[test]
    fn real_handle_prefix_shape_allows_two_proven_ordinary_candidates() {
        let p = peer(
            "p1",
            "peer-user162x-zagros-2026-09-01-laptop",
            &["ZAGROS-DEVELOPER"],
        );
        let desired = set(&["ZAGROS-QUARANTINED"]);
        let people = BTreeMap::from([
            (
                "user253x".to_string(),
                PersonBaseline {
                    email_hash: email_hash("user253x@example.test"),
                    device: vec!["ZAGROS-DEVELOPER".into()],
                    handset: vec!["ZAGROS-OBSERVER".into()],
                },
            ),
            (
                "user162x".to_string(),
                PersonBaseline {
                    email_hash: email_hash("user162x@example.test"),
                    device: vec!["ZAGROS-DEVELOPER".into()],
                    handset: vec!["ZAGROS-OBSERVER".into()],
                },
            ),
        ]);
        let mut idx = ory_index("user253x@example.test", &["DEVELOPER"]);
        idx.by_email_hash.insert(
            email_hash("user162x@example.test"),
            vec![set(&["DEVELOPER"])],
        );
        assert!(
            quarantine_refusal(&p, &desired, "zagros", "ZAGROS", &people, None, Ok(&idx)).is_none()
        );
    }

    #[test]
    fn identity_shared_hash_allows_ordinary_owners_and_blocks_possible_admin() {
        let ordinary = BTreeMap::from([
            (
                "first".to_string(),
                person_baseline("shared@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
            (
                "second".to_string(),
                person_baseline("shared@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
        ]);
        let idx = ory_index("shared@example.test", &["DEVELOPER"]);
        assert!(ory_identity_quarantine_refusal(
            "shared@example.test",
            S,
            P,
            &ordinary,
            &[],
            &BTreeMap::new(),
            Ok(&idx),
        )
        .is_none());

        let mut with_admin = ordinary;
        with_admin.get_mut("second").unwrap().device = vec!["ZAGROS-DEV-PLATFORM-ADMIN".into()];
        let refusal = ory_identity_quarantine_refusal(
            "shared@example.test",
            S,
            P,
            &with_admin,
            &[],
            &BTreeMap::new(),
            Ok(&idx),
        )
        .unwrap();
        assert!(refusal.contains("one possible owner is privileged"));
        assert!(!refusal.contains("sever the account"));

        let refusal = ory_identity_quarantine_refusal(
            "shared@example.test",
            S,
            P,
            &with_admin,
            &[],
            &BTreeMap::new(),
            Err("lookup unavailable"),
        )
        .unwrap();
        assert!(refusal.contains("one possible owner is privileged"));
    }

    #[test]
    fn identity_path_evaluates_every_candidate_for_a_matching_live_peer() {
        let people = BTreeMap::from([
            (
                "maintainer".to_string(),
                person_baseline("maintainer@example.test", &["ZAGROS-DEV-PLATFORM-ADMIN"]),
            ),
            (
                "maintainer-smith".to_string(),
                person_baseline("target@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
        ]);
        let peers = vec![peer(
            "p1",
            "peer-maintainer-smith-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        )];
        let mut idx = ory_index("target@example.test", &["DEVELOPER"]);
        idx.by_email_hash
            .insert(email_hash("maintainer@example.test"), vec![set(&["DEVELOPER"])]);

        let refusal = ory_identity_quarantine_refusal(
            "target@example.test",
            S,
            P,
            &people,
            &peers,
            &BTreeMap::new(),
            Ok(&idx),
        )
        .unwrap();
        assert!(refusal.contains("one possible owner is privileged"));
        assert!(!refusal.contains("sever the account"));
    }

    #[test]
    fn identity_path_defers_unique_privilege_until_peer_ambiguity_is_known() {
        let people = BTreeMap::from([
            (
                "maintainer".to_string(),
                person_baseline("maintainer@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
            (
                "maintainer-smith".to_string(),
                person_baseline(
                    "target@example.test",
                    &["ZAGROS-DEV-PLATFORM-ADMIN"],
                ),
            ),
        ]);
        let peers = vec![peer(
            "p1",
            "peer-maintainer-smith-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        )];
        let mut idx = ory_index("target@example.test", &["DEVELOPER"]);
        idx.by_email_hash
            .insert(email_hash("maintainer@example.test"), vec![set(&["DEVELOPER"])]);

        let refusal = ory_identity_quarantine_refusal(
            "target@example.test",
            S,
            P,
            &people,
            &peers,
            &BTreeMap::new(),
            Ok(&idx),
        )
        .unwrap();
        assert!(refusal.contains("one possible owner is privileged"));
        assert!(!refusal.contains("sever the account"));
    }

    #[test]
    fn converger_allows_proven_ordinary_ambiguity_and_blocks_possible_admin() {
        let ordinary_peer = peer(
            "p1",
            "peer-maintainer-smith-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let privileged_peer = peer(
            "p2",
            "peer-jane-doe-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let people = BTreeMap::from([
            (
                "maintainer".to_string(),
                person_baseline("maintainer@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
            (
                "maintainer-smith".to_string(),
                person_baseline("maintainer-smith@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
            (
                "jane".to_string(),
                person_baseline("jane@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
            (
                "jane-doe".to_string(),
                person_baseline("jane-doe@example.test", &["ZAGROS-DEV-PLATFORM-ADMIN"]),
            ),
        ]);
        let mut idx = OryPrivilegeIndex::default();
        for email in [
            "maintainer@example.test",
            "maintainer-smith@example.test",
            "jane@example.test",
            "jane-doe@example.test",
        ] {
            idx.by_email_hash
                .insert(email_hash(email), vec![set(&["DEVELOPER"])]);
        }
        let mut plan = Plan {
            rows: vec![
                quarantine_row(&ordinary_peer),
                quarantine_row(&privileged_peer),
            ],
        };
        assert_eq!(
            guard_quarantine_plan(
                &mut plan,
                &[ordinary_peer, privileged_peer],
                S,
                P,
                &people,
                &BTreeMap::new(),
                Ok(&idx),
            ),
            1
        );
        assert_eq!(plan.rows[0].status, Status::Drift);
        assert_eq!(plan.rows[1].status, Status::Blocked);
        assert!(plan.rows[1]
            .note
            .contains("one possible owner is privileged"));
    }

    #[test]
    fn converger_handles_doubled_legacy_owner() {
        let ordinary_peer = peer(
            "p1",
            "peer-user371x-user371x-zagros-dev-2026-06-16-JL-2.local",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let privileged_peer = peer(
            "p2",
            "peer-contributora-contributora-zagros-dev-2026-06-14-mac.local",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        let people = BTreeMap::from([
            (
                "user371x".to_string(),
                person_baseline("jonas@example.test", &["ZAGROS-DEV-DEVELOPER"]),
            ),
            (
                "contributora".to_string(),
                person_baseline("user8x@example.test", &["ZAGROS-DEV-PLATFORM-ADMIN"]),
            ),
        ]);
        let mut idx = ory_index("jonas@example.test", &["DEVELOPER"]);
        idx.by_email_hash
            .insert(email_hash("user8x@example.test"), vec![set(&["DEVELOPER"])]);
        let mut plan = Plan {
            rows: vec![
                quarantine_row(&ordinary_peer),
                quarantine_row(&privileged_peer),
            ],
        };
        assert_eq!(
            guard_quarantine_plan(
                &mut plan,
                &[ordinary_peer, privileged_peer],
                S,
                P,
                &people,
                &BTreeMap::new(),
                Ok(&idx),
            ),
            1
        );
        assert_eq!(plan.rows[0].status, Status::Drift);
        assert_eq!(plan.rows[1].status, Status::Blocked);
        assert!(plan.rows[1].note.contains("sever the account"));
        assert!(!plan.rows[1].note.contains("Privilege cannot be ruled out"));
    }

    #[test]
    fn non_privileged_person_and_machine_keep_quarantine_workflow() {
        let desired = set(&["ZAGROS-DEV-QUARANTINED"]);
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-DEVELOPER"]),
        )]);
        let idx = ory_index("alice@example.test", &["DEVELOPER"]);
        let person = peer(
            "p1",
            "peer-alice-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-DEVELOPER"],
        );
        assert!(quarantine_refusal(&person, &desired, S, P, &people, None, Ok(&idx)).is_none());

        let machine = peer("m1", "machine-sensor-1", &["ZAGROS-DEV-SENSOR"]);
        assert!(quarantine_refusal(
            &machine,
            &desired,
            S,
            P,
            &people,
            Some(&key(&["ZAGROS-DEV-SENSOR"])),
            Err("not queried"),
        )
        .is_none());
    }

    #[test]
    fn manifest_only_person_uses_live_ory_privilege() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &[]),
        )]);
        let person = peer(
            "p1",
            "peer-alice-zagros-prod-2026-09-01-laptop",
            &["ZAGROS-PROD-DEVELOPER"],
        );
        let desired = set(&["ZAGROS-PROD-QUARANTINED"]);
        assert!(quarantine_refusal(
            &person,
            &desired,
            "zagros-prod",
            "ZAGROS-PROD",
            &people,
            None,
            Ok(&ory_index("alice@example.test", &["DEVELOPER"])),
        )
        .is_none());
        let privileged = quarantine_refusal(
            &person,
            &desired,
            "zagros-prod",
            "ZAGROS-PROD",
            &people,
            None,
            Ok(&ory_index("alice@example.test", &["IAM-ADMIN"])),
        )
        .unwrap();
        assert!(privileged.contains("sever the account"));
    }

    #[test]
    fn privileged_dormancy_release_is_not_a_security_transition() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person_baseline("alice@example.test", &["ZAGROS-DEV-PLATFORM-ADMIN"]),
        )]);
        let p = peer(
            "p1",
            "peer-alice-zagros-dev-2026-09-01-laptop",
            &["ZAGROS-DEV-PROBATION-PLATFORM-ADMIN"],
        );
        let desired = set(&["ZAGROS-DEV-PLATFORM-ADMIN"]);
        assert!(quarantine_refusal(
            &p,
            &desired,
            S,
            P,
            &people,
            Some(&key(&["ZAGROS-DEV-PLATFORM-ADMIN"])),
            Err("Ory is irrelevant to dormancy release"),
        )
        .is_none());
    }

    #[test]
    fn baseline_with_no_entry_is_the_desired_set() {
        let d = decl("report", " []");
        let peers = vec![
            peer("a", "peer-a", &["All", "ZAGROS-DEV-DEVELOPER"]),
            peer(
                "b",
                "peer-b",
                &["All", "ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-P2P"],
            ),
        ];
        let base = BTreeMap::from([
            ("a".to_string(), key(&["ZAGROS-DEV-DEVELOPER"])),
            ("b".to_string(), key(&["ZAGROS-DEV-DEVELOPER"])),
        ]);
        let plan = plan_membership(&d, P, &[], &peers, &base, &all_groups()).unwrap();
        assert_eq!(row(&plan, "peer-a").status, Status::Match);
        let b = row(&plan, "peer-b");
        assert_eq!(b.status, Status::Drift);
        assert_eq!(b.remove, vec!["ZAGROS-DEV-P2P"]);
        assert!(b.add.is_empty());
        assert_eq!(plan.changes_planned(), 1);
    }

    #[test]
    fn grant_and_absent_edit_the_baseline() {
        let d = decl(
            "enforce",
            "\n  - peer: peer-a\n    grant: [ZAGROS-DEV-P2P]\n    absent: [ZAGROS-DEV-OBSERVER]\n    owner: a\n    reason: x",
        );
        let peers = vec![peer(
            "a",
            "peer-a",
            &["All", "ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-OBSERVER"],
        )];
        let base = BTreeMap::from([(
            "a".to_string(),
            key(&["ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-OBSERVER"]),
        )]);
        let plan = plan_membership(&d, P, &[], &peers, &base, &all_groups()).unwrap();
        let a = row(&plan, "peer-a");
        assert_eq!(a.add, vec!["ZAGROS-DEV-P2P"]);
        assert_eq!(a.remove, vec!["ZAGROS-DEV-OBSERVER"]);
        assert_eq!(
            a.desired.as_deref().unwrap(),
            ["ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-P2P"]
        );
    }

    #[test]
    fn exact_replaces_the_baseline_even_when_unknown() {
        let d = decl(
            "enforce",
            "\n  - peer: vendor-registry-01\n    exact: [ZAGROS-DEV-EXTERNAL-INFRA]\n    owner: user369x\n    reason: x\n    expires: 2026-12-31",
        );
        let peers = vec![peer(
            "v",
            "vendor-registry-01",
            &["All", "ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-EXTERNAL-INFRA"],
        )];
        let plan = plan_membership(&d, P, &[], &peers, &BTreeMap::new(), &all_groups()).unwrap();
        let v = row(&plan, "vendor-registry-01");
        assert_eq!(v.status, Status::Drift);
        assert_eq!(v.remove, vec!["ZAGROS-DEV-DEVELOPER"]);
        assert!(v.add.is_empty());
    }

    #[test]
    fn hold_is_quarantined_alone() {
        let d = decl(
            "enforce",
            "\n  - peer: peer-x\n    hold: quarantine\n    owner: user369x\n    reason: flood\n    expires: 2026-09-30",
        );
        let peers = vec![peer(
            "x",
            "peer-x",
            &["All", "ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-P2P", "mesh-dns"],
        )];
        let base = BTreeMap::from([("x".to_string(), key(&["ZAGROS-DEV-DEVELOPER"]))]);
        let plan = plan_membership(&d, P, &[], &peers, &base, &all_groups()).unwrap();
        let x = row(&plan, "peer-x");
        assert_eq!(x.add, vec!["ZAGROS-DEV-QUARANTINED"]);
        assert_eq!(x.remove, vec!["ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-P2P"]);
        // Unmanaged groups are reported, never in add/remove.
        assert_eq!(x.unmanaged, vec!["All", "mesh-dns"]);
    }

    #[test]
    fn unknown_baseline_is_never_changed() {
        let d = decl(
            "enforce",
            "\n  - peer: peer-g\n    grant: [ZAGROS-DEV-P2P]\n    owner: g\n    reason: x",
        );
        let peers = vec![
            peer("u", "peer-u", &["All", "ZAGROS-DEV-DEVELOPER"]),
            peer("g", "peer-g", &["All", "ZAGROS-DEV-DEVELOPER"]),
        ];
        let plan = plan_membership(&d, P, &[], &peers, &BTreeMap::new(), &all_groups()).unwrap();
        for name in ["peer-u", "peer-g"] {
            let r = row(&plan, name);
            assert_eq!(r.status, Status::Unknown, "{name}");
            assert!(r.add.is_empty() && r.remove.is_empty());
            assert!(r.desired.is_none());
        }
        assert_eq!(plan.changes_planned(), 0);
        assert_eq!(plan.unknown(), 2);
    }

    #[test]
    fn other_prefixes_and_all_are_untouched() {
        let d = decl("enforce", " []");
        // Scope `zagros` on a server that also hosts `zagros-dev`: ZAGROS-DEV-* is not ours.
        let peers = vec![peer(
            "a",
            "peer-a",
            &[
                "All",
                "mesh-dns",
                "EX-SVN-OBSERVER",
                "ZAGROS-DEVELOPER",
                "ZAGROS-DEV-OBSERVER",
            ],
        )];
        let base = BTreeMap::from([("a".to_string(), key(&["ZAGROS-DEVELOPER"]))]);
        let d_zagros = MembershipDecl {
            scope: "zagros".into(),
            ..d
        };
        let plan = plan_membership(
            &d_zagros,
            "ZAGROS",
            &["ZAGROS-DEV".to_string()],
            &peers,
            &base,
            &live(&["ZAGROS-DEVELOPER"]),
        )
        .unwrap();
        let a = row(&plan, "peer-a");
        assert_eq!(a.status, Status::Match);
        assert_eq!(a.current, vec!["ZAGROS-DEVELOPER"]);
        assert!(a.unmanaged.contains(&"ZAGROS-DEV-OBSERVER".to_string()));
        assert!(a.unmanaged.contains(&"All".to_string()));
    }

    #[test]
    fn entry_for_a_missing_peer_is_reported() {
        let d = decl(
            "report",
            "\n  - peer: ghost\n    grant: [ZAGROS-DEV-P2P]\n    owner: g\n    reason: x",
        );
        let plan = plan_membership(&d, P, &[], &[], &BTreeMap::new(), &all_groups()).unwrap();
        assert_eq!(plan.missing_peer(), 1);
        assert_eq!(plan.peers_total(), 0);
        assert_eq!(row(&plan, "ghost").status, Status::MissingPeer);
    }

    #[test]
    fn coverage_gate_blocks_an_add_into_an_uncovered_or_absent_group() {
        let d = decl(
            "enforce",
            "\n  - peer: peer-a\n    grant: [ZAGROS-DEV-P2P]\n    owner: a\n    reason: x",
        );
        let peers = vec![peer("a", "peer-a", &["ZAGROS-DEV-DEVELOPER"])];
        let base = BTreeMap::from([("a".to_string(), key(&["ZAGROS-DEV-DEVELOPER"]))]);
        let mut groups = all_groups();
        groups.insert("ZAGROS-DEV-P2P".into(), GroupState { covered: false });
        let plan = plan_membership(&d, P, &[], &peers, &base, &groups).unwrap();
        assert_eq!(row(&plan, "peer-a").status, Status::Blocked);
        assert_eq!(plan.changes_planned(), 0);
        assert_eq!(plan.drift(), 1);
        groups.remove("ZAGROS-DEV-P2P");
        let plan = plan_membership(&d, P, &[], &peers, &base, &groups).unwrap();
        assert!(row(&plan, "peer-a").note.contains("does not exist"));
    }

    #[test]
    fn max_changes_refuses_over_the_limit() {
        assert!(check_max_changes(10, 10).is_ok());
        let e = check_max_changes(11, 10).unwrap_err();
        assert_eq!(e.code, EXIT_MAX_CHANGES);
        assert!(e.msg.contains("Nothing was changed"));
    }

    #[test]
    fn mode_ceiling_is_the_file() {
        assert_eq!(
            effective_mode(Mode::Report, Converge::Enforce).unwrap(),
            Mode::Report
        );
        assert_eq!(
            effective_mode(Mode::Enforce, Converge::Enforce).unwrap(),
            Mode::Enforce
        );
        assert_eq!(
            effective_mode(Mode::Report, Converge::Report).unwrap(),
            Mode::Report
        );
        assert_eq!(
            effective_mode(Mode::Enforce, Converge::Report)
                .unwrap_err()
                .code,
            EXIT_MODE_CEILING
        );
    }

    #[test]
    fn declaration_is_checked() {
        let bad = |entries: &str| {
            let text = format!("version: 1\nentries:{entries}\n");
            parse_fragment(&text, "zagros-dev").is_err()
        };
        // two forms
        assert!(bad("\n  - peer: a\n    exact: [ZAGROS-DEV-P2P]\n    grant: [ZAGROS-DEV-P2P]\n    owner: o\n    reason: r"));
        // typo'd field
        assert!(bad(
            "\n  - peer: a\n    grants: [ZAGROS-DEV-P2P]\n    owner: o\n    reason: r"
        ));
        // other prefix
        assert!(bad(
            "\n  - peer: a\n    grant: [ZAGROS-P2P]\n    owner: o\n    reason: r"
        ));
        // duplicate peer in one fragment
        assert!(bad("\n  - peer: a\n    hold: quarantine\n    owner: o\n    reason: r\n  - peer: a\n    hold: quarantine\n    owner: o\n    reason: r"));
        // unknown hold
        assert!(bad(
            "\n  - peer: a\n    hold: probation\n    owner: o\n    reason: r"
        ));
        // no owner
        assert!(bad("\n  - peer: a\n    hold: quarantine\n    reason: r"));
        // a fragment may not carry scope/converge
        assert!(parse_fragment(
            "version: 1\nscope: zagros-dev\nconverge: report\nentries: []\n",
            "zagros-dev"
        )
        .is_err());
        // _scope.yaml: wrong scope, entries in it, bad version
        assert!(parse_scope_file("version: 1\nscope: zagros\nconverge: report\n", "zagros-dev").is_err());
        assert!(parse_scope_file(
            "version: 1\nscope: zagros-dev\nconverge: report\nentries: []\n",
            "zagros-dev"
        )
        .is_err());
        assert!(parse_scope_file("version: 2\nscope: zagros-dev\nconverge: report\n", "zagros-dev").is_err());
        assert!(parse_scope_file("version: 1\nscope: zagros-dev\nconverge: report\n", "zagros-dev").is_ok());
    }

    /// Every scope directory shipped in the repo loads; their entries change
    /// with the goatnet, so only validity is pinned here. Until the data moves
    /// to the directory layout there may be none — then there is nothing to
    /// check.
    #[test]
    fn shipped_declarations_load() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/warrant/membership");
        let Ok(rd) = std::fs::read_dir(&root) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let scope = p.file_name().unwrap().to_str().unwrap().to_string();
                if let Err(err) = load_decl(&p, &scope) {
                    panic!("{}: {err:#}", p.display());
                }
            }
        }
    }

    // -- the directory layout ------------------------------------------------

    /// A scope directory from (relative path, body) pairs.
    fn scope_dir(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("zagros-dev");
        for (rel, body) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        (t, dir)
    }

    const SCOPE_REPORT: &str = "version: 1\nscope: zagros-dev\nconverge: report\n";

    fn frag(peer: &str, body: &str) -> String {
        format!("version: 1\nentries:\n  - peer: {peer}\n{body}    owner: o\n    reason: r\n")
    }

    #[test]
    fn directory_merges_fragments_recursively() {
        let (_t, dir) = scope_dir(&[
            ("_scope.yaml", SCOPE_REPORT),
            ("holds.yaml", &frag("peer-x", "    hold: quarantine\n")),
            (
                "orgs/acme.yaml",
                &frag("peer-a", "    grant: [ZAGROS-DEV-P2P]\n"),
            ),
            ("people.yaml", "version: 1\nentries: []\n"),
            ("README.md", "not a fragment"),
            (".people.yaml.tmp.1", "garbage"),
            (
                "_baseline.generated.yaml",
                "version: 1\nscope: zagros-dev\nidentity_hashes:\n  alice: ff8d9819fc0e12bf0d24892e45987e249a28dce836a85cad60e28eaaa8c6d976\npeople:\n  alice:\n    device: [ZAGROS-DEV-DEVELOPER]\n    handset: [ZAGROS-DEV-OBSERVER]\n",
            ),
        ]);
        let d = load_decl(&dir, "zagros-dev").unwrap();
        assert_eq!(d.converge, Converge::Report);
        let peers: BTreeSet<&str> = d.entries().iter().map(|e| e.peer.as_str()).collect();
        assert_eq!(peers, BTreeSet::from(["peer-a", "peer-x"]));
        assert_eq!(d.sources["peer-a"], PathBuf::from("orgs/acme.yaml"));
        assert_eq!(d.sources["peer-x"], PathBuf::from("holds.yaml"));
        assert_eq!(d.people["alice"].handset, vec!["ZAGROS-DEV-OBSERVER"]);
        assert_eq!(
            d.people["alice"].email_hash,
            "ff8d9819fc0e12bf0d24892e45987e249a28dce836a85cad60e28eaaa8c6d976"
        );
    }

    #[test]
    fn pre_migration_baseline_remains_readable_and_has_no_identity_join() {
        let (_t, dir) = scope_dir(&[
            ("_scope.yaml", SCOPE_REPORT),
            (
                "_baseline.generated.yaml",
                "version: 1\nscope: zagros-dev\npeople:\n  alice:\n    device: [ZAGROS-DEV-DEVELOPER]\n",
            ),
        ]);
        let d = load_decl(&dir, "zagros-dev").unwrap();
        assert!(d.people["alice"].email_hash.is_empty());
    }

    #[test]
    fn duplicate_peer_across_fragments_names_both_files() {
        let (_t, dir) = scope_dir(&[
            ("_scope.yaml", SCOPE_REPORT),
            ("holds.yaml", &frag("peer-a", "    hold: quarantine\n")),
            ("orgs/acme.yaml", &frag("peer-a", "    grant: [ZAGROS-DEV-P2P]\n")),
        ]);
        let e = format!("{:#}", load_decl(&dir, "zagros-dev").unwrap_err());
        assert!(e.contains("holds.yaml") && e.contains("orgs/acme.yaml"), "{e}");
    }

    #[test]
    fn single_file_layout_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("zagros-dev.yaml");
        std::fs::write(&file, "version: 1\nscope: zagros-dev\nconverge: report\nentries: []\n")
            .unwrap();
        for p in [file.clone(), t.path().join("zagros-dev")] {
            let e = format!("{:#}", load_decl(&p, "zagros-dev").unwrap_err());
            assert!(e.contains("retired single-file layout"), "{e}");
            assert!(e.contains("_scope.yaml"), "{e}");
        }
        // Governance reads the retired file as a declaration it cannot use.
        assert_eq!(
            repo_converge(&t.path().join("zagros-dev")),
            Some(FileConverge::Unreadable)
        );
    }

    #[test]
    fn stray_underscore_files_and_missing_scope_are_refused() {
        let (_t, dir) = scope_dir(&[
            ("_scope.yaml", SCOPE_REPORT),
            ("_notes.yaml", "version: 1\nentries: []\n"),
        ]);
        let e = format!("{:#}", load_decl(&dir, "zagros-dev").unwrap_err());
        assert!(e.contains("_notes.yaml"), "{e}");
        // _scope.yaml below the top is stray too.
        let (_t, dir) = scope_dir(&[("_scope.yaml", SCOPE_REPORT), ("sub/_scope.yaml", SCOPE_REPORT)]);
        assert!(load_decl(&dir, "zagros-dev").is_err());
        // A .yml fragment would be silently skipped: refused.
        let (_t, dir) = scope_dir(&[("_scope.yaml", SCOPE_REPORT), ("people.yml", "version: 1\n")]);
        assert!(load_decl(&dir, "zagros-dev").is_err());
        // No _scope.yaml.
        let (_t, dir) = scope_dir(&[("people.yaml", "version: 1\nentries: []\n")]);
        let e = format!("{:#}", load_decl(&dir, "zagros-dev").unwrap_err());
        assert!(e.contains("_scope.yaml"), "{e}");
        // A fragment carrying converge:.
        let (_t, dir) = scope_dir(&[
            ("_scope.yaml", SCOPE_REPORT),
            ("people.yaml", "version: 1\nconverge: enforce\nentries: []\n"),
        ]);
        assert!(load_decl(&dir, "zagros-dev").is_err());
        // A baseline for another scope, or naming an unmanaged group.
        for b in [
            "version: 1\nscope: zagros\npeople: {}\n",
            "version: 1\nscope: zagros-dev\npeople:\n  a:\n    device: [All]\n",
        ] {
            let (_t, dir) = scope_dir(&[("_scope.yaml", SCOPE_REPORT), ("_baseline.generated.yaml", b)]);
            assert!(load_decl(&dir, "zagros-dev").is_err(), "{b}");
        }
    }

    // -- baselines -----------------------------------------------------------

    fn people() -> BTreeMap<String, PersonBaseline> {
        BTreeMap::from([
            (
                "alice".to_string(),
                PersonBaseline {
                    email_hash: email_hash("alice@example.test"),
                    device: vec!["ZAGROS-DEV-DEVELOPER".into()],
                    handset: vec!["ZAGROS-DEV-OBSERVER".into()],
                },
            ),
            (
                "alice-smith".to_string(),
                PersonBaseline {
                    email_hash: email_hash("alice.smith@example.test"),
                    device: vec!["ZAGROS-DEV-P2P".into()],
                    handset: vec![],
                },
            ),
        ])
    }

    fn join(name: Option<&str>, groups: Option<&[&str]>) -> KeyJoin {
        KeyJoin {
            key_name: name.map(str::to_string),
            groups: groups.map(set),
        }
    }

    #[test]
    fn declared_baseline_wins_over_the_key() {
        let p = people();
        let kj = join(Some("peer-alice-zagros-dev"), Some(&["ZAGROS-DEV-OBSERVER"]));
        let b = resolve_baseline("peer-alice-zagros-dev-2026-09-01-laptop", S, &p, Some(&kj)).unwrap();
        assert_eq!(b.source, BaselineSource::Declared);
        assert_eq!(b.groups, set(&["ZAGROS-DEV-DEVELOPER"]));
        // Longest handle wins: alice-smith is not alice.
        let b = resolve_baseline("peer-alice-smith-zagros-dev-2026-09-01-mac", S, &p, None).unwrap();
        assert_eq!(b.groups, set(&["ZAGROS-DEV-P2P"]));
    }

    #[test]
    fn handset_takes_the_handset_list() {
        let p = people();
        // By the minting key's name.
        let kj = join(Some("peer-alice-zagros-dev-handset"), None);
        let b = resolve_baseline("peer-alice-zagros-dev-2026-09-01-iPhone", S, &p, Some(&kj)).unwrap();
        assert_eq!(
            (b.source, b.groups),
            (BaselineSource::Declared, set(&["ZAGROS-DEV-OBSERVER"]))
        );
        // By the peer's own name.
        let b = resolve_baseline("peer-alice-handset-zagros-dev-2026-09-01-iPhone", S, &p, None).unwrap();
        assert_eq!(b.groups, set(&["ZAGROS-DEV-OBSERVER"]));
        // A declared person with an empty handset list: declared, empty.
        let b = resolve_baseline("peer-alice-smith-zagros-dev-2026-09-01-iPhone", S, &p, Some(&kj));
        assert_eq!(
            b,
            Some(Baseline {
                source: BaselineSource::Declared,
                groups: BTreeSet::new()
            })
        );
    }

    #[test]
    fn key_then_none_when_not_declared() {
        let p = people();
        let kj = join(Some("k"), Some(&["ZAGROS-DEV-OBSERVER"]));
        // Not a person peer name at all.
        let b = resolve_baseline("vendor-registry-01", S, &p, Some(&kj)).unwrap();
        assert_eq!(b.source, BaselineSource::Key);
        // A person peer whose handle is not declared.
        let b = resolve_baseline("peer-bob-zagros-dev-2026-09-01-mac", S, &p, Some(&kj)).unwrap();
        assert_eq!(b.source, BaselineSource::Key);
        // No key groups (deleted key: only a name survived) → none.
        let gone = join(Some("peer-bob-zagros-dev"), None);
        assert!(resolve_baseline("peer-bob-zagros-dev-2026-09-01-mac", S, &p, Some(&gone)).is_none());
        assert!(resolve_baseline("peer-bob-zagros-dev-2026-09-01-mac", S, &p, None).is_none());
    }

    const S: &str = "zagros-dev";

    #[test]
    fn declared_match_is_anchored_on_org_scope_and_date() {
        let only = |h: &str| {
            BTreeMap::from([(
                h.to_string(),
                PersonBaseline {
                    email_hash: email_hash(&format!("{h}@example.test")),
                    device: vec!["ZAGROS-DEV-DEVELOPER".into()],
                    handset: vec![],
                },
            )])
        };
        // An undeclared longer handle does not fall to the shorter one when
        // the remainder does not parse as (-org)?-scope-date.
        let p = only("maintainer");
        assert_eq!(
            declared_handle("peer-maintainer-smith-jones-zagros-dev-2026-09-01-x", S, &p),
            None
        );
        assert_eq!(declared_handle("peer-maintainer-smith-2026-09-01-x", S, &p), None);
        // ...but with a one-token org the grammar says it IS that person's.
        assert_eq!(
            declared_handle("peer-maintainer-smith-zagros-dev-2026-09-01-x", S, &p),
            Some("maintainer")
        );
        // Both declared: the longer handle wins.
        let mut both = only("maintainer");
        both.extend(only("maintainer-smith"));
        assert_eq!(
            declared_handle("peer-maintainer-smith-zagros-dev-2026-09-01-x", S, &both),
            Some("maintainer-smith")
        );
        // No hostname, and a bad date.
        assert_eq!(declared_handle("peer-maintainer-zagros-dev-2026-09-01", S, &p), Some("maintainer"));
        assert_eq!(declared_handle("peer-maintainer-zagros-dev-2026-09-0x-mac", S, &p), None);
        assert_eq!(declared_handle("maintainer-zagros-dev-2026-09-01", S, &p), None);
    }

    #[test]
    fn wrong_scope_in_the_name_falls_back_to_the_key() {
        let p = people();
        let kj = join(Some("k"), Some(&["ZAGROS-DEV-OBSERVER"]));
        // Minted for one scope, converged on another.
        assert_eq!(declared_handle("peer-alice-zagros-2026-09-01-mac", S, &p), None);
        let b = resolve_baseline("peer-alice-zagros-2026-09-01-mac", S, &p, Some(&kj)).unwrap();
        assert_eq!(b.source, BaselineSource::Key);
        assert!(resolve_baseline("peer-alice-zagros-2026-09-01-mac", S, &p, None).is_none());
        // The same name on its own scope is declared.
        let b = resolve_baseline("peer-alice-zagros-2026-09-01-mac", "zagros", &p, None).unwrap();
        assert_eq!(b.source, BaselineSource::Declared);
    }

    #[test]
    fn plan_rows_carry_the_baseline_source() {
        let mut d = decl("report", " []");
        d.people = people();
        let peers = vec![
            peer("a", "peer-alice-zagros-dev-2026-09-01-laptop", &["ZAGROS-DEV-DEVELOPER"]),
            peer("b", "peer-bob-zagros-dev-2026-09-01-mac", &["ZAGROS-DEV-OBSERVER"]),
            peer("c", "peer-carol-zagros-dev-2026-09-01-mac", &["ZAGROS-DEV-OBSERVER"]),
        ];
        let joins = BTreeMap::from([
            ("a".to_string(), join(Some("ka"), Some(&["ZAGROS-DEV-OBSERVER"]))),
            ("b".to_string(), join(Some("kb"), Some(&["ZAGROS-DEV-OBSERVER"]))),
        ]);
        let base = resolve_baselines("zagros-dev", &d.people, &peers, &joins);
        let plan = plan_membership(&d, P, &[], &peers, &base, &all_groups()).unwrap();
        let a = row(&plan, "peer-alice-zagros-dev-2026-09-01-laptop");
        assert_eq!(a.baseline_source, BaselineSource::Declared);
        assert_eq!(a.status, Status::Match);
        assert_eq!(
            row(&plan, "peer-bob-zagros-dev-2026-09-01-mac").baseline_source,
            BaselineSource::Key
        );
        let c = row(&plan, "peer-carol-zagros-dev-2026-09-01-mac");
        assert_eq!((c.baseline_source, c.status), (BaselineSource::None, Status::Unknown));
        assert_eq!(plan.baseline_declared(), 1);
        let v = serde_json::to_value(a).unwrap();
        assert_eq!(v["baseline_source"], "declared");
    }

    #[test]
    fn baselines_join_through_peer_added() {
        let groups = vec![
            NbGroup {
                id: "g1".into(),
                name: "ZAGROS-DEV-DEVELOPER".into(),
                peers: vec![],
            },
            NbGroup {
                id: "g2".into(),
                name: "ZAGROS-DEV-OBSERVER".into(),
                peers: vec![],
            },
        ];
        let keys = vec![
            SetupKeyMeta {
                id: "k1".into(),
                name: "peer-a".into(),
                auto_groups: vec!["g1".into()],
                revoked: false,
            },
            SetupKeyMeta {
                id: "k2".into(),
                name: "peer-b".into(),
                auto_groups: vec!["g2".into()],
                revoked: false,
            },
        ];
        let ev = |act: &str, init: &str, target: &str, meta: serde_json::Value| NbEvent {
            activity: act.into(),
            initiator_id: init.into(),
            target_id: target.into(),
            meta,
            ..Default::default()
        };
        let events = vec![
            ev("Peer added", "k1", "pa", serde_json::json!({})),
            // initiator not a key id; resolved through meta.setup_key_name
            ev(
                "Peer added",
                "user-1",
                "pb",
                serde_json::json!({"setup_key_name": "peer-b"}),
            ),
            // interactive login: no key
            ev("Peer added", "user-2", "pc", serde_json::json!({})),
            ev("Group added to peer", "k1", "pc", serde_json::json!({})),
            // deleted handset key: only its name survives
            ev(
                "Peer added",
                "gone",
                "pd",
                serde_json::json!({"setup_key_name": "peer-d-handset"}),
            ),
        ];
        let b = key_joins(&events, &keys, &groups);
        assert_eq!(b["pa"].groups, Some(set(&["ZAGROS-DEV-DEVELOPER"])));
        assert_eq!(b["pa"].key_name.as_deref(), Some("peer-a"));
        assert_eq!(b["pb"].groups, Some(set(&["ZAGROS-DEV-OBSERVER"])));
        assert!(!b.contains_key("pc"));
        assert_eq!(b["pd"].groups, None);
        assert!(is_handset("peer-d-zagros-dev-2026-09-01-iPhone", b.get("pd")));
    }

    // -- governance ------------------------------------------------------------

    #[test]
    fn report_scope_is_not_governed() {
        let (_t, dir) = scope_dir(&[("_scope.yaml", SCOPE_REPORT)]);
        let repo = repo_converge(&dir);
        assert_eq!(repo, Some(FileConverge::Report));
        assert!(!governed_by(ScopeAnswer::Declared(FileConverge::Report)));
    }

    #[test]
    fn enforce_scope_is_governed_from_scope_yaml() {
        let (_t, dir) = scope_dir(&[
            (
                "_scope.yaml",
                "# comment\nversion: 1\nscope: zagros-dev\nconverge: enforce # the switch\n",
            ),
            // converge in a fragment is not what decides
            ("people.yaml", "version: 1\nentries: []\n"),
        ]);
        assert_eq!(repo_converge(&dir), Some(FileConverge::Enforce));
    }

    #[test]
    fn unreadable_converge_is_governed() {
        let (_t, dir) = scope_dir(&[("_scope.yaml", "converge: [not, a, word]\n")]);
        assert_eq!(repo_converge(&dir), Some(FileConverge::Unreadable));
        assert!(governed_by(ScopeAnswer::Declared(FileConverge::Unreadable)));
        // A scope directory with no _scope.yaml.
        let (_t, dir) = scope_dir(&[("people.yaml", "version: 1\nentries: []\n")]);
        assert_eq!(repo_converge(&dir), Some(FileConverge::Unreadable));
    }

    /// `<tmp>/ops/warrant/membership/` holding the given scopes' `_scope.yaml`.
    fn membership_tree(scopes: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(MEMBERSHIP_DIR);
        std::fs::create_dir_all(&root).unwrap();
        for (scope, converge) in scopes {
            std::fs::create_dir_all(root.join(scope)).unwrap();
            std::fs::write(
                root.join(scope).join(SCOPE_FILE),
                format!("version: 1\nscope: {scope}\nconverge: {converge}\n"),
            )
            .unwrap();
        }
        (t, root)
    }

    fn user(name: &str, service: bool) -> NbUser {
        NbUser {
            name: name.into(),
            is_service_user: service,
            ..NbUser::default()
        }
    }

    #[test]
    fn in_checkout_the_declaration_decides() {
        let (_t, root) = membership_tree(&[("zagros-dev", "enforce"), ("zagros", "report")]);
        let live_says_governed = Some(ScopeAnswer::Declared(FileConverge::Unreadable));
        let enforce = decide(
            checkout_answer(Some(&root), "zagros-dev"),
            context_answer(Some("none"), Some("zagros-dev"), "x", "zagros-dev"),
            None,
        );
        assert_eq!(
            enforce,
            Governance {
                governed: true,
                source: GovernanceSource::Checkout
            }
        );
        // The checkout's report beats a context that says enforce.
        let report = decide(
            checkout_answer(Some(&root), "zagros"),
            context_answer(Some("enforce"), Some("zagros"), "x", "zagros"),
            live_says_governed,
        );
        assert_eq!(
            report,
            Governance {
                governed: false,
                source: GovernanceSource::Checkout
            }
        );
        // A checkout that carries the goatnets' declarations and has no
        // directory for this one: no declaration, live writes as today.
        let undeclared = decide(checkout_answer(Some(&root), "zagros-prod"), None, None);
        assert_eq!(
            undeclared,
            Governance {
                governed: false,
                source: GovernanceSource::Checkout
            }
        );
        // A checkout without ops/warrant/membership/ (the public tree, an
        // unrelated repository) does not answer.
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            checkout_answer(Some(&t.path().join(MEMBERSHIP_DIR)), "zagros-dev"),
            None
        );
        assert_eq!(checkout_answer(None, "zagros-dev"), None);
    }

    #[test]
    fn context_supplied_converge_decides_outside_a_checkout() {
        for (v, governed) in [
            ("enforce", true),
            ("report", false),
            ("none", false),
            ("bogus", true),
        ] {
            let g = decide(
                None,
                context_answer(Some(v), Some("zagros-dev"), "c", "zagros-dev"),
                None,
            );
            assert_eq!(
                g,
                Governance {
                    governed,
                    source: GovernanceSource::Context
                },
                "{v}"
            );
        }
        // Bound by the context's name when it has no `goatnet` field.
        assert_eq!(
            context_answer(Some("report"), None, "zagros", "zagros"),
            Some(ScopeAnswer::Declared(FileConverge::Report))
        );
        // A context bound to another goatnet, or with no value, does not answer.
        assert_eq!(
            context_answer(Some("none"), Some("zagros"), "zagros", "zagros-dev"),
            None
        );
        assert_eq!(context_answer(Some("none"), None, "zagros", "zagros-dev"), None);
        assert_eq!(
            context_answer(None, Some("zagros-dev"), "c", "zagros-dev"),
            None
        );
        assert_eq!(
            context_answer(Some("  "), Some("zagros-dev"), "c", "zagros-dev"),
            None
        );
    }

    #[test]
    fn nothing_known_asks_the_goatnet_and_fails_closed() {
        // A warrant service user on the account: governed, mode unknown.
        let with_warrant = [user("svc-zagros-dev-warrant", true), user("", false)];
        let g = decide(None, None, Some(live_answer::<()>(Ok(&with_warrant))));
        assert_eq!(
            g,
            Governance {
                governed: true,
                source: GovernanceSource::Live
            }
        );
        // No converger on the account (zagros-prod, goatlite, a partner's
        // goatnet today): the tools write live, as they always have.
        let without = [user("svc-zagros-prod-onboard", true), user("", false)];
        let g = decide(None, None, Some(live_answer::<()>(Ok(&without))));
        assert_eq!(
            g,
            Governance {
                governed: false,
                source: GovernanceSource::Live
            }
        );
        assert!(!governed_by(live_answer::<()>(Ok(&[]))));
        // The account could not be read, or was never read: governed.
        assert!(governed_by(live_answer::<&str>(Err("timeout"))));
        assert!(decide(None, None, None).governed);
        // Only `svc-<goatnet>-warrant` counts.
        assert!(is_warrant_service_user(&user("svc-zagros-warrant", true)));
        assert!(!is_warrant_service_user(&user("svc--warrant", true)));
        assert!(!is_warrant_service_user(&user("warrant", true)));
        assert!(!is_warrant_service_user(&user(
            "svc-zagros-warrant-old",
            true
        )));
    }

    #[test]
    fn merge_rules() {
        let ga = |g: &[&str], a: &[&str]| {
            MembershipEntry::with_form(
                "p",
                EntryForm::GrantAbsent {
                    grant: set(g),
                    absent: set(a),
                },
                "o",
                "r",
                None,
            )
        };
        let m = merge_entry(
            Some(&ga(&["X-P2P"], &["X-OBS"])),
            ga(&["X-OBS"], &["X-DEV"]),
        )
        .unwrap();
        assert_eq!(
            m.form().unwrap(),
            EntryForm::GrantAbsent {
                grant: set(&["X-OBS", "X-P2P"]),
                absent: set(&["X-DEV"])
            }
        );
        let ex = MembershipEntry::with_form("p", EntryForm::Exact(set(&["X-DEV"])), "o", "r", None);
        let m = merge_entry(Some(&ex), ga(&["X-OBS"], &["X-DEV"])).unwrap();
        assert_eq!(m.form().unwrap(), EntryForm::Exact(set(&["X-OBS"])));
        let hold =
            MembershipEntry::with_form("p", EntryForm::Hold, "o", "r", Some("2026-09-30".into()));
        let m = merge_entry(Some(&ga(&["X-P2P"], &[])), hold.clone()).unwrap();
        assert_eq!(m.form().unwrap(), EntryForm::Hold);
        assert_eq!(m.expires.as_deref(), Some("2026-09-30"));
    }

    // -- writers ---------------------------------------------------------------

    #[test]
    fn write_entry_creates_the_target_then_merges_and_keeps_comments() {
        let (_t, dir) = scope_dir(&[(
            "_scope.yaml",
            "# header comment\nversion: 1\nscope: zagros-dev\nconverge: report\n",
        )]);
        let e1 = MembershipEntry::with_form(
            "peer-a",
            EntryForm::GrantAbsent {
                grant: set(&["ZAGROS-DEV-P2P"]),
                absent: BTreeSet::new(),
            },
            "alice",
            "dev clique: \"reason 1\"",
            Some("2026-12-01".into()),
        );
        let (_, f1) = write_entry(&dir, "zagros-dev", PEOPLE_FILE, e1).unwrap();
        assert_eq!(f1, dir.join(PEOPLE_FILE));
        let e2 = MembershipEntry::with_form(
            "peer-b",
            EntryForm::Hold,
            "user369x",
            "flood",
            Some("2026-09-30".into()),
        );
        let (_, f2) = write_entry(&dir, "zagros-dev", HOLDS_FILE, e2.clone()).unwrap();
        assert_eq!(f2, dir.join(HOLDS_FILE));
        let e3 = MembershipEntry::with_form(
            "peer-a",
            EntryForm::GrantAbsent {
                grant: BTreeSet::new(),
                absent: set(&["ZAGROS-DEV-OBSERVER"]),
            },
            "alice",
            "second",
            None,
        );
        let (merged, f3) = write_entry(&dir, "zagros-dev", PEOPLE_FILE, e3).unwrap();
        assert_eq!(f3, dir.join(PEOPLE_FILE));
        assert_eq!(
            merged.grant.as_deref(),
            Some(&["ZAGROS-DEV-P2P".to_string()][..])
        );
        assert_eq!(
            merged.absent.as_deref(),
            Some(&["ZAGROS-DEV-OBSERVER".to_string()][..])
        );
        assert_eq!(merged.expires.as_deref(), Some("2026-12-01"));

        let text = std::fs::read_to_string(dir.join(PEOPLE_FILE)).unwrap();
        assert!(text.starts_with("version: 1\n"));
        let d = load_decl(&dir, "zagros-dev").unwrap();
        assert_eq!(d.entries().len(), 2);
        assert_eq!(d.entries().iter().find(|e| e.peer == "peer-a"), Some(&merged));
        assert_eq!(d.entries().iter().find(|e| e.peer == "peer-b"), Some(&e2));
        // _scope.yaml untouched.
        assert!(std::fs::read_to_string(dir.join("_scope.yaml"))
            .unwrap()
            .starts_with("# header comment\n"));
    }

    #[test]
    fn write_entry_edits_in_place_in_another_fragment() {
        let (_t, dir) = scope_dir(&[
            ("_scope.yaml", SCOPE_REPORT),
            (
                "orgs/acme.yaml",
                "# acme's people\nversion: 1\nentries:\n  - peer: peer-a\n    grant: [ZAGROS-DEV-P2P]\n    owner: o\n    reason: r\n  # keep me\n  - peer: peer-z\n    hold: quarantine\n    owner: o\n    reason: r\n",
            ),
        ]);
        let e = MembershipEntry::with_form(
            "peer-a",
            EntryForm::GrantAbsent {
                grant: set(&["ZAGROS-DEV-PLATFORM-ADMIN"]),
                absent: BTreeSet::new(),
            },
            "user369x",
            "platform-admin grant (ADR 1182)",
            None,
        );
        let (merged, file) = write_entry(&dir, "zagros-dev", PEOPLE_FILE, e).unwrap();
        assert_eq!(file, dir.join("orgs/acme.yaml"));
        assert!(!dir.join(PEOPLE_FILE).exists(), "no duplicate created");
        assert_eq!(
            merged.grant.as_deref().unwrap(),
            ["ZAGROS-DEV-P2P", "ZAGROS-DEV-PLATFORM-ADMIN"]
        );
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# acme's people\n") && text.contains("  # keep me\n"));
        let d = load_decl(&dir, "zagros-dev").unwrap();
        assert_eq!(d.entries().len(), 2);
    }

    #[test]
    fn writer_targets_by_kind() {
        let p = "ZAGROS-DEV";
        assert_eq!(target_fragment(&EntryForm::Hold, p), HOLDS_FILE);
        assert_eq!(
            target_fragment(&EntryForm::Exact(set(&["ZAGROS-DEV-EXTERNAL-INFRA"])), p),
            SERVICES_FILE
        );
        assert_eq!(
            target_fragment(&EntryForm::Exact(set(&["ZAGROS-DEV-PROBATION-DEVELOPER"])), p),
            HOLDS_FILE
        );
        assert_eq!(
            target_fragment(&EntryForm::Exact(set(&["ZAGROS-DEV-OBSERVER"])), p),
            PEOPLE_FILE
        );
        assert_eq!(
            target_fragment(
                &EntryForm::GrantAbsent {
                    grant: set(&["ZAGROS-DEV-P2P"]),
                    absent: BTreeSet::new()
                },
                p
            ),
            PEOPLE_FILE
        );
        assert!(check_fragment_name(Path::new("../x.yaml")).is_err());
        assert!(check_fragment_name(Path::new("_scope.yaml")).is_err());
        assert!(check_fragment_name(Path::new("orgs/acme.yaml")).is_ok());
    }

    #[test]
    fn metrics_carry_the_scope_label() {
        let r = Report {
            scope: "zagros-dev".into(),
            mode: "enforce",
            peers_total: 3,
            r#match: 1,
            drift: 2,
            blocked: 0,
            privileged_quarantine_blocked: 1,
            unknown: 0,
            missing_peer: 0,
            baseline_declared: 2,
            changes_planned: 2,
            changes_applied: 2,
            drift_after: Some(0),
            paused: None,
            pause_expired: None,
            pause_warning: None,
            rows: vec![],
        };
        let body = metrics_body(&r, 42);
        assert!(body.contains("goat_warrant_membership_peers_total{scope=\"zagros-dev\"} 3\n"));
        assert!(body.contains("goat_warrant_membership_drift_total{scope=\"zagros-dev\"} 0\n"));
        assert!(body.contains("goat_warrant_membership_enforce{scope=\"zagros-dev\"} 1\n"));
        assert!(body.contains(
            "goat_warrant_membership_last_run_timestamp_seconds{scope=\"zagros-dev\"} 42\n"
        ));
        assert!(body.contains("goat_warrant_membership_unknown_baseline_total"));
        assert!(body.contains(
            "goat_warrant_membership_privileged_quarantine_blocked{scope=\"zagros-dev\"} 1\n"
        ));
        assert!(body.contains(
            "goat_warrant_membership_baseline_declared_total{scope=\"zagros-dev\"} 2\n"
        ));
        assert!(
            body.contains("goat_warrant_membership_changes_applied_total{scope=\"zagros-dev\"} 2\n")
        );
        assert!(body.contains("goat_warrant_membership_match_total"));
        assert!(body.contains("goat_warrant_membership_paused{scope=\"zagros-dev\"} 0\n"));
    }

    // -- pause ------------------------------------------------------------

    fn t(s: &str) -> chrono::DateTime<chrono::Utc> {
        parse_utc(s).unwrap()
    }

    fn pause_json(scope: &str, until: &str) -> String {
        serde_json::to_string(&PauseRecord {
            scope: scope.into(),
            until: until.into(),
            reason: "emergency quarantine of peer-x".into(),
            actor: "maintainer".into(),
            created: "2026-09-23T10:00:00Z".into(),
        })
        .unwrap()
    }

    #[test]
    fn active_pause_runs_as_report() {
        let now = t("2026-09-23T10:30:00Z");
        let st = pause_state(
            Some(&pause_json("zagros-dev", "2026-09-23T11:00:00Z")),
            "zagros-dev",
            now,
        );
        assert!(matches!(st, PauseState::Active(ref p) if p.actor == "maintainer"));
        assert_eq!(mode_under_pause(Mode::Enforce, &st), Mode::Report);
        assert_eq!(mode_under_pause(Mode::Report, &st), Mode::Report);
    }

    #[test]
    fn expired_pause_is_ignored_and_reported() {
        let now = t("2026-09-23T11:00:01Z");
        let st = pause_state(
            Some(&pause_json("zagros-dev", "2026-09-23T11:00:00Z")),
            "zagros-dev",
            now,
        );
        assert!(matches!(st, PauseState::Expired(_)));
        assert_eq!(mode_under_pause(Mode::Enforce, &st), Mode::Enforce);
        // No file is not a pause either.
        assert_eq!(pause_state(None, "zagros-dev", now), PauseState::None);
        assert_eq!(
            mode_under_pause(Mode::Enforce, &PauseState::None),
            Mode::Enforce
        );
    }

    #[test]
    fn malformed_pause_converges_with_a_warning() {
        let now = t("2026-09-23T10:30:00Z");
        for (text, scope) in [
            ("{not json".to_string(), "zagros-dev"),
            ("{\"scope\":\"zagros-dev\"}".to_string(), "zagros-dev"),
            (pause_json("zagros-dev", "tomorrow"), "zagros-dev"),
            // another scope's file
            (pause_json("zagros", "2026-09-23T11:00:00Z"), "zagros-dev"),
            // further out than any pause may run (hand-edited)
            (pause_json("zagros-dev", "2030-01-01T00:00:00Z"), "zagros-dev"),
        ] {
            let st = pause_state(Some(&text), scope, now);
            assert!(matches!(st, PauseState::Malformed(_)), "{text}: {st:?}");
            assert_eq!(mode_under_pause(Mode::Enforce, &st), Mode::Enforce);
        }
    }

    #[test]
    fn malformed_pause_file_on_disk_is_a_warning_in_the_report() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(pause_path(dir.path(), "zagros-dev"), "garbage").unwrap();
        let st = read_pause(dir.path(), "zagros-dev", t("2026-09-23T10:30:00Z"));
        assert!(matches!(st, PauseState::Malformed(_)));
        // A missing file is no pause.
        let st = read_pause(dir.path(), "zagros", t("2026-09-23T10:30:00Z"));
        assert_eq!(st, PauseState::None);
    }

    #[test]
    fn pause_over_24h_is_refused() {
        let now = t("2026-09-23T10:00:00Z");
        for d in ["25h", "24h1s", "1441m", "999999999999h"] {
            let e = build_pause("zagros-dev", d, "why", "maintainer", now).unwrap_err();
            assert!(format!("{e}").contains("24h"), "{d}: {e}");
        }
        let p = build_pause("zagros-dev", "24h", "why", "maintainer", now).unwrap();
        assert_eq!(p.until, "2026-09-24T10:00:00Z");
        let p = build_pause("zagros-dev", "1h30m", "why", "maintainer", now).unwrap();
        assert_eq!(p.until, "2026-09-23T11:30:00Z");
        assert_eq!(p.created, "2026-09-23T10:00:00Z");
        for d in ["", "0m", "1d", "h", "30", "-1h"] {
            assert!(
                build_pause("zagros-dev", d, "why", "maintainer", now).is_err(),
                "{d}"
            );
        }
    }

    #[test]
    fn pause_with_empty_reason_is_refused() {
        let now = t("2026-09-23T10:00:00Z");
        for r in ["", "   "] {
            let e = build_pause("zagros-dev", "1h", r, "maintainer", now).unwrap_err();
            assert!(format!("{e}").contains("--reason"), "{e}");
        }
    }

    #[test]
    fn pause_refuses_a_scope_that_is_not_a_plain_id() {
        let now = t("2026-09-23T10:00:00Z");
        for s in ["../etc", "ZAGROS", "a/b", ""] {
            assert!(build_pause(s, "1h", "why", "maintainer", now).is_err(), "{s}");
        }
    }

    #[test]
    fn a_written_pause_reads_back_active() {
        let now = t("2026-09-23T10:00:00Z");
        let p = build_pause("zagros-dev", "30m", "edit", "maintainer", now).unwrap();
        let text = serde_json::to_string_pretty(&p).unwrap();
        assert_eq!(
            pause_state(Some(&text), "zagros-dev", t("2026-09-23T10:29:59Z")),
            PauseState::Active(p.clone())
        );
        assert_eq!(
            pause_state(Some(&text), "zagros-dev", t("2026-09-23T10:30:00Z")),
            PauseState::Expired(p)
        );
    }
}
