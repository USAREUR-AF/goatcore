//! ACL assertions — the gate.
//!
//! netvis makes the gap between the declared ACL and the live mesh visible.
//! This module is the other half: it keeps the gap from reopening, by letting
//! the declaration carry **assertions about itself** that are checked before a
//! change lands and again, continuously, against live state.
//!
//! # Why this shape
//!
//! Every decision here was bought with somebody else's production experience.
//!
//! **Two separately-named checks, never one with a direction flag.** Batfish
//! shipped the general version, it produced semantic ambiguity, and they split
//! it. Forward Networks names the pair **existence** and **isolation**. Those
//! are the two words used here. `accept:` asserts existence — *this path MUST
//! be reachable* — which is the capability we did not have: until now we could
//! only observe what does reach, never assert what should. AWS Network Access
//! Analyzer still cannot: its scope language has `MatchPaths` and
//! `ExcludePaths` and no `MustExistPaths`, so a missing required path yields
//! zero findings, byte-identical to compliant.
//!
//! **The quantifier is ALL-TO-ALL and it is written down.** "Set A should
//! reach set B" has four readings and Batfish reports the ambiguity causing
//! real misuse. Every source must reach every destination; anything less is a
//! violation with a named pair.
//!
//! **Search for the VIOLATION; EMPTY is the pass.** That inversion turns a
//! spot check into a proof over the whole space and yields a counterexample
//! instead of a bare `false`.
//!
//! **Show a positive example beside the counterexample.** Batfish added this
//! deliberately: contrasting the two localises the fault. Beside "nothing
//! reaches the breakglass" we show the nearest rule that almost does.
//!
//! **Three verdicts, and `Undetermined` CAPS the roll-up.** Exposing a third
//! state is not enough — Batfish's own open bug is parse failures causing
//! later false positives. An assertion that could not be decided must not be
//! summarised as a pass.
//!
//! **A closed reason enum that separates "the mesh is broken" from "your
//! declaration was malformed."** AWS has a whole class of codes for the
//! latter. A malformed row must never be graded as a broken mesh.
//!
//! **No solver.** BDDs and SMT exist because a data plane is a fixed point
//! over routing protocols. A NetBird policy is group-to-group with ports:
//! set intersection over bounded membership. The lineage is cited; none of it
//! is imported.
//!
//! # One evaluator, two sources
//!
//! The evaluator reads a [`Model`] — a flat set of [`Permit`]s over group
//! names. Two things build one:
//!
//! * [`Declaration::model`] — from `netbird_acl_matrix.yml` +
//!   `netbird_acl_taxonomy.yml`, with `netbird_exercises` expanded exactly as
//!   the playbooks expand it. Needs no network, so it is the PR gate.
//! * [`Model::from_doc`] — from a live `netvis::Doc`. Same evaluator, so a
//!   verdict cannot mean two different things depending on where it ran.
//!
//! Two implementations of "does this rule permit that" is how two surfaces
//! come to disagree about the same goatnet. There is one.
//!
//! # The YAML is passed IN, never compiled in here
//!
//! [`Declaration::parse`] takes the two files as strings. An `include_str!`
//! at this level would embed repo-relative paths in a LIBRARY, and every
//! Dockerfile that links it — microscope, stethoscope, panoscope, flashlight,
//! none of which know what an ACL assertion is — would have to start shipping
//! `ops/ansible/*.yml` to compile. It also means a taxonomy edit would rebuild
//! five images that do not read it. The two consumers that DO care
//! (`netvis`, `goat-cli`) own the `include_str!` themselves, exactly as
//! `netvis/src/doctrine.rs` already does.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::meshgraph::netvis;

/// A scope-qualified reference resolves against the scope named before the
/// colon. `HOST:` is the alias for an exercise's host site (ADR 1124).
const HOST_ALIAS: &str = "HOST";
/// NetBird's account-wide All group. When a permit names this as source or
/// destination it subsumes every named pair for the protocols/ports it grants.
const ALL_GROUP_NAME: &str = "All";

// ===========================================================================
// The model
// ===========================================================================

/// One group-to-group permit, flattened out of whatever produced it.
///
/// `ports` empty means EVERY port — that is NetBird's own reading and
/// inverting it would silently narrow a wide rule, which is the direction a
/// security surface must never be wrong in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permit {
    pub src: String,
    pub dst: String,
    /// `all` | `tcp` | `udp` | `icmp`. Empty is read as `all`.
    pub protocol: String,
    pub ports: Vec<String>,
    /// The rule this came from, so a verdict can name what granted the path.
    /// A finding an operator cannot trace to a rule is a finding they cannot
    /// act on.
    pub rule: String,
}

impl Permit {
    /// Public because `goat warrant why-blocked` (ADR 1129) answers "does a
    /// rule already permit this" and must answer it the SAME way the
    /// assertions do. A second copy of this reading is a second opinion about
    /// what a goatnet permits, and the two would drift.
    pub fn covers_proto(&self, want: &str) -> bool {
        let have = if self.protocol.is_empty() { "all" } else { self.protocol.as_str() };
        want == ANY_PROTO || have.eq_ignore_ascii_case("all") || have.eq_ignore_ascii_case(want)
    }
    pub fn covers_port(&self, want: &str) -> bool {
        want == ANY_PORT || self.ports.is_empty() || self.ports.iter().any(|p| p == want)
    }
}

/// `proto: any` — the assertion does not care which protocol carries it.
pub const ANY_PROTO: &str = "any";
/// `GROUP:*` — the assertion is about the pair, not about a port.
pub const ANY_PORT: &str = "*";

/// Where a model came from. Carried into the report because "declared" and
/// "live" answer different questions and a reader must never have to guess
/// which one they are holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The YAML in git. What we said should be true.
    Declared,
    /// The live NetBird plane. What is true right now.
    Live,
}

/// Everything an assertion is evaluated against.
#[derive(Debug, Clone)]
pub struct Model {
    pub site: String,
    /// This scope's own prefix — what a bare suffix in an assertion resolves
    /// against.
    pub prefix: String,
    /// For an exercise scope, the prefix of the site hosting it. `HOST:` in a
    /// rule OR in an assertion target resolves against this (ADR 1124). None
    /// for a persistent site, where `HOST:` is meaningless and must fail
    /// rather than quietly resolving to the scope itself.
    pub host_prefix: Option<String>,
    /// Every group this scope can NAME — its own, plus its host scope's,
    /// because a cross-scope reference (ADR 1124) is a legitimate thing for a
    /// rule or an assertion to point at and they live on the same mgmt
    /// server. A test naming a group that is in neither is a malformed
    /// declaration, not a broken mesh, and the two must not grade the same.
    pub groups: BTreeSet<String>,
    /// Scope token (lowercased) → group prefix, for `SCOPE:SUFFIX:port`
    /// targets. Carried ON the model so the evaluator is a pure function of
    /// its inputs — it used to re-read and re-parse the whole declaration
    /// mid-evaluation, which is both a hidden dependency and a per-target
    /// YAML parse.
    pub scope_prefixes: BTreeMap<String, String>,
    pub permits: Vec<Permit>,
    pub source: Source,
    /// False when the source could not be read. Anything that would have to
    /// consult it returns `Undetermined`.
    pub groups_read: bool,
    pub policies_read: bool,
}

