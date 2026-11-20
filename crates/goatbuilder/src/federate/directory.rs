//! `goatbuilder federate directory` — read the peering directory (saltlick).
//!
//! Reads only: no verb here signs, uploads, publishes or withdraws anything.
//! Writing to the directory is phase 5 of a peering run, where the package,
//! its keys and the run's record live (docs/design/peering-directory-service.md).
//!
//! The question these verbs answer is the one a 404 leaves open: is the file
//! missing, or does this directory not serve that route at all? saltlick
//! v0.6.0 and later list every route they serve at `/v1/version`. A route
//! missing from that list is a directory release problem (move its pin); a
//! listed route that answers 404 is a party that has not uploaded the file.
//! An older directory has no `/v1/version`; then the router's plain-text 404
//! (`404 page not found`) marks a missing route, and saltlick's JSON 404 (with
//! `error`) marks a missing file.

use std::io::Read as _;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::{OutputFormat, Report};

/// The variable phase 5 reads the directory from; `--url` overrides it.
pub const URL_ENV: &str = "FED_DIRECTORY_URL";

/// `goatbuilder federate directory` and its three readers.
#[derive(Args, Debug)]
pub struct DirectoryArgs {
    /// The directory's base URL, e.g. https://directory.example. Default $FED_DIRECTORY_URL.
    #[arg(long, global = true)]
    pub url: Option<String>,
    #[command(subcommand)]
    pub action: DirectoryAction,
}

#[derive(Subcommand, Debug)]
pub enum DirectoryAction {
    /// Every party listed, with its expiry and the files it hosts, and the
    /// directory's release.
    Ls,
    /// One party's entry, and each file it hosts with its size and sha256.
    Show {
        /// The party's sovereign id, as listed by `ls`.
        party: String,
    },
    /// Whether a peering can be exchanged through the directory: the party's
    /// entry, its CA and SPIFFE bundle, and its package for one counterparty.
    /// Each missing item says whether the directory lacks the route (move the
    /// directory's pin) or the party has not uploaded the file (ask them to run
    /// phase 5). Exit 0 when everything is there, 1 (GB-372) otherwise.
    Check {
        /// The party whose files are checked.
        party: String,
        /// The counterparty: check the package the party signed for it.
        #[arg(long = "for", value_name = "COUNTERPARTY")]
        counterparty: Option<String>,
    },
}

/// What one GET found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Found {
    /// 200: the file is there.
    Present,
    /// A JSON 404 from saltlick: the route exists, the party or file does not.
    NotHosted,
    /// The path is not a route this directory serves (missing from
    /// /v1/version, or the router's plain-text 404 on an older release).
    RouteMissing,
    /// Anything else: another status, or no answer.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub item: String,
    pub path: String,
    pub found: Found,
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// The directory's own account of itself. `release` is None on a directory
/// older than /v1/version (saltlick before v0.6.0).
#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub url: String,
    pub release: Option<String>,
    pub routes: Option<Vec<String>>,
}

/// One HTTP answer, reduced to what classification needs.
pub struct Answer {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// The transport, injectable so tests need no network.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<Answer, String>;
}

pub struct Http(ureq::Agent);

