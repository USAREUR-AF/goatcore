//! `update` — goatbuilder replaces itself with the release its goatnet serves,
//! and tells the operator, at most once a day, when it is behind.
//!
//! The same contract as `goat update` (crates/goat-cli/src/cmd/update.rs), on
//! the same core (crates/goat-selfupdate): the surface is
//! `get.<site_id>.<site_dns_zone>/goatbuilder/`, which the goat-get role serves
//! from `goat_get_goatbuilder_release_tag` (docs/design/goatbuilder-distribution.md).
//!
//!   - The surface comes from, in order: `GOATBUILDER_UPDATE_URL`; `--site`,
//!     read from the site's group_vars (the facts the role renders the hostname
//!     from); the surface the last successful run used, remembered in
//!     `~/.goatbuilder/update-check.json`. None of them → GB-072.
//!   - The download is checked against the surface's `<asset>.sha256`, which
//!     must agree with the release's `SHA256SUMS` (GB-074 when they disagree).
//!     The keyless signature over `SHA256SUMS` (`SHA256SUMS.cosign-bundle`) is
//!     verified in process against the release workflow's identity
//!     (update/signature.rs), with no cosign and offline from the bundle; a
//!     signature that does not verify, or is not served, is GB-261. Both are
//!     checked before the installed binary is touched. The replace is atomic.
//!   - version.json and the checksum are capped whole; the binary is bounded by
//!     a stall (no bytes for 30 s), not a total, because the mesh path to
//!     get.<site> can run at a few hundred KB/s (goat #2134).
//!   - Already at the served version: one row, exit 0. `--force` reinstalls.
//!     `--check` reads and reports only: exit 0 current, 3 behind.
//!
//! The notice ([`notice`]) is best-effort and silent on any failure: a cached
//! answer, refreshed by one bounded fetch at most once a day, printed to a
//! terminal's stderr at most once a day, never in `-o json|yaml`, and off with
//! `GOATBUILDER_NO_VERSION_CHECK` (or goat's `GOAT_NO_VERSION_CHECK`).


mod signature;

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{Check, OutputFormat, Report, EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

/// version.json and the `.sha256` are a few bytes: each request is capped whole.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// The binary is bounded by a stall: no bytes for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// The notice's fetch: tight, so a cold cache off the mesh costs a run little.
const NOTICE_TIMEOUT: Duration = Duration::from_millis(1500);
/// doctor's fetch: an explicit check, so a little more patience.
const DOCTOR_TIMEOUT: Duration = Duration::from_secs(5);
/// The cached answer is refreshed, and the notice shown, at most this often.
const DAY: u64 = 24 * 60 * 60;
/// The largest binary this will read (the releases are a few MB).
const MAX_BINARY: u64 = 512 * 1024 * 1024;

pub const SUPPRESS_ENV: &str = "GOATBUILDER_NO_VERSION_CHECK";
pub const URL_ENV: &str = "GOATBUILDER_UPDATE_URL";

pub struct Options<'a> {
    pub site: Option<&'a str>,
    pub check: bool,
    pub force: bool,
    pub output: OutputFormat,
    /// `--timeout <secs>` (GOATBUILDER_TIMEOUT): bound the connect/reach phase —
    /// the TCP connect and the small version/checksum fetches — so an
    /// unreachable surface fails fast. The binary download stays stall-bounded.
    pub timeout: Option<u64>,
}

/// This binary's version (`goatbuilder --version` reports the same string).
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Where the served release lives and how that was decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    /// `https://get.<site>.<zone>/goatbuilder`, no trailing slash.
    pub base: String,
    pub site: Option<String>,
    pub from: String,
}