impl Model {
    /// Build from a live `netvis::Doc` — the same document the matrix draws
    /// from, so the assertions and the picture cannot disagree.
    pub fn from_doc(doc: &netvis::Doc, prefix: &str, host_prefix: Option<&str>) -> Self {
        let mut permits = Vec::new();
        for e in &doc.edges {
            let (s, d) = (&doc.nodes[e.s].name, &doc.nodes[e.d].name);
            for ri in &e.rules {
                let Some(r) = doc.rules.get(*ri) else { continue };
                if !r.enabled {
                    continue;
                }
                permits.push(Permit {
                    src: s.clone(),
                    dst: d.clone(),
                    protocol: if r.protocol.is_empty() { "all".into() } else { r.protocol.clone() },
                    ports: r.ports.clone(),
                    rule: if r.rule_name.is_empty() { r.policy.clone() } else { r.rule_name.clone() },
                });
            }
        }
        Self {
            site: doc.site.clone(),
            prefix: prefix.to_string(),
            host_prefix: host_prefix.map(str::to_string),
            groups: doc.nodes.iter().map(|n| n.name.clone()).collect(),
            scope_prefixes: BTreeMap::new(),
            permits,
            source: Source::Live,
            groups_read: doc.groups_read,
            policies_read: doc.policies_read,
        }
    }

    fn resolve(&self, prefix: &str, suffix: &str) -> String {
        if suffix == netvis::SELF_SUFFIX { prefix.to_string() } else { format!("{prefix}-{suffix}") }
    }

    /// The permit, if any, that grants this pair on the requested proto/port. A
    /// permit naming the account-wide `All` group on either side subsumes every
    /// named pair for the protocols/ports it grants, so `All → All` covers the
    /// whole space.
    fn permit_for(&self, src: &str, dst: &str, proto: &str, port: &str) -> Option<&Permit> {
        self.permits.iter().find(|p| {
            p.covers_proto(proto)
                && p.covers_port(port)
                && (p.src == src || p.src == ALL_GROUP_NAME)
                && (p.dst == dst || p.dst == ALL_GROUP_NAME)
        })
    }

    /// Whether an unrestricted All→All permit is present — the flat-mesh
    /// posture the assertions are there to keep from resurrecting.
    fn unrestricted_all_to_all(&self) -> Option<&Permit> {
        self.permits.iter().find(|p| {
            p.src == ALL_GROUP_NAME
                && p.dst == ALL_GROUP_NAME
                && (p.protocol.is_empty() || p.protocol.eq_ignore_ascii_case("all"))
                && p.ports.is_empty()
        })
    }
}

// ===========================================================================
// The declaration
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclError {
    Unparseable(String),
    NoSuchSite(String),
}

impl std::fmt::Display for DeclError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unparseable(w) => write!(f, "the declared ACL does not parse: {w}"),
            Self::NoSuchSite(s) => write!(f, "no site or exercise scope named {s} is declared"),
        }
    }
}
impl std::error::Error for DeclError {}

#[derive(Debug, Deserialize)]
struct TaxonomyFile {
    netbird_acl_taxonomy: BTreeMap<String, TaxSite>,
    #[serde(default)]
    netbird_exercises: BTreeMap<String, ExerciseRow>,
    #[serde(default)]
    netbird_exercise_template: Option<GroupTemplate>,
    #[serde(default)]
    netbird_exercise_gear_template: Option<GroupTemplate>,
}
#[derive(Debug, Deserialize)]
struct TaxSite {
    site_prefix: String,
    #[serde(default)]
    groups: Vec<TaxGroup>,
}
#[derive(Debug, Deserialize)]
struct TaxGroup {
    suffix: String,
}
#[derive(Debug, Deserialize)]
struct GroupTemplate {
    #[serde(default)]
    groups: Vec<TaxGroup>,
}
#[derive(Debug, Deserialize)]
struct ExerciseRow {
    site_prefix: String,
    host_site: String,
    #[serde(default)]
    gear: bool,
}

#[derive(Debug, Deserialize)]
struct MatrixFile {
    netbird_acl_matrix: BTreeMap<String, MatrixSite>,
    #[serde(default)]
    netbird_exercise_rules_template: Option<RuleTemplate>,
    #[serde(default)]
    netbird_exercise_gear_rules_template: Option<RuleTemplate>,
    /// The assertions. A separate top-level key because they are a different
    /// KIND of statement from the rules: the rules say what is permitted, the
    /// tests say what must and must not follow from that. Folding them into
    /// the rule rows would make an assertion look like a grant.
    #[serde(default)]
    netbird_acl_tests: BTreeMap<String, Vec<TestRow>>,
}
#[derive(Debug, Deserialize)]
struct MatrixSite {
    #[serde(default)]
    rules: Vec<MatrixRule>,
}
#[derive(Debug, Deserialize)]
struct RuleTemplate {
    #[serde(default)]
    rules: Vec<MatrixRule>,
}
#[derive(Debug, Clone, Deserialize)]
struct MatrixRule {
    #[serde(default)]
    name: String,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    destinations: Vec<String>,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    ports: Vec<String>,
    #[serde(default)]
    action: String,
    #[serde(default)]
    bidirectional: bool,
}

/// The ACL declaration, parsed. Build one from the two YAML files and ask it
/// for a scope's model or a scope's assertions.
pub struct Declaration {
    tax: TaxonomyFile,
    mat: MatrixFile,
}

/// One scope, resolved. A named struct rather than a four-tuple because
/// `Option<String>` appearing twice in a positional return is exactly the
/// shape where the host prefix and the host NAME get swapped at a call site
/// and nothing catches it.
struct Scope {
    prefix: String,
    suffixes: Vec<String>,
    rules: Vec<MatrixRule>,
    /// The site hosting this one, when it is an exercise scope.
    host_site: Option<String>,
}

impl Declaration {
    /// `taxonomy` is `ops/ansible/netbird_acl_taxonomy.yml`, `matrix` is
    /// `ops/ansible/netbird_acl_matrix.yml`. Passed in, never read from disk
    /// or compiled in — see the module note.
    pub fn parse(taxonomy: &str, matrix: &str) -> Result<Self, DeclError> {
        Ok(Self {
            tax: serde_yaml::from_str(taxonomy)
                .map_err(|e| DeclError::Unparseable(format!("taxonomy: {e}")))?,
            mat: serde_yaml::from_str(matrix)
                .map_err(|e| DeclError::Unparseable(format!("matrix: {e}")))?,
        })
    }

