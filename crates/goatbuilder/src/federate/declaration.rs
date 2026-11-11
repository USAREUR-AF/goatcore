//! The peering declaration: what `goatbuilder.yaml` `peerings:` may say beyond
//! the four fields the driver always read, and the checks that keep each field
//! with one owner.
//!
//! ONE OWNER PER FIELD. The verified agreement determines what flows between
//! two parties. The goat context's `federation.toml` owns the release sets this
//! party offers (`[data] release`); `goat federate init` puts them in our half
//! of the agreement. A peering's `offer:` may only NARROW that list for one
//! partner: every set it names must be one federation.toml offers, and a set
//! federation.toml does not offer is refused, never added. So no second,
//! hand-kept list can widen what was declared and reviewed in one place.
//!
//! What this module checks, and where:
//!
//! - `problems`: the declaration on its own, offline (`goatbuilder validate`):
//!   reserved words as partner names (GB-120) and the shape of `offer:` (GB-129).
//! - `check_offer`: before a run of `goatbuilder federate <with>` whose peering
//!   declares `offer:`, against the federation.toml phase 3 reads (GB-129).

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::{Config, Peering};

/// Words of `goatbuilder federate` that can never name a partner: the one
/// shared list, `config::RESERVED_PEERING_NAMES` (validate.py mirrors it).
pub use crate::config::RESERVED_PEERING_NAMES as RESERVED_NAMES;

/// The declaration file `goat federate init` reads, in the goat context.
pub const DECLARATION_FILE: &str = "federation.toml";

/// Pins (goatbuilder.yaml `values:`): the per-peering values a person chooses
/// instead of having them allocated (federate/alloc.rs), each optional. The
/// allocation record holds the same keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeeringPins {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wg_interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wg_listen_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_subnet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_listen_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub republisher_metrics_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spire_outbound_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_listen_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_subnet: Option<String>,
}

/// The one declared peering `with` (on `env` when given). `Ok(None)`: not
/// declared. Refuses when `with` is declared on several environments and no
/// `env` says which, so no check ever runs against the wrong entry.
pub fn peering<'a>(cfg: &'a Config, with: &str, env: Option<&str>) -> Result<Option<&'a Peering>> {
    let hits: Vec<&Peering> = cfg
        .peerings
        .iter()
        .filter(|p| p.with == with && env.map(|e| p.environment == e).unwrap_or(true))
        .collect();
    if hits.len() > 1 {
        let envs: Vec<&str> = hits.iter().map(|p| p.environment.as_str()).collect();
        return Err(anyhow::anyhow!(
            "peering '{with}' is declared on several environments ({}); pass --env",
            envs.join(", ")
        ));
    }
    Ok(hits.first().copied())
}

fn at(i: usize, p: &Peering) -> String {
    format!("peerings/{i} (with '{}')", p.with)
}