impl Http {
    pub fn new() -> Self {
        Http(ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build())
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for Http {
    fn get(&self, url: &str) -> Result<Answer, String> {
        let resp = match self.0.get(url).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(t.to_string()),
        };
        let status = resp.status();
        let content_type = resp.header("content-type").unwrap_or("").to_string();
        let mut body = Vec::new();
        resp.into_reader().take(1 << 20).read_to_end(&mut body).map_err(|e| format!("read {url}: {e}"))?;
        Ok(Answer { status, content_type, body })
    }
}

/// The base URL: --url, else $FED_DIRECTORY_URL. Refused (GB-370) when neither
/// is set or the value is not an http(s) URL.
pub fn base_url(flag: Option<&str>) -> anyhow::Result<String> {
    let raw = match flag {
        Some(u) => u.to_string(),
        None => std::env::var(URL_ENV).unwrap_or_default(),
    };
    let u = raw.trim().trim_end_matches('/').to_string();
    if u.is_empty() {
        return Err(goat_errors::coded("GB-370", format!("no directory URL: pass --url or set {URL_ENV}")).into());
    }
    if !(u.starts_with("https://") || u.starts_with("http://")) || u.contains(char::is_whitespace) {
        return Err(goat_errors::coded("GB-370", format!("the directory URL {u:?} is not an http(s) URL")).into());
    }
    Ok(u)
}

/// A sovereign id or counterparty as saltlick accepts it: [a-z0-9-], 2 to 16.
fn valid_id(s: &str) -> bool {
    (2..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn check_id(what: &str, s: &str) -> anyhow::Result<()> {
    if valid_id(s) {
        Ok(())
    } else {
        Err(goat_errors::coded("GB-370", format!("{what} {s:?} is not a directory id: 2 to 16 of a-z, 0-9 and -")).into())
    }
}

/// Read /v1/version. A directory older than saltlick v0.6.0 answers the
/// router's 404; that is reported as an unknown release, not a failure. No
/// answer at all, or something that is not saltlick, is GB-371.
pub fn version(f: &dyn Fetch, base: &str) -> anyhow::Result<Version> {
    let url = format!("{base}/v1/version");
    let a = f.get(&url).map_err(|e| goat_errors::coded("GB-371", format!("the directory at {base} did not answer: {e}")))?;
    if a.status == 404 {
        // Older release: confirm it is a directory at all.
        let h = f.get(&format!("{base}/healthz")).map_err(|e| goat_errors::coded("GB-371", format!("the directory at {base} did not answer: {e}")))?;
        if h.status != 200 {
            return Err(goat_errors::coded("GB-371", format!("{base} is not answering as a peering directory: /healthz gave HTTP {}", h.status)).into());
        }
        return Ok(Version { url: base.to_string(), release: None, routes: None });
    }
    if a.status != 200 {
        return Err(goat_errors::coded("GB-371", format!("{url} answered HTTP {}", a.status)).into());
    }
    let v: serde_json::Value = serde_json::from_slice(&a.body).map_err(|e| goat_errors::coded("GB-371", format!("{url} is not JSON: {e}")))?;
    let release = v.get("release").and_then(|r| r.as_str()).map(str::to_string);
    let routes = v.get("routes").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
    Ok(Version { url: base.to_string(), release, routes })
}

/// Whether a concrete GET path matches a route pattern such as
/// `GET /v1/parties/{sovereign_id}/packages/{counterparty}`.
fn route_serves(pattern: &str, path: &str) -> bool {
    let Some(p) = pattern.strip_prefix("GET ") else { return false };
    let ps: Vec<&str> = p.trim_matches('/').split('/').collect();
    let xs: Vec<&str> = path.trim_matches('/').split('/').collect();
    ps.len() == xs.len() && ps.iter().zip(&xs).all(|(a, b)| (a.starts_with('{') && a.ends_with('}')) || a == b)
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// GET one path and classify the answer.
pub fn probe(f: &dyn Fetch, v: &Version, item: &str, path: &str) -> Probe {
    let mut p = Probe { item: item.to_string(), path: path.to_string(), found: Found::Failed, status: None, detail: None, bytes: None, sha256: None };
    if let Some(routes) = &v.routes {
        if !routes.iter().any(|r| route_serves(r, path)) {
            p.found = Found::RouteMissing;
            p.detail = Some(format!("directory release {} does not serve this route", v.release.as_deref().unwrap_or("?")));
            return p;
        }
    }
    match f.get(&format!("{}{path}", v.url)) {
        Err(e) => p.detail = Some(e),
        Ok(a) => {
            p.status = Some(a.status);
            match a.status {
                200 => {
                    p.found = Found::Present;
                    p.bytes = Some(a.body.len());
                    p.sha256 = Some(sha256_hex(&a.body));
                }
                404 if a.content_type.starts_with("application/json") => {
                    p.found = Found::NotHosted;
                    p.detail = serde_json::from_slice::<serde_json::Value>(&a.body).ok().and_then(|j| j.get("error").and_then(|e| e.as_str()).map(str::to_string));
                }
                404 => {
                    p.found = Found::RouteMissing;
                    p.detail = Some("the directory's router has no such route (plain-text 404): its release predates this route".into());
                }
                s => p.detail = Some(format!("HTTP {s}")),
            }
        }
    }
    p
}

/// What `check` reports.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub directory: Version,
    pub party: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty: Option<String>,
    pub items: Vec<Probe>,
    pub ok: bool,
}

pub fn check(f: &dyn Fetch, base: &str, party: &str, counterparty: Option<&str>) -> anyhow::Result<CheckReport> {
    check_id("party", party)?;
    if let Some(c) = counterparty {
        check_id("counterparty", c)?;
    }
    let v = version(f, base)?;
    let mut items = vec![
        probe(f, &v, "entry", &format!("/v1/parties/{party}")),
        probe(f, &v, "ca", &format!("/v1/parties/{party}/ca")),
        probe(f, &v, "spiffe-bundle", &format!("/v1/parties/{party}/spiffe-bundle")),
    ];
    match counterparty {
        Some(c) => items.push(probe(f, &v, &format!("package for {c}"), &format!("/v1/parties/{party}/packages/{c}"))),
        None => items.push(probe(f, &v, "package", &format!("/v1/parties/{party}/package"))),
    }
    let ok = items.iter().all(|p| p.found == Found::Present);
    Ok(CheckReport { directory: v, party: party.to_string(), counterparty: counterparty.map(str::to_string), items, ok })
}

/// What a missing item means for the person reading, in one line.
fn remedy(p: &Probe) -> &'static str {
    match p.found {
        Found::Present => "",
        Found::NotHosted => "not uploaded: the party runs phase 5 of its peering (it uploads this file)",
        Found::RouteMissing => "the directory does not serve this route: its operator moves the directory to a release that does",
        Found::Failed => "no usable answer: check the URL and the directory's /healthz",
    }
}

fn release_line(v: &Version) -> String {
    match &v.release {
        Some(r) => format!("directory {} (release {r})", v.url),
        None => format!("directory {} (release unknown: older than saltlick v0.6.0, which added /v1/version)", v.url),
    }
}

fn found_word(f: &Found) -> &'static str {
    match f {
        Found::Present => "present",
        Found::NotHosted => "NOT HOSTED",
        Found::RouteMissing => "ROUTE MISSING",
        Found::Failed => "FAILED",
    }
}

pub fn render_check(r: &CheckReport) -> String {
    let mut s = release_line(&r.directory) + "\n";
    s += &format!("party {}{}\n", r.party, r.counterparty.as_deref().map(|c| format!(", counterparty {c}")).unwrap_or_default());
    for p in &r.items {
        s += &format!("  {:<22} {:<14} {}\n", p.item, found_word(&p.found), p.path);
        if p.found != Found::Present {
            if let Some(d) = &p.detail {
                s += &format!("  {:<22} {}\n", "", d);
            }
            s += &format!("  {:<22} {}\n", "", remedy(p));
        }
    }
    s += if r.ok { "everything the peering reads is in the directory\n" } else { "something the peering reads is missing (above)\n" };
    s
}

fn emit<T: Serialize>(value: &T, table: impl FnOnce() -> String, format: OutputFormat) {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(value).unwrap_or_default()),
        OutputFormat::Yaml => print!("{}", serde_yaml::to_string(value).unwrap_or_default()),
        OutputFormat::Table => print!("{}", table()),
    }
}