/// The surface, from the override, `--site`, or the remembered one (GB-072).
pub fn resolve(repo: &Path, site: Option<&str>) -> Result<Surface> {
    if let Some(s) = url_override() {
        return Ok(s);
    }
    if let Some(site) = site {
        return from_site(repo, site);
    }
    if let Some(c) = read_cache().filter(|c| !c.surface.is_empty()) {
        return Ok(Surface { base: c.surface, site: c.site.clone(), from: "the surface the last update used (~/.goatbuilder/update-check.json)".into() });
    }
    Err(goat_errors::coded(
        "GB-072",
        "cannot resolve goatbuilder's download surface: pass --site <site> (a goatnet that serves goatbuilder), or set GOATBUILDER_UPDATE_URL=https://get.<site>.<zone>/goatbuilder",
    )
    .into())
}

fn url_override() -> Option<Surface> {
    let v = std::env::var(URL_ENV).ok()?;
    let v = v.trim().trim_end_matches('/');
    (!v.is_empty()).then(|| Surface { base: v.to_string(), site: None, from: URL_ENV.into() })
}

/// `get.<site_id>.<site_dns_zone>/<subdir>` from the site's group_vars: the
/// facts the goat-get role renders `goat_get_hostname` from.
pub fn from_site(repo: &Path, site: &str) -> Result<Surface> {
    let group = format!("{}_hosts", site.trim_end_matches("_hosts").replace('-', "_"));
    let dir = repo.join("ops/ansible/group_vars").join(&group);
    if !dir.is_dir() {
        return Err(goat_errors::coded(
            "GB-072",
            format!(
                "`{site}` is not a goatnet site this checkout declares (no ops/ansible/group_vars/{group}/ under {}): run from a goat checkout (or -C <checkout>), or set GOATBUILDER_UPDATE_URL",
                repo.display()
            ),
        )
        .into());
    }
    let v = crate::doctor::group_scalars(repo, std::slice::from_ref(&group));
    let enabled = v.get("goat_get_goatbuilder_enabled").map(|s| s.as_str()) == Some("true");
    if !enabled {
        return Err(goat_errors::coded(
            "GB-072",
            format!("{site} does not serve goatbuilder: goat_get_goatbuilder_enabled is not true in ops/ansible/group_vars/{group}/ — pick a site that sets it, or set GOATBUILDER_UPDATE_URL"),
        )
        .into());
    }
    let literal = |k: &str| v.get(k).filter(|s| !s.is_empty() && !s.contains("{{")).cloned();
    let host = match literal("goat_get_hostname") {
        Some(h) => h,
        None => match (literal("site_id"), literal("site_dns_zone")) {
            (Some(id), Some(zone)) => format!("get.{id}.{zone}"),
            _ => {
                return Err(goat_errors::coded("GB-072", format!("ops/ansible/group_vars/{group}/ declares no literal site_id and site_dns_zone, so get.<site>.<zone> cannot be named; set GOATBUILDER_UPDATE_URL")).into());
            }
        },
    };
    let sub = literal("goat_get_goatbuilder_www_subdir").unwrap_or_else(|| "goatbuilder".into());
    Ok(Surface { base: format!("https://{host}/{}", sub.trim_matches('/')), site: Some(site.to_string()), from: format!("--site {site} (ops/ansible/group_vars/{group}/)") })
}

/// The default bound on establishing a connection; `--timeout` overrides it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

fn agent(read: Duration) -> ureq::Agent {
    agent_c(CONNECT_TIMEOUT, read)
}

/// An agent with an explicit connect bound, so `--timeout` can shorten the
/// connect/reach phase without touching the read (stall) bound of a download.
fn agent_c(connect: Duration, read: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(connect).timeout_read(read).build()
}

fn stall_timeout() -> Duration {
    // A slow-link or test knob; the default suits the mesh.
    std::env::var("GOATBUILDER_UPDATE_STALL_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(STALL_TIMEOUT)
}

fn fetch_text(agent: &ureq::Agent, url: &str, total: Duration) -> std::result::Result<String, String> {
    let resp = agent.get(url).timeout(total).call().map_err(|e| describe(url, e))?;
    resp.into_string().map_err(|e| format!("read {url}: {e}"))
}

fn describe(url: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("{url} answered HTTP {code}"),
        // The transport's own words usually name the URL; naming it again reads
        // as two failures of two different things.
        ureq::Error::Transport(t) => {
            let t = t.to_string();
            let said = if t.contains(url) { t } else { format!("{url}: {t}") };
            format!("{said} (is this machine on the goatnet's mesh?)")
        }
    }
}