/// A release set as federation.toml spells it: a set name `X` is the key
/// expression `release/X/**` (ops/permissions/release-sets.yaml); a key
/// expression under `release/` is kept as written. `None`: neither.
pub fn release_key(item: &str) -> Option<String> {
    let name_ok = |s: &str| {
        !s.is_empty()
            && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    if name_ok(item) {
        return Some(format!("release/{item}/**"));
    }
    let rest = item.strip_prefix("release/")?;
    let first = rest.split('/').next().unwrap_or_default();
    let chunks_ok = rest.split('/').all(|c| c == "*" || c == "**" || name_ok(c));
    (name_ok(first) && chunks_ok).then(|| item.to_string())
}

/// The declaration's own problems, each line naming its catalog code: what
/// `goatbuilder validate` reports. Nothing here reads a file.
pub fn problems(cfg: &Config) -> Vec<String> {
    let mut out = Vec::new();
    for (i, p) in cfg.peerings.iter().enumerate() {
        if RESERVED_NAMES.contains(&p.with.as_str()) {
            out.push(format!(
                "{}: '{}' is a word of `goatbuilder federate` and cannot name a partner (GB-120); reserved: {}. Name the partner by its sovereign id",
                at(i, p), p.with, RESERVED_NAMES.join(" ")
            ));
        }
        let Some(offer) = &p.offer else { continue };
        if offer.is_empty() {
            out.push(format!(
                "{}: `offer: []` offers nothing, and an empty list is not a declaration (GB-129). Remove `offer:` to offer every set federation.toml declares, or name the sets",
                at(i, p)
            ));
        }
        let mut seen: Vec<String> = Vec::new();
        for item in offer {
            match release_key(item) {
                None => out.push(format!(
                    "{}: offer '{item}' is neither a release set name nor a key expression under release/ (GB-129). Name a set federation.toml's [data] release offers",
                    at(i, p)
                )),
                Some(k) if seen.contains(&k) => out.push(format!(
                    "{}: offer names {k} twice (GB-129). Name each set once",
                    at(i, p)
                )),
                Some(k) => seen.push(k),
            }
        }
    }
    out
}

/// The federation.toml `goat federate init` reads when this process runs it:
/// `$GOAT_PROFILE_DIR` (else `$HOME/.goat`), context `$GOAT_CONTEXT` (else the
/// `current_context` in its config.toml, else `default`), the same order goat
/// uses (crates/goat-cli/src/profile.rs).
pub fn declaration_path() -> Result<PathBuf> {
    let dir = match std::env::var_os("GOAT_PROFILE_DIR") {
        Some(d) => PathBuf::from(d),
        None => {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                goat_errors::coded(
                    "GB-129",
                    "the peering declares offer:, and neither GOAT_PROFILE_DIR nor HOME is set, so the federation.toml it narrows cannot be found (GB-129). Set GOAT_PROFILE_DIR to the goat profile directory phase 3 uses",
                )
            })?;
            PathBuf::from(home).join(".goat")
        }
    };
    let context = std::env::var("GOAT_CONTEXT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| current_context(&dir.join("config.toml")))
        .unwrap_or_else(|| "default".into());
    Ok(dir.join("contexts").join(context).join(DECLARATION_FILE))
}

fn current_context(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("current_context")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(String::from)
}

/// The `[data] release` list of a federation.toml. `Ok(None)`: the file offers
/// nothing (no `[data]`, or no `release` in it: receive-only).
pub fn declared_release(path: &Path) -> Result<Option<Vec<String>>> {
    let bad = |why: String| {
        goat_errors::coded(
            "GB-129",
            format!(
                "the peering declares offer:, which narrows the release sets of {}, and that file cannot be read (GB-129): {why}. Point GOAT_PROFILE_DIR and GOAT_CONTEXT at the goat context phase 3 runs in, or remove offer:",
                path.display()
            ),
        )
    };
    let text = std::fs::read_to_string(path).map_err(|e| bad(e.to_string()))?;
    let v: toml::Value = toml::from_str(&text).map_err(|e| bad(e.to_string()))?;
    let Some(release) = v.get("data").and_then(|d| d.get("release")) else {
        return Ok(None);
    };
    let list = release
        .as_array()
        .ok_or_else(|| bad("[data] release is not a list".into()))?;
    let mut out = Vec::new();
    for item in list {
        let s = item
            .as_str()
            .ok_or_else(|| bad("[data] release holds a value that is not a string".into()))?;
        out.push(s.to_string());
    }
    Ok(Some(out))
}

/// What a peering's `offer:` leaves of the declared sets: the offered sets as
/// key expressions, in the declaration's order. Refuses (with the reason) any
/// set the declaration does not offer: a local list narrows, never widens.
pub fn narrow(
    offer: &[String],
    declared: Option<&[String]>,
) -> std::result::Result<Vec<String>, String> {
    let declared = declared.unwrap_or_default();
    let mut wanted = Vec::new();
    let mut wider = Vec::new();
    for item in offer {
        match release_key(item) {
            Some(k) if declared.contains(&k) => wanted.push(k),
            Some(k) => wider.push(k),
            None => wider.push(item.clone()),
        }
    }
    if !wider.is_empty() {
        let has = if declared.is_empty() {
            "offers no release set".to_string()
        } else {
            format!("offers only {}", declared.join(", "))
        };
        return Err(format!(
            "offer names {} and federation.toml {has}: a peering's offer may narrow the declared sets, never add one",
            wider.join(", ")
        ));
    }
    Ok(declared
        .iter()
        .filter(|d| wanted.contains(d))
        .cloned()
        .collect())
}