    /// Every scope declared — sites and exercise scopes alike. The gate
    /// iterates this so a new scope is covered the day it is declared, rather
    /// than the day someone remembers to add it to a list.
    pub fn scopes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tax.netbird_acl_taxonomy.keys().cloned().collect();
        v.extend(self.tax.netbird_exercises.keys().cloned());
        v.extend(
            self.mat
                .netbird_acl_matrix
                .keys()
                .chain(self.mat.netbird_acl_tests.keys())
                .filter(|s| self.default_scope(s).is_some())
                .cloned(),
        );
        v.sort();
        v.dedup();
        v
    }

    /// The assertions declared for one scope. Empty is a legitimate answer
    /// and is reported as such — a scope with no assertions is not a scope
    /// that passes.
    pub fn tests(&self, site: &str) -> Vec<TestRow> {
        self.mat.netbird_acl_tests.get(site).cloned().unwrap_or_default()
    }

    /// Prefix + group suffixes + rules for a scope, with `netbird_exercises`
    /// expanded exactly as `ops/ansible/tasks/netbird-exercise-expand.yml`
    /// expands it: the fleet-standard template groups and rules, plus the
    /// ADR 1116 gear set when the row declares `gear: true`. One row in that
    /// map IS the whole declaration, and a checker that did not expand it
    /// would report every exercise scope as undeclared.
    fn scope(&self, site: &str) -> Option<Scope> {
        if let Some(t) = self.tax.netbird_acl_taxonomy.get(site) {
            let rules = self
                .mat
                .netbird_acl_matrix
                .get(site)
                .map(|m| m.rules.clone())
                .unwrap_or_default();
            return Some(Scope {
                prefix: t.site_prefix.clone(),
                suffixes: t.groups.iter().map(|g| g.suffix.clone()).collect(),
                rules,
                host_site: None,
            });
        }
        let ex = self.tax.netbird_exercises.get(site)?;
        let mut suffixes: Vec<String> = self
            .tax
            .netbird_exercise_template
            .as_ref()
            .map(|t| t.groups.iter().map(|g| g.suffix.clone()).collect())
            .unwrap_or_default();
        let mut rules: Vec<MatrixRule> = self
            .mat
            .netbird_exercise_rules_template
            .as_ref()
            .map(|t| t.rules.clone())
            .unwrap_or_default();
        if ex.gear {
            if let Some(t) = self.tax.netbird_exercise_gear_template.as_ref() {
                suffixes.extend(t.groups.iter().map(|g| g.suffix.clone()));
            }
            if let Some(t) = self.mat.netbird_exercise_gear_rules_template.as_ref() {
                rules.extend(t.rules.iter().cloned());
            }
        }
        Some(Scope {
            prefix: ex.site_prefix.clone(),
            suffixes,
            rules,
            host_site: Some(ex.host_site.clone()),
        })
    }

    /// A goatnet the matrix declares but the taxonomy has no stanza for
    /// inherits `_default`, with `site_prefix` = the site id upper-cased —
    /// exactly as `ops/ansible/tasks/netbird-taxonomy-default.yml` derives it
    /// for the ACL plays. Only for a site the matrix names (rules or
    /// assertions): a name declared nowhere stays no scope, so a typo is
    /// still refused rather than checked against an empty model.
    fn default_scope(&self, site: &str) -> Option<Scope> {
        if self.tax.netbird_acl_taxonomy.contains_key(site)
            || self.tax.netbird_exercises.contains_key(site)
        {
            return None;
        }
        let matrix = self.mat.netbird_acl_matrix.get(site);
        if matrix.is_none() && !self.mat.netbird_acl_tests.contains_key(site) {
            return None;
        }
        let d = self.tax.netbird_acl_taxonomy.get("_default")?;
        Some(Scope {
            prefix: site.to_uppercase(),
            suffixes: d.groups.iter().map(|g| g.suffix.clone()).collect(),
            rules: matrix.map(|m| m.rules.clone()).unwrap_or_default(),
            host_site: None,
        })
    }

    pub fn model(&self, site: &str) -> Result<Model, DeclError> {
        let Scope { prefix, suffixes, rules, host_site } = self
            .scope(site)
            .or_else(|| self.default_scope(site))
            .ok_or_else(|| DeclError::NoSuchSite(site.to_string()))?;
        let host_prefix = host_site
            .as_deref()
            .and_then(|h| self.scope(h).map(|s| s.prefix))
            .unwrap_or_else(|| prefix.clone());

        let mut groups: BTreeSet<String> =
            suffixes.iter().map(|s| format!("{prefix}-{s}")).collect();
        // The host scope's groups are nameable from here (`HOST:INFRA`) and
        // are real groups on the same mgmt server. Without them every
        // cross-scope assertion grades UNDETERMINED — which caps the roll-up,
        // so the ADR 1124 lane would silently take the whole gate down with
        // it rather than being checked.
        if let Some(h) = host_site.as_deref() {
            if let Some(hs) = self.scope(h) {
                groups.extend(hs.suffixes.iter().map(|s| format!("{}-{s}", hs.prefix)));
            }
        }

        // `SCOPE:SUFFIX` resolves against the named scope (ADR 1124); a bare
        // suffix against this one. `HOST:` is the exercise's host site.
        let resolve = |r: &str| -> Option<String> {
            match r.split_once(':') {
                None => Some(format!("{prefix}-{r}")),
                Some((scope, suffix)) if scope == HOST_ALIAS => {
                    Some(format!("{host_prefix}-{suffix}"))
                }
                Some((scope, suffix)) => {
                    let p = self.scope(&scope.to_lowercase()).map(|s| s.prefix)?;
                    Some(format!("{p}-{suffix}"))
                }
            }
        };

        let mut permits = Vec::new();
        for r in &rules {
            // `action: drop` is read, and is not a permit. Folding a drop
            // into the permit set would be the worst possible direction to
            // be wrong in.
            if !r.action.is_empty() && !r.action.eq_ignore_ascii_case("accept") {
                continue;
            }
            for s in &r.sources {
                for d in &r.destinations {
                    let (Some(s), Some(d)) = (resolve(s), resolve(d)) else { continue };
                    let mk = |src: String, dst: String| Permit {
                        src,
                        dst,
                        protocol: if r.protocol.is_empty() { "all".into() } else { r.protocol.clone() },
                        ports: r.ports.clone(),
                        rule: r.name.clone(),
                    };
                    permits.push(mk(s.clone(), d.clone()));
                    // A bidirectional rule is ONE rule granting TWO
                    // directions. Flattening it here is what lets the
                    // evaluator stay direction-blind.
                    if r.bidirectional {
                        permits.push(mk(d, s));
                    }
                }
            }
        }

        Ok(Model {
            site: site.to_string(),
            prefix,
            host_prefix: host_site.as_deref().map(|_| host_prefix.clone()),
            scope_prefixes: self
                .scopes()
                .iter()
                .filter_map(|s| self.scope(s).map(|sc| (s.to_lowercase(), sc.prefix)))
                .collect(),
            groups,
            permits,
            source: Source::Declared,
            groups_read: true,
            policies_read: true,
        })
    }
}