/// The served version, from `<base>/version.json`.
fn latest(agent: &ureq::Agent, base: &str, total: Duration) -> std::result::Result<String, String> {
    let url = format!("{base}/version.json");
    let body = fetch_text(agent, &url, total)?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("{url} is not JSON: {e}"))?;
    v.get("version")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| format!("{url} names no version"))
}

fn ok(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
}

fn unread(text: String) -> anyhow::Error {
    goat_errors::coded("GB-073", text).into()
}

pub fn run(repo: &Path, o: &Options) -> Result<Report> {
    let s = resolve(repo, o.site)?;
    let running = current();
    // `--timeout` shortens the connect/reach phase and caps the small version
    // and checksum fetches; without it, the defaults (15 s connect, 30 s read).
    let bound = o.timeout.map(Duration::from_secs);
    let connect = bound.unwrap_or(CONNECT_TIMEOUT);
    let fetch_total = bound.unwrap_or(FETCH_TIMEOUT);
    tracing::debug!(surface = %s.base, connect = ?connect, fetch_total = ?fetch_total, "update: reaching the served version");
    let probe = agent_c(connect, fetch_total);
    let latest = latest(&probe, &s.base, fetch_total).map_err(|e| unread(format!("cannot read the served version: {e}")))?;
    tracing::debug!(running, served = %latest, "update: versions");
    remember(&s, &latest);

    let mut rows = vec![ok("surface", format!("{}/  (from {})", s.base, s.from))];
    let behind = goat_selfupdate::is_outdated(running, &latest);
    let site_flag = s.site.as_deref().map(|x| format!(" --site {x}")).unwrap_or_default();
    if o.check {
        rows.push(Check {
            name: "version".into(),
            ok: !behind,
            detail: if behind { format!("running {running}, served {latest}: an update is available") } else { format!("running {running}, served {latest}: current") },
            fix: behind.then(|| format!("goatbuilder update{site_flag}")),
        });
        return Ok(Report::new("update", rows, EXIT_NOT_CONVERGED));
    }
    if !behind && !o.force {
        let detail = if running == latest {
            format!("running {running}, served {latest}: already up to date")
        } else {
            format!("running {running}, served {latest}: this binary is not older than the served one; nothing to do (--force installs the served one)")
        };
        rows.push(ok("version", detail));
        return Ok(Report::new("update", rows, EXIT_OK));
    }
    rows.push(ok("version", format!("running {running}, served {latest}{}", if behind { "" } else { " (--force)" })));

    let asset = match goat_selfupdate::platform() {
        Some(p) if !p.starts_with("windows") => format!("goatbuilder-{p}"),
        _ => {
            return Err(goat_errors::coded(
                "GB-072",
                format!("no goatbuilder release is built for {}/{}; build it from source (cargo build -p goatbuilder)", std::env::consts::OS, std::env::consts::ARCH),
            )
            .into())
        }
    };
    let bin_url = format!("{}/{asset}", s.base);
    let sha_url = format!("{bin_url}.sha256");
    // The checksum and its signature first: a surface that cannot vouch for the
    // binary is refused before the long download.
    let sha_txt = fetch_text(&probe, &sha_url, fetch_total).map_err(|e| unread(format!("cannot read the checksum: {e}")))?;
    let want = goat_selfupdate::parse_checksum(&sha_txt).ok_or_else(|| unread(format!("{sha_url} holds no sha256 digest")))?;
    let signature = signed(repo, &probe, &s.base, &asset, &want, &latest, fetch_total)?;
    if matches!(o.output, OutputFormat::Table) {
        eprintln!("downloading {bin_url} …");
    }
    let bin = download(&bin_url, connect)?;
    if let Err(m) = goat_selfupdate::verify(&bin, &want) {
        return Err(goat_errors::coded(
            "GB-074",
            format!("checksum mismatch, refusing to install {bin_url}: {asset}.sha256 says {}, the download is {}; the installed binary is untouched", m.want, m.got),
        )
        .into());
    }
    rows.push(ok("download", format!("{bin_url} ({} bytes)", bin.len())));
    rows.push(ok("checksum", format!("sha256 {want} matches {asset}.sha256 and SHA256SUMS")));
    rows.push(ok("signature", signature));

    let exe = std::env::current_exe().map_err(|e| goat_errors::coded("GB-075", format!("cannot resolve this binary's path: {e}")))?;
    let left = goat_selfupdate::install(&exe, &bin, &format!("goatbuilder-update-{latest}")).map_err(|e| goat_errors::coded("GB-075", e.to_string()))?;
    let mut detail = format!("{running} → {latest} at {}", exe.display());
    if let Some(old) = left {
        detail.push_str(&format!("; the previous binary is {} until the next update", old.display()));
    }
    rows.push(ok("install", detail));
    Ok(Report::new("update", rows, EXIT_REFUSED))
}