/// The result of checking one peering's `offer:` against federation.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferCheck {
    /// The federation.toml read.
    pub source: PathBuf,
    /// Its `[data] release`.
    pub declared: Vec<String>,
    /// What the peering offers, a subset of `declared`.
    pub offered: Vec<String>,
}

impl OfferCheck {
    /// Whether the peering offers fewer sets than federation.toml declares.
    pub fn narrower(&self) -> bool {
        self.offered.len() < self.declared.len()
    }
}

/// Check the `offer:` of the peering `with` (on `env` when given) against the
/// federation.toml at `path`. `Ok(None)`: the peering declares no `offer:` (or
/// is not declared), and nothing is read. Refuses (GB-129) an offer naming a
/// set the file does not offer, and an unreadable file.
pub fn check_offer(
    cfg: &Config,
    with: &str,
    env: Option<&str>,
    path: &Path,
) -> Result<Option<OfferCheck>> {
    let Some(p) = peering(cfg, with, env)? else {
        return Ok(None);
    };
    let Some(offer) = &p.offer else {
        return Ok(None);
    };
    let declared = declared_release(path)?.unwrap_or_default();
    let offered = narrow(offer, Some(&declared)).map_err(|why| {
        goat_errors::coded(
            "GB-129",
            format!(
                "peering '{with}' on '{}': {why} ({}) (GB-129). Add the set to [data] release in federation.toml, the one owner of the offered sets, or remove it from offer:",
                p.environment,
                path.display()
            ),
        )
    })?;
    Ok(Some(OfferCheck {
        source: path.to_path_buf(),
        declared,
        offered,
    }))
}

/// Before a run of `goatbuilder federate <with>`: a peering that declares
/// `offer:` must offer only sets federation.toml declares. Phase 3 still offers
/// what federation.toml declares (`goat federate init` takes no narrowed list
/// yet), so an offer narrower than the file is refused too: running would offer
/// the partner more than the declaration says. A peering without `offer:` is not
/// checked and nothing is read.
pub fn driver_check(cfg: &Config, with: &str, env: Option<&str>, repo: &Path) -> Result<Option<OfferCheck>> {
    // Only an entry with `offer:` needs checking; ambiguity matters only then.
    let declares = cfg.peerings.iter().any(|p| {
        p.with == with && env.map(|e| p.environment == e).unwrap_or(true) && p.offer.is_some()
    });
    if !declares {
        return Ok(None);
    }
    let p = peering(cfg, with, env)?;
    // The file phase 3 reads: the peering's generated federation.toml, else the context's.
    let path = match p.and_then(|p| super::preflight::peering_declaration(repo, p)) {
        Some(generated) => generated,
        None => declaration_path()?,
    };
    match check_offer(cfg, with, env, &path)? {
        Some(c) => phase3_can_offer(with, c).map(Some),
        None => Ok(None),
    }
}