/// Run one directory verb. The verb prints its own view (or JSON/YAML
/// document); the Report carries only the exit. A missing item is GB-372: at a
/// table the coded line follows the view; under -o json|yaml the document's
/// `ok: false` says it and the exit is 1, so stdout stays one document.
pub fn run(args: &DirectoryArgs, format: OutputFormat) -> anyhow::Result<Report> {
    let result = run_with(&Http::new(), args, format);
    let mut r = Report::new("federate", vec![], 1);
    r.printed = true;
    match result {
        Ok(()) => Ok(r),
        Err(e) if e.downcast_ref::<Missing>().is_some() => {
            if matches!(format, OutputFormat::Table) {
                let m = e.downcast_ref::<Missing>().map(|m| m.0.clone()).unwrap_or_default();
                Err(goat_errors::coded("GB-372", m).into())
            } else {
                r.ok = false;
                r.exit = 1;
                Ok(r)
            }
        }
        Err(e) => Err(e),
    }
}

/// A check that ran and found something missing (as opposed to a refusal).
#[derive(Debug)]
pub struct Missing(pub String);
impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Missing {}

pub fn run_with(f: &dyn Fetch, args: &DirectoryArgs, format: OutputFormat) -> anyhow::Result<()> {
    let base = base_url(args.url.as_deref())?;
    match &args.action {
        DirectoryAction::Ls => {
            let v = version(f, &base)?;
            let a = f.get(&format!("{base}/v1/parties")).map_err(|e| goat_errors::coded("GB-371", format!("{base}/v1/parties: {e}")))?;
            if a.status != 200 {
                return Err(goat_errors::coded("GB-371", format!("{base}/v1/parties answered HTTP {}", a.status)).into());
            }
            let j: serde_json::Value = serde_json::from_slice(&a.body).map_err(|e| goat_errors::coded("GB-371", format!("{base}/v1/parties is not JSON: {e}")))?;
            let parties = j.get("parties").cloned().unwrap_or(serde_json::Value::Array(vec![]));
            let out = serde_json::json!({ "directory": v, "parties": parties });
            emit(&out, || {
                let mut s = release_line(&v) + "\n";
                s += &format!("  {:<16} {:<22} {:<11} {:<8} hosted\n", "party", "spec", "expires", "expired");
                for p in parties.as_array().into_iter().flatten() {
                    let g = |k: &str| p.get(k).and_then(|x| x.as_str()).unwrap_or("-").to_string();
                    let hosted: Vec<&str> = ["package", "ca", "spiffe_bundle"].into_iter().filter(|k| p.get(*k).is_some()).collect();
                    let expired = p.get("expired").and_then(|x| x.as_bool()).map(|b| if b { "yes" } else { "no" }).unwrap_or("-");
                    s += &format!("  {:<16} {:<22} {:<11} {:<8} {}\n", g("sovereign_id"), g("spec"), g("expires"), expired, hosted.join(","));
                }
                s
            }, format);
            Ok(())
        }
        DirectoryAction::Show { party } => {
            check_id("party", party)?;
            let v = version(f, &base)?;
            let entry = probe(f, &v, "entry", &format!("/v1/parties/{party}"));
            if entry.found != Found::Present {
                let r = CheckReport { directory: v, party: party.clone(), counterparty: None, items: vec![entry], ok: false };
                emit(&r, || render_check(&r), format);
                return Err(Missing(format!("{party} has no entry in the directory")).into());
            }
            let items: Vec<Probe> = std::iter::once(entry)
                .chain(["package", "ca", "spiffe-bundle"].iter().map(|k| probe(f, &v, k, &format!("/v1/parties/{party}/{k}"))))
                .collect();
            let r = CheckReport { directory: v, party: party.clone(), counterparty: None, ok: true, items };
            emit(&r, || {
                let mut s = release_line(&r.directory) + "\n" + &format!("party {}\n", r.party);
                for p in &r.items {
                    match &p.sha256 {
                        Some(h) => s += &format!("  {:<14} {:<7} bytes {:<6} sha256 {h}\n", p.item, "present", p.bytes.unwrap_or(0)),
                        None => s += &format!("  {:<14} {}\n", p.item, found_word(&p.found)),
                    }
                }
                s += "  per-counterparty packages are not listed by the directory; check one with `check <party> --for <counterparty>`\n";
                s
            }, format);
            Ok(())
        }
        DirectoryAction::Check { party, counterparty } => {
            let r = check(f, &base, party, counterparty.as_deref())?;
            emit(&r, || render_check(&r), format);
            if r.ok {
                Ok(())
            } else {
                let missing: Vec<String> = r.items.iter().filter(|p| p.found != Found::Present).map(|p| format!("{} ({})", p.item, found_word(&p.found).to_lowercase())).collect();
                Err(Missing(format!("the directory lacks what the peering reads: {}", missing.join(", "))).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake(HashMap<String, (u16, &'static str, &'static str)>);
    impl Fetch for Fake {
        fn get(&self, url: &str) -> Result<Answer, String> {
            let path = url.trim_start_matches("https://d.example");
            match self.0.get(path) {
                Some((s, ct, b)) => Ok(Answer { status: *s, content_type: ct.to_string(), body: b.as_bytes().to_vec() }),
                None if path.starts_with("/v1/parties/") && (path.ends_with("/ca") || path.contains("/packages/") || path.ends_with("/package") || path.ends_with("/spiffe-bundle") || path.matches('/').count() == 3) => {
                    Ok(Answer { status: 404, content_type: "application/json".into(), body: br#"{"error":"not hosted"}"#.to_vec() })
                }
                None => Ok(Answer { status: 404, content_type: "text/plain; charset=utf-8".into(), body: b"404 page not found\n".to_vec() }),
            }
        }
    }
    const J: &str = "application/json";
    const ROUTES: &str = r#"{"release":"v0.6.0","routes":["GET /healthz","GET /v1/version","GET /v1/parties","GET /v1/parties/{sovereign_id}","GET /v1/parties/{sovereign_id}/package","GET /v1/parties/{sovereign_id}/packages/{counterparty}","GET /v1/parties/{sovereign_id}/ca","GET /v1/parties/{sovereign_id}/spiffe-bundle"]}"#;

    fn fake(new: bool, extra: &[(&str, u16, &'static str, &'static str)]) -> Fake {
        let mut m = HashMap::new();
        m.insert("/healthz".to_string(), (200, J, r#"{"status":"ok"}"#));
        if new {
            m.insert("/v1/version".to_string(), (200, J, ROUTES));
        }
        for (p, s, ct, b) in extra {
            m.insert(p.to_string(), (*s, *ct, *b));
        }
        Fake(m)
    }

    #[test]
    fn a_new_directory_tells_a_missing_file_from_a_present_one() {
        let f = fake(true, &[("/v1/parties/xaa", 200, J, "{}"), ("/v1/parties/xaa/ca", 200, "application/pem-certificate-chain", "PEM"), ("/v1/parties/xaa/spiffe-bundle", 200, J, "{}")]);
        let r = check(&f, "https://d.example", "xaa", Some("xbb")).unwrap();
        assert_eq!(r.directory.release.as_deref(), Some("v0.6.0"));
        let pkg = r.items.last().unwrap();
        assert_eq!(pkg.found, Found::NotHosted);
        assert!(!r.ok);
        assert_eq!(r.items[1].found, Found::Present);
        assert_eq!(r.items[1].sha256.as_deref().unwrap().len(), 64);
    }

    #[test]
    fn an_old_directory_without_the_route_reads_route_missing() {
        // Before v0.5.0 there was no per-counterparty route: the router's plain 404.
        let mut f = fake(false, &[("/v1/parties/xaa", 200, J, "{}")]);
        f.0.insert("/v1/parties/xaa/packages/xbb".into(), (404, "text/plain; charset=utf-8", "404 page not found\n"));
        let r = check(&f, "https://d.example", "xaa", Some("xbb")).unwrap();
        assert_eq!(r.directory.release, None);
        assert_eq!(r.items.last().unwrap().found, Found::RouteMissing);
        assert!(render_check(&r).contains("moves the directory to a release"));
    }

    #[test]
    fn a_route_absent_from_version_is_missing_without_a_request() {
        let f = fake(true, &[]);
        let v = version(&f, "https://d.example").unwrap();
        assert_eq!(probe(&f, &v, "x", "/v1/parties/xaa/receipts").found, Found::RouteMissing);
        assert!(route_serves("GET /v1/parties/{sovereign_id}/packages/{counterparty}", "/v1/parties/xaa/packages/xbb"));
        assert!(!route_serves("GET /v1/parties/{sovereign_id}/packages/{counterparty}", "/v1/parties/xaa/package"));
    }

    #[test]
    fn the_url_and_ids_are_checked_before_any_request() {
        assert!(base_url(Some("")).is_err());
        assert!(base_url(Some("directory.example")).is_err());
        assert_eq!(base_url(Some("https://d.example/")).unwrap(), "https://d.example");
        assert!(check(&fake(true, &[]), "https://d.example", "X/../y", None).is_err());
        assert!(valid_id("can-jisr") && !valid_id("a") && !valid_id("UPPER"));
    }

    #[test]
    fn something_that_is_not_a_directory_is_gb_371() {
        let f = Fake(HashMap::new());
        let e = version(&f, "https://d.example").unwrap_err();
        assert!(e.to_string().contains("not answering as a peering directory"), "{e}");
    }
}