// ===========================================================================
// The assertions
// ===========================================================================

/// One assertion, in Tailscale's field names so that anyone who has used
/// theirs reads ours cold: `src`, `proto`, `accept`, `deny`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestRow {
    /// What this assertion is for, in a sentence. Required: an assertion
    /// nobody can read is an assertion nobody will fix.
    pub name: String,
    /// Source group suffix(es). All of them, to all destinations.
    #[serde(deserialize_with = "one_or_many")]
    pub src: Vec<String>,
    /// `tcp` | `udp` | `icmp` | `any`.
    #[serde(default = "any_proto")]
    pub proto: String,
    /// EXISTENCE — every one of these MUST be reachable from every `src`.
    #[serde(default)]
    pub accept: Vec<String>,
    /// ISOLATION — none of these may be reachable from any `src`.
    #[serde(default)]
    pub deny: Vec<String>,
    /// A violation that is already true and already decided.
    ///
    /// Batfish §5.2: a day-one audit of an aged network "becomes a
    /// low-priority, background project", while "we see a greater willingness
    /// to go fix new errors immediately". So the gate fails on the DELTA. A
    /// baselined row still runs, is still reported, and is reported again —
    /// loudly — when it starts passing, because that is when the baseline
    /// should be deleted and the ratchet tightened.
    #[serde(default)]
    pub baseline: Option<Baseline>,
}

fn any_proto() -> String {
    ANY_PROTO.to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Baseline {
    pub violated_since: String,
    pub why: String,
}

fn one_or_many<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

// ===========================================================================
// The verdict
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Satisfied,
    /// Known, decided, and not counted against the gate — but still shown.
    Baselined,
    Violated,
    /// Could not be decided. ORDERED LAST on purpose: the roll-up takes the
    /// maximum, so one undecidable assertion caps the whole report. Exposing
    /// the third state without letting it govern is the bug Batfish shipped.
    Undetermined,
}

/// Why an assertion could not be decided. CLOSED, and split by WHOSE fault it
/// is: a malformed row must never be graded as a broken mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The declaration names a group this scope does not have.
    UnknownGroup,
    /// The target is not `GROUP:port` (or `SCOPE:GROUP:port`).
    MalformedTarget,
    /// `proto` is not one of tcp / udp / icmp / any.
    UnknownProtocol,
    /// The row selects nothing — no source, or neither accept nor deny.
    EmptySelector,
    /// The group list could not be read. Not a denial.
    GroupsUnread,
    /// The policy set could not be read. An absent path is UNKNOWN.
    PoliciesUnread,
}

impl Reason {
    /// Whether the fault is in what we wrote or in what we could read. The
    /// remediation is completely different and the report says which.
    pub fn is_declaration_fault(self) -> bool {
        matches!(
            self,
            Self::UnknownGroup | Self::MalformedTarget | Self::UnknownProtocol | Self::EmptySelector
        )
    }
    pub fn explain(self) -> &'static str {
        match self {
            Self::UnknownGroup => "the assertion names a group this scope does not declare — fix the assertion or declare the group",
            Self::MalformedTarget => "a target is not `GROUP:port` — `*` is a legal port meaning any",
            Self::UnknownProtocol => "proto must be tcp, udp, icmp or any",
            Self::EmptySelector => "the assertion selects nothing: it needs a src and at least one accept or deny",
            Self::GroupsUnread => "the group list could not be read, so membership is unknown — not empty",
            Self::PoliciesUnread => "the policy set could not be read, so an absent path is UNKNOWN, not a denial",
        }
    }
}

/// A named pair that breaks the assertion. Never a bare `false`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counterexample {
    pub src: String,
    pub dst: String,
    pub port: String,
    pub proto: String,
    /// For an isolation failure: the rule that grants what should not be
    /// granted. `None` for existence — nothing granted it, that is the point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted_by: Option<String>,
    /// The nearest thing that ALMOST holds. Batfish added contrast
    /// deliberately: it is what localises the fault. Beside "nothing reaches
    /// the breakglass", the rule that nearly does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub name: String,
    /// `existence` or `isolation` — Forward Networks' words, and two
    /// separately-named checks rather than one with a direction flag.
    pub check: String,
    pub verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    /// Every pair that breaks it, not the first. A report that stops at one
    /// makes the second look like it was not there.
    pub counterexamples: Vec<Counterexample>,
    /// Pairs that hold. Present so a pass is a computed result rather than an
    /// empty render — the same discipline the findings view is held to.
    pub checked_pairs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Baseline>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub site: String,
    pub source: Source,
    /// The roll-up: the WORST verdict present. `Undetermined` outranks
    /// `Violated` because an unknown is not a pass and must not be reported
    /// as one.
    pub verdict: Verdict,
    pub outcomes: Vec<Outcome>,
    pub satisfied: u32,
    pub violated: u32,
    pub baselined: u32,
    pub undetermined: u32,
    /// A baselined assertion that now passes. The ratchet tightens here, and
    /// it only tightens if someone is told.
    pub baseline_now_passing: Vec<String>,
    /// The quantifier, stated in the artifact rather than in a doc somebody
    /// has to find. "Set A should reach set B" has four readings.
    pub quantifier: String,
}

impl Report {
    /// The gate's answer. `Violated` fails; `Undetermined` fails, because an
    /// assertion nobody could decide is not an assertion anybody passed.
    pub fn is_clean(&self) -> bool {
        matches!(self.verdict, Verdict::Satisfied | Verdict::Baselined)
    }
}

// ===========================================================================
// The evaluator
// ===========================================================================

/// `SUFFIX:PORT` or `SCOPE:SUFFIX:PORT`. `*` is a legal port meaning any.
fn parse_target(t: &str) -> Option<(Option<String>, String, String)> {
    let parts: Vec<&str> = t.split(':').collect();
    match parts.as_slice() {
        [g, p] if !g.is_empty() && !p.is_empty() => Some((None, (*g).into(), (*p).into())),
        [s, g, p] if !s.is_empty() && !g.is_empty() && !p.is_empty() => {
            Some((Some((*s).into()), (*g).into(), (*p).into()))
        }
        _ => None,
    }
}

pub fn evaluate(model: &Model, tests: &[TestRow]) -> Report {
    let mut outcomes = Vec::new();
    for t in tests {
        if !t.accept.is_empty() {
            outcomes.push(one(model, t, "existence", &t.accept));
        }
        if !t.deny.is_empty() {
            outcomes.push(one(model, t, "isolation", &t.deny));
        }
        if t.accept.is_empty() && t.deny.is_empty() {
            outcomes.push(undecided(t, "existence", Reason::EmptySelector));
        }
    }
    // Built-in flat-mesh check: a live All→All proto-all permit means the mesh
    // is effectively flat again. Evaluated by the same engine and surfaced as
    // one outcome beside the declared assertions.
    if !tests.is_empty() {
        outcomes.push(flat_mesh_outcome(model));
    }

    let (mut sat, mut vio, mut base, mut und) = (0, 0, 0, 0);
    let mut worst = Verdict::Satisfied;
    let mut now_passing = Vec::new();
    for o in &outcomes {
        match o.verdict {
            Verdict::Satisfied => {
                sat += 1;
                if o.baseline.is_some() {
                    now_passing.push(o.name.clone());
                }
            }
            Verdict::Baselined => base += 1,
            Verdict::Violated => vio += 1,
            Verdict::Undetermined => und += 1,
        }
        if o.verdict > worst {
            worst = o.verdict;
        }
    }

    Report {
        site: model.site.clone(),
        source: model.source,
        verdict: worst,
        outcomes,
        satisfied: sat,
        violated: vio,
        baselined: base,
        undetermined: und,
        baseline_now_passing: now_passing,
        quantifier: "all-to-all: every source must reach every destination".into(),
    }
}