/// Phase 3 offers exactly what federation.toml declares, so a narrower offer
/// cannot be carried out yet and is refused rather than silently widened.
fn phase3_can_offer(with: &str, c: OfferCheck) -> Result<OfferCheck> {
    if c.narrower() {
        return Err(goat_errors::coded(
            "GB-129",
            format!(
                "peering '{with}' offers {} but {} declares {}, and phase 3 cannot offer fewer sets than federation.toml declares yet (GB-129). Narrow [data] release in federation.toml to what this partner receives, or remove offer:",
                c.offered.join(", "),
                c.source.display(),
                c.declared.join(", ")
            ),
        )
        .into());
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(peerings: serde_json::Value) -> Config {
        serde_json::from_value(json!({
            "schema": "goatbuilder/v1",
            "preset": "full",
            "repo": {"source": "example-org/example-repo", "image_namespace": "ghcr.io/example-org"},
            "components": {},
            "environments": [{"name": "dev", "converger": "push-ansible", "sites": [{"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}}]}],
            "promotion": {"path": [{"name": "dev", "kind": "rung", "environment": "dev"}]},
            "peerings": peerings,
        }))
        .unwrap()
    }

    fn p(with: &str) -> serde_json::Value {
        json!({"with": with, "environment": "dev", "site": "alpha", "edge_class": "edge"})
    }

    fn decl(dir: &Path, data: &str) -> PathBuf {
        let path = dir.join(DECLARATION_FILE);
        std::fs::write(
            &path,
            format!("sovereign_id = \"xaa\"\nterm_days = 30\ncut_contact = \"fed-ops@example.org\"\n\n{data}"),
        )
        .unwrap();
        path
    }

    fn code(e: &anyhow::Error) -> String {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_set_name_is_its_release_key_expression() {
        assert_eq!(
            release_key("REL-ALL").as_deref(),
            Some("release/REL-ALL/**")
        );
        assert_eq!(release_key("all").as_deref(), Some("release/all/**"));
        assert_eq!(
            release_key("release/xab/**").as_deref(),
            Some("release/xab/**")
        );
        for bad in ["", "release/", "release/**", "from/xaa/**", "a b", "-x"] {
            assert_eq!(release_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_offer_narrows_and_never_widens() {
        let declared = vec!["release/all/**".to_string(), "release/xab/**".to_string()];
        assert_eq!(
            narrow(&["xab".into()], Some(&declared)).unwrap(),
            vec!["release/xab/**"]
        );
        assert_eq!(
            narrow(&["release/xab/**".into(), "all".into()], Some(&declared)).unwrap(),
            declared,
            "the declaration's order"
        );
        let e = narrow(&["xab".into(), "xac".into()], Some(&declared)).unwrap_err();
        assert!(
            e.contains("release/xac/**") && e.contains("never add"),
            "{e}"
        );
        let e = narrow(&["xab".into()], None).unwrap_err();
        assert!(e.contains("offers no release set"), "{e}");
    }

    #[test]
    fn reserved_words_and_a_malformed_offer_are_declaration_problems() {
        let mut a = p("status");
        a["offer"] = json!([]);
        let mut b = p("xab");
        b["offer"] = json!(["xab", "release/xab/**", "from/xaa/**"]);
        let out = problems(&cfg(json!([a, b, p("xac")])));
        assert_eq!(out.len(), 4, "{out:#?}");
        assert!(out[0].contains("(GB-120)") && out[0].contains("peerings/0"));
        assert!(out[1].contains("offer: []") && out[1].contains("(GB-129)"));
        assert!(out[2].contains("twice") && out[2].contains("peerings/1"));
        assert!(out[3].contains("from/xaa/**"));
    }

    #[test]
    fn check_offer_reads_federation_toml_only_when_offer_is_declared() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("nowhere.toml");
        let c = cfg(json!([p("xab")]));
        assert_eq!(check_offer(&c, "xab", None, &missing).unwrap(), None);

        let mut b = p("xab");
        b["offer"] = json!(["xab"]);
        let c = cfg(json!([b]));
        let e = check_offer(&c, "xab", None, &missing).unwrap_err();
        assert_eq!(code(&e), "GB-129");
        assert!(e.to_string().contains("cannot be read"), "{e}");

        let path = decl(
            d.path(),
            "[data]\nrelease = [\"release/all/**\", \"release/xab/**\"]\n",
        );
        let got = check_offer(&c, "xab", None, &path).unwrap().unwrap();
        assert_eq!(got.offered, vec!["release/xab/**"]);
        assert!(got.narrower());

        let path = decl(d.path(), "[data]\nrelease = [\"release/all/**\"]\n");
        let e = check_offer(&c, "xab", None, &path).unwrap_err();
        assert_eq!(code(&e), "GB-129");
        assert!(e.to_string().contains("never add"), "{e}");

        // Phase 3 cannot carry out a narrower offer yet: refused, not widened.
        let path = decl(
            d.path(),
            "[data]\nrelease = [\"release/all/**\", \"release/xab/**\"]\n",
        );
        let narrower = check_offer(&c, "xab", None, &path).unwrap().unwrap();
        let e = phase3_can_offer("xab", narrower).unwrap_err();
        assert_eq!(code(&e), "GB-129");
        assert!(e.to_string().contains("cannot offer fewer"), "{e}");
        let path = decl(d.path(), "[data]\nrelease = [\"release/xab/**\"]\n");
        let same = check_offer(&c, "xab", None, &path).unwrap().unwrap();
        assert!(phase3_can_offer("xab", same).is_ok());

        // One partner on two environments: without --env, refused rather than
        // checking whichever entry comes first.
        let two = cfg(
            json!([p("xab"), {"with": "xab", "environment": "prod", "site": "alpha", "edge_class": "edge", "offer": ["xab"]}]),
        );
        assert!(peering(&two, "xab", None).is_err());
        assert!(check_offer(&two, "xab", None, &path).is_err());
        assert_eq!(check_offer(&two, "xab", Some("dev"), &path).unwrap(), None);
        assert!(check_offer(&two, "xab", Some("prod"), &path)
            .unwrap()
            .is_some());

        // Receive-only: no release at all, so any offer widens.
        let path = decl(
            d.path(),
            "[data]\naccept_hz = 1\naccept_bytes_per_message = 1\naccept_distinct_keys = 1\n",
        );
        assert!(check_offer(&c, "xab", None, &path).is_err());
    }

    /// A partner on two environments with no offer: nothing to check, so no
    /// --env is demanded (the driver as before). With an offer on either entry
    /// and no --env, it cannot tell which applies, and refuses.
    #[test]
    fn the_offer_check_demands_env_only_when_an_offer_is_declared() {
        let mut prod = p("xab");
        prod["environment"] = json!("prod");
        let c = cfg(json!([p("xab"), prod.clone()]));
        assert_eq!(driver_check(&c, "xab", None, Path::new("/nonexistent")).unwrap(), None);
        prod["offer"] = json!(["xab"]);
        let c = cfg(json!([p("xab"), prod]));
        let e = driver_check(&c, "xab", None, Path::new("/nonexistent")).unwrap_err();
        assert!(e.to_string().contains("pass --env"), "{e}");
        assert_eq!(driver_check(&c, "xab", Some("dev"), Path::new("/nonexistent")).unwrap(), None);
    }

    /// The offer is checked against the file phase 3 reads: the peering's
    /// generated federation.toml when allocate wrote one, not the context's.
    #[test]
    fn the_offer_check_reads_the_generated_peering_declaration() {
        let repo = tempfile::tempdir().unwrap();
        let mut b = p("xab");
        b["offer"] = json!(["xab"]);
        let c = cfg(json!([b]));
        let env = c.peerings[0].environment.clone();
        let f = repo
            .path()
            .join(format!("ops/federation/{env}/peerings/xab/federation.toml"));
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(
            &f,
            format!(
                "{}: x\n[data]\nrelease = [\"release/xab/**\"]\n",
                crate::federate::peering_files::MARKER
            ),
        )
        .unwrap();
        let got = driver_check(&c, "xab", None, repo.path()).unwrap().unwrap();
        assert_eq!(got.offered, vec!["release/xab/**"]);
    }

    /// One reserved list: validate.py's RESERVED_PEERING_NAMES is the Rust list.
    #[test]
    fn validate_py_reserves_the_same_words() {
        let py = include_str!("../../../../ops/goatbuilder/validate.py");
        let start = py.find("RESERVED_PEERING_NAMES = (").expect("validate.py defines it");
        let body = &py[start..py[start..].find(')').map(|e| start + e).unwrap()];
        let words: Vec<&str> = body.split('"').skip(1).step_by(2).collect();
        assert_eq!(words, RESERVED_NAMES.to_vec());
        for w in ["help", "allocate", "status"] {
            assert!(RESERVED_NAMES.contains(&w), "{w}");
        }
    }
}