/// SHA256SUMS, the release's one signed file, must list `asset` at the digest
/// its `.sha256` gives, and its signature must verify for the release workflow
/// at the served version's tag (update/signature.rs,
/// docs/design/goatbuilder-distribution.md). Returns the signature row's text.
fn signed(repo: &Path, agent: &ureq::Agent, base: &str, asset: &str, want: &str, version: &str, total: Duration) -> Result<String> {
    let sums_url = format!("{base}/SHA256SUMS");
    let sums = fetch_text(agent, &sums_url, total).map_err(|e| unread(format!("cannot read the signed checksums: {e}")))?;
    match listed(&sums, asset) {
        Some(d) if d == want => {}
        Some(d) => return Err(goat_errors::coded("GB-074", format!("checksum mismatch, refusing to install: {asset}.sha256 says {want}, SHA256SUMS says {d}; the installed binary is untouched")).into()),
        None => return Err(goat_errors::coded("GB-074", format!("checksum mismatch, refusing to install: SHA256SUMS at {base}/ does not list {asset}; the installed binary is untouched")).into()),
    }
    let bundle_url = format!("{base}/SHA256SUMS.cosign-bundle");
    let bundle = fetch_text(agent, &bundle_url, total)
        .map_err(|e| sig_refused(format!("the surface serves no signature for SHA256SUMS ({bundle_url}: {e})")))?;
    // The signer is the checkout's repo.release_source, else its repo.source;
    // run outside a checkout, the repository whose workflow built this binary.
    // No repository is named in the code. The signature is always checked: an
    // unreadable config never skips it, and an unknown signer refuses.
    let configured = crate::config::load(&repo.join("ops/goatbuilder/goatbuilder.yaml")).ok().and_then(|c| c.repo).map(|r| r.release_source.unwrap_or(r.source));
    let source = release_signer(configured, BUILT_BY).ok_or_else(|| {
        goat_errors::coded(
            "GB-078",
            "signature not checked, refusing to install: no release signer is known. Run from a checkout whose ops/goatbuilder/goatbuilder.yaml names repo.release_source (or repo.source); this binary was not built by a release workflow, so it names none itself. The installed binary is untouched",
        )
    })?;
    // Checked in process (update/signature.rs): no cosign, and nothing fetched
    // beyond the bundle; the trusted root is embedded, plus the operator's.
    let root = signature::TrustRoot::resolve().map_err(|e| sig_refused(format!("cannot read the trusted root: {e}")))?;
    let expect = signature::Expect { signer: &source, version, workflow: signature::RELEASE_WORKFLOW };
    match signature::verify(sums.as_bytes(), &bundle, &expect, &root) {
        Ok(v) => Ok(format!("SHA256SUMS signed by {} (Rekor entry {}, {})", v.identity, v.log_index, chrono::DateTime::from_timestamp(v.integrated_time, 0).map(|d| d.to_rfc3339()).unwrap_or_default())),
        Err(r) => Err(sig_refused(format!("the signature over {base}/SHA256SUMS does not verify for the {source} release workflow at {}{version}: {r}", signature::TAG_PREFIX))),
    }
}