fn flat_mesh_outcome(model: &Model) -> Outcome {
    // When the sources could not be read, or the policy set was unreadable,
    // this structural check is undecidable for the same reason as every other
    // assertion.
    if !model.groups_read {
        return Outcome {
            name: "no enabled permit grants All→All on all protocols".into(),
            check: "flat_mesh".into(),
            verdict: Verdict::Undetermined,
            reason: Some(Reason::GroupsUnread),
            counterexamples: Vec::new(),
            checked_pairs: 0,
            baseline: None,
        };
    }
    if !model.policies_read {
        return Outcome {
            name: "no enabled permit grants All→All on all protocols".into(),
            check: "flat_mesh".into(),
            verdict: Verdict::Undetermined,
            reason: Some(Reason::PoliciesUnread),
            counterexamples: Vec::new(),
            checked_pairs: 0,
            baseline: None,
        };
    }

    let hit = model.unrestricted_all_to_all();
    let (verdict, counterexamples) = if let Some(p) = hit {
        (
            Verdict::Violated,
            vec![Counterexample {
                src: ALL_GROUP_NAME.to_string(),
                dst: ALL_GROUP_NAME.to_string(),
                port: ANY_PORT.to_string(),
                proto: "all".into(),
                granted_by: Some(p.rule.clone()),
                nearest: None,
            }],
        )
    } else {
        (Verdict::Satisfied, Vec::new())
    };

    Outcome {
        name: "no enabled permit grants All→All on all protocols".into(),
        check: "flat_mesh".into(),
        verdict,
        reason: None,
        counterexamples,
        // One structural check over the whole space — not a pairwise row, but
        // it must never be vacuous.
        checked_pairs: 1,
        baseline: None,
    }
}

fn undecided(t: &TestRow, check: &str, reason: Reason) -> Outcome {
    Outcome {
        name: t.name.clone(),
        check: check.into(),
        verdict: Verdict::Undetermined,
        reason: Some(reason),
        counterexamples: Vec::new(),
        checked_pairs: 0,
        baseline: t.baseline.clone(),
    }
}

fn one(model: &Model, t: &TestRow, check: &str, targets: &[String]) -> Outcome {
    if !model.groups_read {
        return undecided(t, check, Reason::GroupsUnread);
    }
    if !model.policies_read {
        return undecided(t, check, Reason::PoliciesUnread);
    }
    if t.src.is_empty() {
        return undecided(t, check, Reason::EmptySelector);
    }
    let proto = t.proto.to_lowercase();
    if !matches!(proto.as_str(), "tcp" | "udp" | "icmp" | ANY_PROTO) {
        return undecided(t, check, Reason::UnknownProtocol);
    }

    // Resolve both ends BEFORE deciding anything. A test naming a group that
    // does not exist is a malformed declaration, and grading it as "nothing
    // reaches it" would be the surface inventing a security finding out of a
    // typo.
    let mut sources = Vec::new();
    for s in &t.src {
        let full = model.resolve(&model.prefix, s);
        if !model.groups.contains(&full) {
            return undecided(t, check, Reason::UnknownGroup);
        }
        sources.push(full);
    }
    let mut dests = Vec::new();
    for tg in targets {
        let Some((scope, suffix, port)) = parse_target(tg) else {
            return undecided(t, check, Reason::MalformedTarget);
        };
        let prefix = match scope.as_deref() {
            None => model.prefix.clone(),
            // `HOST:` is only meaningful inside a scope that HAS a host. On a
            // persistent site it is undecidable, not self-referential — the
            // parity checker refuses the same thing in a rule.
            Some(HOST_ALIAS) => match model.host_prefix.clone() {
                Some(p) => p,
                None => return undecided(t, check, Reason::UnknownGroup),
            },
            Some(s) => match model.scope_prefixes.get(&s.to_lowercase()) {
                Some(p) => p.clone(),
                None => return undecided(t, check, Reason::UnknownGroup),
            },
        };
        let full = model.resolve(&prefix, &suffix);
        if !model.groups.contains(&full) {
            return undecided(t, check, Reason::UnknownGroup);
        }
        dests.push((full, port));
    }

    let existence = check == "existence";
    let mut counterexamples = Vec::new();
    let mut pairs = 0u32;
    for s in &sources {
        for (d, port) in &dests {
            pairs += 1;
            let hit = model.permit_for(s, d, &proto, port);
            // SEARCH FOR THE VIOLATION. Empty is the pass, and every failure
            // arrives already named.
            match (existence, hit) {
                (true, None) => counterexamples.push(Counterexample {
                    src: s.clone(),
                    dst: d.clone(),
                    port: port.clone(),
                    proto: proto.clone(),
                    granted_by: None,
                    nearest: nearest_miss(model, s, d),
                }),
                (false, Some(p)) => counterexamples.push(Counterexample {
                    src: s.clone(),
                    dst: d.clone(),
                    port: port.clone(),
                    proto: proto.clone(),
                    granted_by: Some(p.rule.clone()),
                    nearest: None,
                }),
                _ => {}
            }
        }
    }

    let verdict = if counterexamples.is_empty() {
        Verdict::Satisfied
    } else if t.baseline.is_some() {
        Verdict::Baselined
    } else {
        Verdict::Violated
    };
    Outcome {
        name: t.name.clone(),
        check: check.into(),
        verdict,
        reason: None,
        counterexamples,
        checked_pairs: pairs,
        baseline: t.baseline.clone(),
    }
}

/// The rule that ALMOST permits this pair — same two groups, wrong port or
/// wrong protocol. Contrast is what localises the fault; "nothing reaches the
/// breakglass" is a fact, "and this rule reaches it on 443" is a diagnosis.
fn nearest_miss(model: &Model, s: &str, d: &str) -> Option<String> {
    if let Some(p) = model.permits.iter().find(|p| p.src == s && p.dst == d) {
        return Some(format!(
            "`{}` permits {s} → {d} but only on {} {}",
            p.rule,
            p.protocol,
            if p.ports.is_empty() { "any port".to_string() } else { p.ports.join(", ") }
        ));
    }
    if let Some(p) = model.permits.iter().find(|p| p.src == s) {
        return Some(format!("the nearest grant from {s} is `{}`, which reaches {}", p.rule, p.dst));
    }
    if let Some(p) = model.permits.iter().find(|p| p.dst == d) {
        return Some(format!("{d} is reached by `{}`, from {}", p.rule, p.src));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests compile the real files in — they run from the source tree and
    /// never inside an image, so this costs no downstream Dockerfile a COPY.
    const TAXONOMY_YAML: &str = include_str!("../../../ops/ansible/netbird_acl_taxonomy.yml");
    const MATRIX_YAML: &str = include_str!("../../../ops/ansible/netbird_acl_matrix.yml");

    fn decl() -> Declaration {
        Declaration::parse(TAXONOMY_YAML, MATRIX_YAML).expect("the declaration must parse")
    }
    fn declared(site: &str) -> Result<Model, DeclError> {
        decl().model(site)
    }

    fn model(permits: &[(&str, &str, &str, &[&str])]) -> Model {
        let mut groups = BTreeSet::new();
        let mut ps = Vec::new();
        for (s, d, proto, ports) in permits {
            groups.insert(s.to_string());
            groups.insert(d.to_string());
            ps.push(Permit {
                src: (*s).into(),
                dst: (*d).into(),
                protocol: (*proto).into(),
                ports: ports.iter().map(|p| (*p).to_string()).collect(),
                rule: format!("{s} → {d}"),
            });
        }
        // Groups that exist but nothing permits.
        for g in ["T-BREAKGLASS", "T-QUARANTINED"] {
            groups.insert(g.into());
        }
        Model {
            site: "t".into(),
            prefix: "T".into(),
            host_prefix: None,
            scope_prefixes: BTreeMap::new(),
            groups,
            permits: ps,
            source: Source::Declared,
            groups_read: true,
            policies_read: true,
        }
    }

    fn row(name: &str, src: &str, proto: &str, accept: &[&str], deny: &[&str]) -> TestRow {
        TestRow {
            name: name.into(),
            src: vec![src.into()],
            proto: proto.into(),
            accept: accept.iter().map(|s| (*s).to_string()).collect(),
            deny: deny.iter().map(|s| (*s).to_string()).collect(),
            baseline: None,
        }
    }

    /// A goatnet with no taxonomy stanza inherits `_default`, its prefix
    /// derived as the upper-cased site id, as the ACL plays derive it
    /// (`ops/ansible/tasks/netbird-taxonomy-default.yml`). Declared in the
    /// matrix, it is a scope; named nowhere, it is still no scope at all.
    #[test]
    fn a_site_without_a_taxonomy_stanza_inherits_default() {
        let tax = "netbird_acl_taxonomy:\n  _default:\n    site_prefix: __DERIVED__\n    \
                   groups:\n    - suffix: INFRA\n    - suffix: OBSERVER\n";
        let mat = "netbird_acl_matrix:\n  efdi:\n    rules:\n    - name: infra to all\n      \
                   sources: [INFRA]\n      destinations: [OBSERVER]\n      protocol: tcp\n      \
                   ports: ['443']\n      action: accept\n";
        let d = Declaration::parse(tax, mat).expect("parse");
        let m = d.model("efdi").expect("efdi inherits _default");
        assert_eq!(m.prefix, "EFDI");
        assert!(m.groups.contains("EFDI-INFRA") && m.groups.contains("EFDI-OBSERVER"));
        assert_eq!(m.permits.len(), 1);
        assert_eq!(
            (m.permits[0].src.as_str(), m.permits[0].dst.as_str()),
            ("EFDI-INFRA", "EFDI-OBSERVER")
        );
        assert!(d.scopes().contains(&"efdi".to_string()));
        assert!(matches!(d.model("nosuchsite"), Err(DeclError::NoSuchSite(_))));
    }

    #[test]
    fn existence_is_satisfied_when_the_port_is_permitted() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["22", "443"])]);
        let r = evaluate(&m, &[row("admin ssh", "ADMIN", "tcp", &["INFRA:22"], &[])]);
        assert_eq!(r.verdict, Verdict::Satisfied);
        assert_eq!(r.outcomes[0].check, "existence");
        assert_eq!(r.outcomes[0].checked_pairs, 1);
    }

    #[test]
    fn permit_covers_proto_handles_all_case_insensitively() {
        let p = Permit {
            src: "All".into(),
            dst: "All".into(),
            protocol: "ALL".into(),
            ports: vec![],
            rule: "test".into(),
        };
        // A permit with protocol "ALL" should cover a specific protocol like "tcp"
        assert!(p.covers_proto("tcp"), "protocol 'ALL' should cover 'tcp'");
    }

    #[test]
    fn flat_mesh_check_is_skipped_if_no_tests_are_provided_to_avoid_standalone_failures() {
        let m = model(&[("All", "All", "all", &[])]);
        let r = evaluate(&m, &[]);
        assert!(!r.outcomes.iter().any(|o| o.check == "flat_mesh"));
    }

    #[test]
    fn flat_mesh_is_undetermined_when_groups_or_policies_are_unread() {
        let mut m = model(&[("All", "All", "all", &[])]);
        m.groups_read = false;
        let r = evaluate(&m, &[row("admin no infra", "ADMIN", "tcp", &[], &["INFRA:22"])]);
        let flat = r.outcomes.iter().find(|o| o.check == "flat_mesh").unwrap();
        assert_eq!(flat.verdict, Verdict::Undetermined);
        assert_eq!(flat.reason, Some(Reason::GroupsUnread));

        m.groups_read = true;
        m.policies_read = false;
        let r2 = evaluate(&m, &[row("admin no infra", "ADMIN", "tcp", &[], &["INFRA:22"])]);
        let flat2 = r2.outcomes.iter().find(|o| o.check == "flat_mesh").unwrap();
        assert_eq!(flat2.verdict, Verdict::Undetermined);
        assert_eq!(flat2.reason, Some(Reason::PoliciesUnread));
    }

    /// The capability that did not exist before: asserting a path SHOULD be
    /// there. AWS's flagship product cannot express it, so a host nothing
    /// reaches is byte-identical to a compliant one.
    #[test]
    fn a_required_path_that_is_missing_is_a_violation_with_a_named_pair() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["443"])]);
        let r = evaluate(&m, &[row("admin ssh", "ADMIN", "tcp", &["INFRA:22"], &[])]);
        assert_eq!(r.verdict, Verdict::Violated);
        let c = &r.outcomes[0].counterexamples[0];
        assert_eq!((c.src.as_str(), c.dst.as_str(), c.port.as_str()), ("T-ADMIN", "T-INFRA", "22"));
        // Contrast: the rule that almost does it.
        assert!(c.nearest.as_ref().unwrap().contains("443"), "{:?}", c.nearest);
    }

    #[test]
    fn isolation_names_the_rule_that_grants_what_should_not_be_granted() {
        let m = model(&[("T-QUARANTINED", "T-DEV", "all", &[])]);
        let r = evaluate(&m, &[row("quarantine holds", "QUARANTINED", "any", &[], &["DEV:*"])]);
        assert_eq!(r.verdict, Verdict::Violated);
        let c = &r.outcomes[0].counterexamples[0];
        assert_eq!(r.outcomes[0].check, "isolation");
        assert_eq!(c.granted_by.as_deref(), Some("T-QUARANTINED → T-DEV"));
    }

    /// The quantifier is all-to-all and it is not negotiable at read time:
    /// one source out of three failing is a violation, and the report names
    /// which one.
    #[test]
    fn the_quantifier_is_all_to_all() {
        let m = model(&[
            ("T-A", "T-INFRA", "tcp", &["22"]),
            ("T-B", "T-INFRA", "tcp", &["22"]),
            ("T-C", "T-INFRA", "tcp", &["443"]),
        ]);
        let mut t = row("everyone ssh", "A", "tcp", &["INFRA:22"], &[]);
        t.src = vec!["A".into(), "B".into(), "C".into()];
        let r = evaluate(&m, &[t]);
        assert_eq!(r.verdict, Verdict::Violated);
        assert_eq!(r.outcomes[0].checked_pairs, 3);
        assert_eq!(r.outcomes[0].counterexamples.len(), 1);
        assert_eq!(r.outcomes[0].counterexamples[0].src, "T-C");
        assert!(r.quantifier.contains("all-to-all"));
    }

    /// A typo must never be graded as a security finding. This is the AWS
    /// split: "no path matches the specified destination port range — verify
    /// that the filters are as intended" is a different class from "the mesh
    /// blocks it".
    #[test]
    fn a_group_that_does_not_exist_is_a_malformed_declaration_not_a_broken_mesh() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["22"])]);
        let r = evaluate(&m, &[row("typo", "ADMIN", "tcp", &["INFAR:22"], &[])]);
        assert_eq!(r.verdict, Verdict::Undetermined);
        let reason = r.outcomes[0].reason.unwrap();
        assert_eq!(reason, Reason::UnknownGroup);
        assert!(reason.is_declaration_fault());
    }

    /// Exposing the third state is not enough — it has to GOVERN. One
    /// undecidable assertion beside a hundred passes is not a pass.
    #[test]
    fn one_undetermined_caps_the_roll_up() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["22"])]);
        let r = evaluate(
            &m,
            &[
                row("fine", "ADMIN", "tcp", &["INFRA:22"], &[]),
                row("typo", "ADMIN", "tcp", &["NOPE:22"], &[]),
            ],
        );
        assert_eq!(r.satisfied, 2);
        assert_eq!(r.undetermined, 1);
        assert_eq!(r.verdict, Verdict::Undetermined);
        assert!(!r.is_clean());
        assert!(Verdict::Undetermined > Verdict::Violated);
    }

    /// An unread source is not a denial. The same rule the matrix obeys.
    #[test]
    fn an_unread_policy_set_decides_nothing() {
        let mut m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["22"])]);
        m.policies_read = false;
        let r = evaluate(&m, &[row("admin ssh", "ADMIN", "tcp", &["INFRA:22"], &[])]);
        assert_eq!(r.verdict, Verdict::Undetermined);
        assert_eq!(r.outcomes[0].reason, Some(Reason::PoliciesUnread));
        assert!(!r.outcomes[0].reason.unwrap().is_declaration_fault());
    }

    /// Baseline-then-ratchet: a known violation is recorded, not alarmed, so
    /// the day-one audit does not drown the new error. And the moment it
    /// passes, the report says so — a baseline nobody removes is a baseline
    /// that becomes permission.
    #[test]
    fn a_baselined_violation_does_not_fail_the_gate_and_announces_when_it_starts_passing() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["443"])]);
        let mut t = row("admin ssh", "ADMIN", "tcp", &["INFRA:22"], &[]);
        t.baseline = Some(Baseline {
            violated_since: "2026-08-12".into(),
            why: "ssh is not open on the substrate yet".into(),
        });
        let r = evaluate(&m, &[t.clone()]);
        assert_eq!(r.verdict, Verdict::Baselined);
        assert!(r.is_clean(), "a recorded violation must not fail the gate");
        assert_eq!(r.baselined, 1);

        let fixed = model(&[("T-ADMIN", "T-INFRA", "tcp", &["22", "443"])]);
        let r = evaluate(&fixed, &[t]);
        assert_eq!(r.verdict, Verdict::Satisfied);
        assert_eq!(r.baseline_now_passing, vec!["admin ssh".to_string()]);
    }

    #[test]
    fn any_port_asks_about_the_pair_not_a_port() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["443"])]);
        let r = evaluate(&m, &[row("reach at all", "ADMIN", "any", &["INFRA:*"], &[])]);
        assert_eq!(r.verdict, Verdict::Satisfied);
        let r = evaluate(&m, &[row("nothing at all", "ADMIN", "any", &[], &["BREAKGLASS:*"])]);
        assert_eq!(r.verdict, Verdict::Satisfied);
    }

    #[test]
    fn a_malformed_target_says_so_rather_than_failing_the_mesh() {
        let m = model(&[("T-ADMIN", "T-INFRA", "tcp", &["22"])]);
        let r = evaluate(&m, &[row("no port", "ADMIN", "tcp", &["INFRA"], &[])]);
        assert_eq!(r.outcomes[0].reason, Some(Reason::MalformedTarget));
    }

    #[test]
    fn an_all_to_all_proto_all_permit_subsumes_every_pair_and_trips_the_flat_mesh_check() {
        // A single unrestricted All→All permit and no other rule: it alone
        // must ground every named pair's isolation counterexample, and be
        // named once as the flat mesh.
        let mut m = model(&[("All", "All", "all", &[])]);
        m.groups.insert("T-ADMIN".into());
        m.groups.insert("T-INFRA".into());
        let r = evaluate(&m, &[row("admin must not reach infra", "ADMIN", "tcp", &[], &["INFRA:22"])]);
        assert_eq!(r.verdict, Verdict::Violated);

        let iso = r
            .outcomes
            .iter()
            .find(|o| o.check == "isolation")
            .expect("isolation outcome");
        assert_eq!(iso.counterexamples.len(), 1);
        let c = &iso.counterexamples[0];
        assert_eq!((c.src.as_str(), c.dst.as_str(), c.port.as_str()), ("T-ADMIN", "T-INFRA", "22"));
        assert_eq!(c.granted_by.as_deref(), Some("All → All"));

        let flat = r
            .outcomes
            .iter()
            .find(|o| o.check == "flat_mesh")
            .expect("flat-mesh outcome");
        assert_eq!(flat.verdict, Verdict::Violated);
        assert_eq!(flat.checked_pairs, 1);
        assert_eq!(flat.counterexamples.len(), 1);
        let fc = &flat.counterexamples[0];
        assert_eq!(fc.src.as_str(), ALL_GROUP_NAME);
        assert_eq!(fc.dst.as_str(), ALL_GROUP_NAME);
        assert_eq!(fc.granted_by.as_deref(), Some("All → All"));
    }

    #[test]
    fn a_permit_sourced_from_all_subsumes_every_source_for_its_destination_and_ports() {
        let mut m = model(&[("All", "T-INFRA", "tcp", &["22"])]);
        m.groups.insert("T-ADMIN".into());

        // Should satisfy an existence check for ADMIN -> INFRA:22
        let r_exist = evaluate(&m, &[row("admin ssh", "ADMIN", "tcp", &["INFRA:22"], &[])]);
        assert_eq!(r_exist.verdict, Verdict::Satisfied);

        // Should NOT satisfy an existence check for ADMIN -> INFRA:443
        let r_exist_443 = evaluate(&m, &[row("admin web", "ADMIN", "tcp", &["INFRA:443"], &[])]);
        assert_eq!(r_exist_443.verdict, Verdict::Violated);

        // Should violate isolation for ADMIN -> INFRA:22
        let r_iso = evaluate(&m, &[row("admin no ssh", "ADMIN", "tcp", &[], &["INFRA:22"])]);
        assert_eq!(r_iso.verdict, Verdict::Violated);
        let c = &r_iso.outcomes[0].counterexamples[0];
        assert_eq!(c.granted_by.as_deref(), Some("All → T-INFRA"));

        // Does NOT trip the flat mesh check, because it's not unrestricted All->All
        let flat = r_iso.outcomes.iter().find(|o| o.check == "flat_mesh").unwrap();
        assert_eq!(flat.verdict, Verdict::Satisfied);
    }

    #[test]
    fn a_permit_destined_for_all_subsumes_every_destination_for_its_source_and_ports() {
        let mut m = model(&[("T-ADMIN", "All", "tcp", &["443"])]);
        m.groups.insert("T-INFRA".into());

        // Should satisfy an existence check for ADMIN -> INFRA:443
        let r_exist = evaluate(&m, &[row("admin web", "ADMIN", "tcp", &["INFRA:443"], &[])]);
        assert_eq!(r_exist.verdict, Verdict::Satisfied);

        // Should NOT satisfy an existence check for ADMIN -> INFRA:22
        let r_exist_22 = evaluate(&m, &[row("admin ssh", "ADMIN", "tcp", &["INFRA:22"], &[])]);
        assert_eq!(r_exist_22.verdict, Verdict::Violated);

        // Should violate isolation for ADMIN -> INFRA:443
        let r_iso = evaluate(&m, &[row("admin no web", "ADMIN", "tcp", &[], &["INFRA:443"])]);
        assert_eq!(r_iso.verdict, Verdict::Violated);
        let c = &r_iso.outcomes[0].counterexamples[0];
        assert_eq!(c.granted_by.as_deref(), Some("T-ADMIN → All"));

        // Does NOT trip the flat mesh check
        let flat = r_iso.outcomes.iter().find(|o| o.check == "flat_mesh").unwrap();
        assert_eq!(flat.verdict, Verdict::Satisfied);
    }

    #[test]
    fn unrestricted_all_to_all_matches_empty_protocol_and_empty_ports() {
        let m = model(&[("All", "All", "", &[])]);
        assert!(m.unrestricted_all_to_all().is_some());
    }

    #[test]
    fn unrestricted_all_to_all_matches_all_protocol_and_empty_ports_case_insensitively() {
        let m1 = model(&[("All", "All", "all", &[])]);
        assert!(m1.unrestricted_all_to_all().is_some());

        let m2 = model(&[("All", "All", "ALL", &[])]);
        assert!(m2.unrestricted_all_to_all().is_some());
    }

    #[test]
    fn restricted_all_to_all_by_port_or_protocol_is_not_flagged_as_flat_mesh() {
        let m1 = model(&[("All", "All", "tcp", &[])]);
        assert!(m1.unrestricted_all_to_all().is_none());

        let m2 = model(&[("All", "All", "all", &["22"])]);
        assert!(m2.unrestricted_all_to_all().is_none());

        let m3 = model(&[("All", "All", "", &["443"])]);
        assert!(m3.unrestricted_all_to_all().is_none());
    }

    #[test]
    fn a_restricted_all_to_all_permit_subsumes_matching_pairs_but_does_not_trip_flat_mesh() {
        let mut m = model(&[("All", "All", "tcp", &["22"])]);
        m.groups.insert("T-ADMIN".into());
        m.groups.insert("T-INFRA".into());

        let r = evaluate(&m, &[
            row("admin no infra 22", "ADMIN", "tcp", &[], &["INFRA:22"]),
            row("admin no infra 443", "ADMIN", "tcp", &[], &["INFRA:443"]),
        ]);

        // 22 should be subsumed (Violated)
        let iso_22 = r.outcomes.iter().find(|o| o.name == "admin no infra 22").unwrap();
        assert_eq!(iso_22.verdict, Verdict::Violated);
        let c = &iso_22.counterexamples[0];
        assert_eq!(c.granted_by.as_deref(), Some("All → All"));

        // 443 should remain Satisfied
        let iso_443 = r.outcomes.iter().find(|o| o.name == "admin no infra 443").unwrap();
        assert_eq!(iso_443.verdict, Verdict::Satisfied);

        // Flat mesh should be Satisfied (not tripped)
        let flat = r.outcomes.iter().find(|o| o.check == "flat_mesh").unwrap();
        assert_eq!(flat.verdict, Verdict::Satisfied);
    }

    #[test]
    fn an_zagros_prod_shaped_model_does_not_regress_against_the_declared_assertions() {
        let mut m = declared("zagros-prod").expect("zagros-prod model");
        // A rule that sources from All and targets one peer resource on netbird-ssh 22.
        m.groups.insert("NETBIRD-SSH".into());
        m.permits.push(Permit {
            src: ALL_GROUP_NAME.into(),
            dst: "NETBIRD-SSH".into(),
            protocol: "tcp".into(),
            ports: vec!["22".into()],
            rule: "SSH Access".into(),
        });

        let tests = decl().tests("zagros-prod");
        let r = evaluate(&m, &tests);

        assert!(
            r.is_clean(),
            "The zagros-prod shipped assertion set should not regress when an All-sourced port-scoped permit is added.\nViolated: {}, Undetermined: {}",
            r.violated, r.undetermined
        );
    }

    // ── the declaration ────────────────────────────────────────────────────




    /// An assertion that checks nothing passes everything. Every declared row
    /// must resolve to at least one pair, or it is a sentence in a YAML file
    /// pretending to be a control.
    #[test]
    fn no_declared_assertion_is_vacuous() {
        for site in decl().scopes() {
            let tests = decl().tests(&site);
            if tests.is_empty() {
                continue;
            }
            let r = evaluate(&declared(&site).expect("model"), &tests);
            for o in &r.outcomes {
                assert!(
                    o.checked_pairs > 0,
                    "{site}: [{}] `{}` checked zero pairs — it cannot fail, so it is not a check",
                    o.check,
                    o.name
                );
            }
        }
    }

}