/// The digest SHA256SUMS lists for `name` (`<sha256>  <name>`, a leading `*`
/// on the name ignored), lowercased; None when it does not list `name`.
fn listed(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let (d, n) = (f.next()?, f.next()?);
        (n.trim_start_matches('*') == name).then(|| d.to_lowercase())
    })
}

/// `update verify-release <dir>`: verify a downloaded goatbuilder release
/// without installing it. The same two checks `update` makes before it
/// installs: the keyless signature over `<dir>/SHA256SUMS`
/// ([`signature::verify`], against the release workflow of `repo` at
/// `goatbuilder/v<version>`), and every other file in `<dir>` against that
/// SHA256SUMS: a file at the digest SHA256SUMS lists for it, and a
/// `<asset>.sha256` naming the digest SHA256SUMS lists for `<asset>`.
pub struct VerifyRelease<'a> {
    pub dir: &'a Path,
    /// `<owner>/<repo>` the release was built in.
    pub repo: &'a str,
    /// The release's version (`0.6.1`), its tag `goatbuilder/v<version>`.
    pub version: &'a str,
    /// A trusted_root.json trusted beside the embedded public one.
    pub trusted_root: Option<&'a Path>,
}

pub fn verify_release(o: &VerifyRelease) -> Result<Report> {
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-+/".contains(c));
    if !plain(o.repo) || o.repo.matches('/').count() != 1 {
        return Err(goat_errors::coded("GB-242", format!("--signer '{}' is not <owner>/<repo>", o.repo)).into());
    }
    if !plain(o.version) || o.version.contains('/') {
        return Err(goat_errors::coded("GB-242", format!("--version '{}' is not a version (the release tag is goatbuilder/v<version>)", o.version)).into());
    }
    let read = |name: &str| std::fs::read(o.dir.join(name));
    let sums = read("SHA256SUMS").map_err(|e| goat_errors::coded(SIGNATURE_REFUSED, format!("refused: {}/SHA256SUMS cannot be read ({e}) (check: sums)", o.dir.display())))?;
    let bundle = read("SHA256SUMS.cosign-bundle").map_err(|e| goat_errors::coded(SIGNATURE_REFUSED, format!("refused: the release has no signature over SHA256SUMS, {}/SHA256SUMS.cosign-bundle ({e}) (check: bundle)", o.dir.display())))?;
    let root = signature::TrustRoot::resolve_with(o.trusted_root).map_err(|e| goat_errors::coded(SIGNATURE_REFUSED, format!("refused: cannot read the trusted root: {e} (check: trusted-root)")))?;
    let expect = signature::Expect { signer: o.repo, version: o.version, workflow: signature::RELEASE_WORKFLOW };
    let v = signature::verify(&sums, &String::from_utf8_lossy(&bundle), &expect, &root).map_err(|r| {
        goat_errors::coded(
            SIGNATURE_REFUSED,
            format!("refused: SHA256SUMS does not verify for the {} release workflow at {}{}: {r} (check: {})", o.repo, signature::TAG_PREFIX, o.version, r.check),
        )
    })?;
    let sums = String::from_utf8_lossy(&sums).to_string();
    let mut names: Vec<String> = std::fs::read_dir(o.dir)
        .map_err(|e| goat_errors::coded("GB-074", format!("refused: {} cannot be listed: {e} (check: listed)", o.dir.display())))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "SHA256SUMS" && n != "SHA256SUMS.cosign-bundle")
        .collect();
    names.sort();
    let mut rows = vec![ok("signature", format!("SHA256SUMS signed by {} (Rekor entry {})", v.identity, v.log_index))];
    for name in &names {
        let path = o.dir.join(name);
        if !std::fs::symlink_metadata(&path).map(|m| m.is_file()).unwrap_or(false) {
            return Err(goat_errors::coded("GB-074", format!("refused: {name} in the release is not a plain file (check: listed)")).into());
        }
        let bytes = std::fs::read(&path).map_err(|e| goat_errors::coded("GB-074", format!("refused: {name} cannot be read ({e}) (check: listed)")))?;
        let detail = match (listed(&sums, name), name.strip_suffix(".sha256").and_then(|a| listed(&sums, a))) {
            (Some(want), _) => {
                let got = goat_selfupdate::sha256_hex(&bytes);
                if got != want {
                    return Err(goat_errors::coded("GB-074", format!("refused: {name} is sha256 {got}, and SHA256SUMS lists {want} (check: listed)")).into());
                }
                format!("sha256 {want}, as SHA256SUMS lists it")
            }
            (None, Some(want)) => {
                let says = goat_selfupdate::parse_checksum(&String::from_utf8_lossy(&bytes));
                if says.as_deref() != Some(want.as_str()) {
                    return Err(goat_errors::coded("GB-074", format!("refused: {name} names {}, and SHA256SUMS lists {want} for {} (check: listed)", says.unwrap_or_else(|| "no digest".into()), name.trim_end_matches(".sha256"))).into());
                }
                format!("names {want}, as SHA256SUMS lists it")
            }
            (None, None) => return Err(goat_errors::coded("GB-074", format!("refused: SHA256SUMS does not list {name} (check: listed)")).into()),
        };
        rows.push(ok(name, detail));
    }
    Ok(Report::new("update-verify-release", rows, EXIT_REFUSED))
}

/// The signature over SHA256SUMS did not verify, or could not be checked.
fn sig_refused(why: String) -> anyhow::Error {
    goat_errors::coded(SIGNATURE_REFUSED, format!("{why}; refusing to install, and the installed binary is untouched")).into()
}

/// The catalog code for a signature that does not verify.
const SIGNATURE_REFUSED: &str = "GB-261";

/// The repository whose GitHub workflow built this binary (`GITHUB_REPOSITORY`
/// at build time): the binary's own release provenance. None for a local build.
const BUILT_BY: Option<&str> = option_env!("GITHUB_REPOSITORY");

/// Who must have signed SHA256SUMS: the deployment's configuration, else the
/// binary's own provenance. Neither: None, and the update refuses.
fn release_signer(configured: Option<String>, built_by: Option<&str>) -> Option<String> {
    configured
        .filter(|s| !s.trim().is_empty())
        .or_else(|| built_by.filter(|s| !s.trim().is_empty()).map(str::to_string))
}

/// The binary, bounded by a stall rather than a total. `connect` bounds only
/// the reach to the surface; the transfer itself is stall-bounded, so a slow
/// mesh link that keeps sending is never cut off (goat #2134).
fn download(url: &str, connect: Duration) -> Result<Vec<u8>> {
    let stall = stall_timeout();
    tracing::debug!(%url, stall = ?stall, "update: downloading the binary");
    let resp = agent_c(connect, stall).get(url).call().map_err(|e| unread(format!("cannot download the binary: {}", describe(url, e))))?;
    let mut bin = Vec::new();
    resp.into_reader().take(MAX_BINARY).read_to_end(&mut bin).map_err(|e| {
        let why = if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) {
            format!("no bytes for {} s after {} received ({e})", stall.as_secs(), bin.len())
        } else {
            format!("{e} after {} bytes", bin.len())
        };
        unread(format!("the download of {url} stalled or broke: {why}; the installed binary is untouched"))
    })?;
    Ok(bin)
}

// ---- the remembered surface and the notice ----

#[derive(Serialize, Deserialize, Default, Debug)]
struct Cache {
    #[serde(default)]
    surface: String,
    #[serde(default)]
    site: Option<String>,
    #[serde(default)]
    version: String,
    #[serde(default)]
    checked_at_unix: u64,
    #[serde(default)]
    noticed_at_unix: u64,
}

#[cfg(test)]
thread_local! {
    /// A unit test's own cache file, so no test touches $HOME.
    static TEST_CACHE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn cache_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(p) = TEST_CACHE.with(|c| c.borrow().clone()) {
        return Some(p);
    }
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".goatbuilder/update-check.json"))
}

fn read_cache() -> Option<Cache> {
    serde_json::from_str(&std::fs::read_to_string(cache_path()?).ok()?).ok()
}

/// Best-effort: an unwritable home only means the next run fetches again.
fn write_cache(c: &Cache) {
    if let Some(p) = cache_path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::to_string_pretty(c).unwrap_or_default());
    }
}

fn remember(s: &Surface, version: &str) {
    let mut c = read_cache().unwrap_or_default();
    if c.surface != s.base {
        c.noticed_at_unix = 0;
    }
    c.surface = s.base.clone();
    if s.site.is_some() || s.from == URL_ENV {
        c.site = s.site.clone();
    }
    c.version = version.to_string();
    c.checked_at_unix = now();
    write_cache(&c);
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn suppressed() -> bool {
    std::env::var_os(SUPPRESS_ENV).is_some() || std::env::var_os("GOAT_NO_VERSION_CHECK").is_some()
}

/// The behind-version line, if one is due: a terminal, the table format, not
/// suppressed, a known surface, and no notice in the last day. Never fails.
pub fn notice(output: OutputFormat) {
    if suppressed() || !matches!(output, OutputFormat::Table) || !std::io::stderr().is_terminal() {
        return;
    }
    if let Some(line) = notice_line(now(), |base| latest(&agent(NOTICE_TIMEOUT), base, NOTICE_TIMEOUT).ok()) {
        eprintln!("{line}");
    }
}

/// The decision behind [`notice`], with the clock and the fetch passed in.
fn notice_line(t: u64, fetch: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut c = read_cache().unwrap_or_default();
    let base = url_override().map(|s| s.base).unwrap_or_else(|| c.surface.clone());
    if base.is_empty() {
        return None;
    }
    if c.surface != base || t.saturating_sub(c.checked_at_unix) >= DAY {
        match fetch(&base) {
            Some(v) => {
                if c.surface != base {
                    c.noticed_at_unix = 0;
                }
                c.surface = base.clone();
                c.version = v;
                c.checked_at_unix = t;
            }
            // Unreachable now: an older "behind" is still true, so the cached answer stands.
            None if c.surface == base => {}
            None => return None,
        }
    }
    let due = goat_selfupdate::is_outdated(current(), &c.version) && t.saturating_sub(c.noticed_at_unix) >= DAY;
    if due {
        c.noticed_at_unix = t;
    }
    write_cache(&c);
    due.then(|| {
        let site = c.site.as_deref().map(|s| format!(" --site {s}")).unwrap_or_default();
        format!(
            "goatbuilder {} is behind {}, which {base}/ serves. Update: goatbuilder update{site}   (silence: {SUPPRESS_ENV}=1)",
            current(),
            c.version
        )
    })
}

/// doctor's row: this binary against the served release, read fresh. None when
/// no surface is known or it cannot be read (doctor's other rows cover reach).
pub fn doctor_row(repo: &Path, site: Option<&str>) -> Option<Check> {
    if suppressed() {
        return None;
    }
    let s = match site {
        Some(site) => url_override().or_else(|| from_site(repo, site).ok()),
        None => resolve(repo, None).ok(),
    }?;
    let latest = latest(&agent(DOCTOR_TIMEOUT), &s.base, DOCTOR_TIMEOUT).ok()?;
    remember(&s, &latest);
    let running = current();
    let behind = goat_selfupdate::is_outdated(running, &latest);
    let site_flag = s.site.as_deref().map(|x| format!(" --site {x}")).unwrap_or_default();
    Some(Check {
        name: "goatbuilder version".into(),
        ok: !behind,
        detail: if behind {
            format!("running {running}, {}/ serves {latest}: a fixed defect may be back in this binary", s.base)
        } else {
            format!("running {running}, {}/ serves {latest}", s.base)
        },
        fix: behind.then(|| format!("goatbuilder update{site_flag}")),
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_signer_comes_from_config_then_provenance_never_a_name_in_the_code() {
        assert_eq!(super::release_signer(Some("org/a".into()), Some("org/b")).as_deref(), Some("org/a"));
        assert_eq!(super::release_signer(None, Some("org/b")).as_deref(), Some("org/b"));
        assert_eq!(super::release_signer(Some(" ".into()), None), None);
        assert_eq!(super::release_signer(None, None), None);
    }

    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn site_resolves_from_its_group_vars_either_spelling() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "ops/ansible/group_vars/zagros_dev_hosts/vars.yml", "site_id: zagros-dev   # was: zagros\nsite_dns_zone: netbird.zagros-dev.example\ngoat_get_goatbuilder_enabled: true\n");
        for name in ["zagros-dev", "zagros_dev", "zagros_dev_hosts"] {
            let s = from_site(t.path(), name).unwrap();
            assert_eq!(s.base, "https://get.zagros-dev.netbird.zagros-dev.example/goatbuilder", "{name}");
        }
    }

    #[test]
    fn a_site_that_does_not_serve_goatbuilder_is_refused_by_name() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "site_id: zagros\nsite_dns_zone: z.net\ngoat_get_goatbuilder_enabled: false\n");
        let e = from_site(t.path(), "zagros").unwrap_err();
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().unwrap().code, "GB-072");
        assert!(e.to_string().contains("does not serve goatbuilder"), "{e}");
        let e = from_site(t.path(), "nosuch").unwrap_err();
        assert!(e.to_string().contains("`nosuch` is not a goatnet site"), "{e}");
    }

    #[test]
    fn a_literal_hostname_override_and_subdir_are_honoured_and_a_template_is_not() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "ops/ansible/group_vars/a_hosts/vars.yml", "site_id: a\nsite_dns_zone: z.net\ngoat_get_goatbuilder_enabled: true\ngoat_get_hostname: dl.example.net\ngoat_get_goatbuilder_www_subdir: gb/\n");
        assert_eq!(from_site(t.path(), "a").unwrap().base, "https://dl.example.net/gb");
        write(t.path(), "ops/ansible/group_vars/b_hosts/vars.yml", "site_id: b\nsite_dns_zone: z.net\ngoat_get_goatbuilder_enabled: true\ngoat_get_hostname: \"get.{{ site_id }}.x\"\n");
        assert_eq!(from_site(t.path(), "b").unwrap().base, "https://get.b.z.net/goatbuilder");
    }

    #[test]
    fn the_notice_is_cached_shown_once_a_day_and_silent_without_a_surface() {
        let home = tempfile::tempdir().unwrap();
        TEST_CACHE.with(|c| *c.borrow_mut() = Some(home.path().join("update-check.json")));
        if url_override().is_some() {
            return; // the override names a surface; this test is about the remembered one
        }
        let behind = |_: &str| Some("999.0.0".to_string());
        let t0 = now();

        // No surface known: silent, and no fetch.
        assert_eq!(notice_line(t0, |_| panic!("no surface, no fetch")), None);

        // A remembered surface, behind: shown once, then not again the same day.
        remember(&Surface { base: "https://get.s.z/goatbuilder".into(), site: Some("s".into()), from: "--site s".into() }, current());
        let first = notice_line(t0 + DAY, behind).expect("behind after a day's refresh");
        assert!(first.contains("999.0.0") && first.contains("goatbuilder update --site s") && first.contains(SUPPRESS_ENV), "{first}");
        assert_eq!(notice_line(t0 + DAY + 60, |_| panic!("fresh cache, no fetch")), None);

        // The next day: shown again, from the cache even when the surface is unreachable.
        assert!(notice_line(t0 + 2 * DAY + 1, |_| None).is_some());

        // Current: never shown.
        remember(&Surface { base: "https://get.s.z/goatbuilder".into(), site: Some("s".into()), from: "--site s".into() }, current());
        assert_eq!(notice_line(t0 + 5 * DAY, |_| Some(current().to_string())), None);
    }
}
