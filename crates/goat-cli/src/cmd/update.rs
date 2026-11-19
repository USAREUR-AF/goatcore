// `goat update` — self-update the running binary from this goatnet's
// download surface (`get.<site>.<zone>`, docs/design/goat-cli-mesh-download.md).
//
// Closes the loop the version_check nag only *reports*: an operator who is
// told "you're behind" can now just run `goat update` instead of hunting for
// the install page. Per-goatnet by construction — it fetches from the SAME
// surface `version_check` resolves (get.<site> derived from the active
// profile's portal host), so `goat update` on an ZAGROS context updates from
// get.zagros, on nova from get.nova. Nothing is pinned to a particular goatnet or
// to any external host.
//
// Contract:
//   - Provenance before bytes: the surface's `<asset>.sha256` must agree with
//     the release's `SHA256SUMS`, and `SHA256SUMS` must carry a cosign
//     signature (`SHA256SUMS.cosign-bundle`) whose certificate identity is
//     exactly the release workflow on the release tag of the version being
//     installed: `https://github.com/<repo>/.github/workflows/_release-binary.yml
//     @refs/tags/goat-cli-v<version>`, for the repository that built this
//     binary (`GITHUB_REPOSITORY` at build time, or `GOAT_UPDATE_SIGNER`). A
//     run from a branch, a pull request or another workflow does not verify,
//     and neither does an older release offered under a newer version.json:
//     the version the surface advertises is the one the signature must name.
//   - No downgrade: the advertised version must be newer than the running one
//     (`--force` reinstalls the same version; `--allow-downgrade` is the only
//     way to an older one). The surface is a mirror: a checksum it
//     serves next to the binary proves only that the two came from the same
//     place, so it is never the only check. Without cosign on PATH the update
//     refuses unless `--allow-unsigned` says, for this one run, that the
//     surface's own checksum is enough.
//   - Bounded: every small file is capped in size and time; the binary is
//     capped at MAX_BINARY bytes, bounded by a stall (no bytes for
//     STALL_TIMEOUT) and by a total deadline (DOWNLOAD_DEADLINE, or
//     `GOAT_UPDATE_DEADLINE_SECS`). A surface that trickles or floods is
//     refused with the installed binary untouched.
//   - Verifies the downloaded binary against the checked digest before
//     touching the installed binary. A mismatch aborts with it untouched.
//   - Atomic replace: download to a temp file next to the running binary,
//     verify, then rename into place on the same filesystem. On Unix the
//     rename lands directly over `current_exe()` (the running process keeps
//     its open inode; the new binary is in place for the next invocation).
//     Windows locks a running `.exe` and refuses the overwrite, so there the
//     running exe is renamed to `<name>.old` first and the new one moved in
//     (rolled back on failure). No partial-write window either way.
//   - Idempotent: already-latest prints one line and exits 0 (unless
//     `--force` re-installs the same version, e.g. to repair a corrupt file).
//   - Honest + fail-closed: any resolve/fetch/verify/replace error is a
//     non-zero exit with a specific message and the old binary intact.

use crate::version_check::{current, get_base_url};
use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::path::PathBuf;
use std::time::Duration;

// version.json and the .sha256 are a few bytes: each request is capped whole.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
// The binary is ~40 MB and the mesh path to get.<site> can run at a few hundred
// KB/s (zagros-dev: 38.7 MB in 117 s), so a whole-request cap fails a
// healthy download. It is bounded instead by a stall: no bytes for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
// ...and by a total: 30 minutes is ~55 KB/s for 100 MB, well under the slowest
// healthy mesh path above. GOAT_UPDATE_DEADLINE_SECS overrides it.
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(30 * 60);
const DEADLINE_ENV: &str = "GOAT_UPDATE_DEADLINE_SECS";
// The largest binary this will hold in memory (the releases are ~40-100 MB).
const MAX_BINARY: u64 = 512 * 1024 * 1024;
// version.json, the checksums and the signature bundle are a few KB each.
const MAX_SMALL: u64 = 1024 * 1024;
// cosign verify-blob may refresh its Sigstore trust root; bound it too.
const COSIGN_TIMEOUT: Duration = Duration::from_secs(120);
// Who must have signed SHA256SUMS, when set: `<owner>/<repo>` (a private fork's
// release repository, or a local build that has no provenance of its own).
const SIGNER_ENV: &str = "GOAT_UPDATE_SIGNER";
// The repository whose GitHub workflow built this binary (`GITHUB_REPOSITORY`
// at build time): its own release provenance. None for a local build.
const BUILT_BY: Option<&str> = option_env!("GITHUB_REPOSITORY");
// The workflow that signs a goat release (goat-release.yml calls it) and the
// release tag's prefix; together with the version they are the one
// certificate identity a release signature may carry.
const RELEASE_WORKFLOW: &str = ".github/workflows/_release-binary.yml";
const TAG_PREFIX: &str = "goat-cli-v";
// A pinned Sigstore trust root (a TrustedRoot JSON file kept on THIS machine,
// never one served by the surface). When set, cosign verifies against it with
// --offline and fetches nothing: the path for a goatnet with no internet.
const TRUSTED_ROOT_ENV: &str = "GOAT_SIGSTORE_TRUSTED_ROOT";
// The release's signatures over SHA256SUMS: the Sigstore bundle format
// (verifiable offline against a pinned root) and the legacy cosign bundle
// every release carries.
const SIGSTORE_BUNDLE: &str = "SHA256SUMS.sigstore.json";
const LEGACY_BUNDLE: &str = "SHA256SUMS.cosign-bundle";

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Re-install even if already at the advertised-latest version (repairs a
    /// corrupt binary; re-verifies the checksum either way).
    #[arg(long)]
    pub force: bool,

    /// Resolve + report what would happen, but don't touch the installed
    /// binary.
    #[arg(long)]
    pub dry_run: bool,

    /// Install on the surface's own checksum alone when cosign is not on
    /// PATH. That checksum comes from the same server as the binary, so it
    /// proves nothing about who built it; without this flag the update
    /// refuses unless the release signature over SHA256SUMS verifies.
    #[arg(long)]
    pub allow_unsigned: bool,

    /// Install the advertised version even when it is OLDER than the running
    /// one. Without it a downgrade is refused: a surface that offers an older
    /// (genuine, signed) release could otherwise roll back a fix.
    #[arg(long)]
    pub allow_downgrade: bool,
}

/// Platform asset name the goat-get surface serves (matches the release asset
/// names + the goat-get www dir: goat-<os>-<arch>[.exe]); the one mapping
/// goatbuilder uses too (crates/goat-selfupdate).
fn asset_name() -> Result<String> {
    goat_selfupdate::asset_name("goat").ok_or_else(|| {
        anyhow!(
            "no goat binary published for this platform ({}/{}); \
             install manually from the goatnet's get.<site> page",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })
}

pub async fn run(ctx: &super::Ctx, args: UpdateArgs) -> Result<()> {
    let base = get_base_url(&ctx.profile).ok_or_else(|| {
        anyhow!(
            "cannot resolve this context's download surface (get.<site>). \
             Set GOAT_GET_ENDPOINT or run from a context whose profile has a \
             portal.<site>.… endpoint."
        )
    })?;
    let base = base.trim_end_matches('/');
    let running = current();

    // 1. What does the surface advertise?
    let client = reqwest::Client::builder()
        .connect_timeout(FETCH_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .build()
        .context("build http client")?;
    let vj = fetch_small(&client, &format!("{base}/version.json"))
        .await
        .context("read version.json (are you on this goatnet's mesh?)")?;
    let vjson: serde_json::Value = serde_json::from_str(&vj).context("parse version.json")?;
    let latest = vjson
        .get("version")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("version.json has no version field"))?;

    println!("running: {running}   latest: {latest}   surface: {base}/");

    // The advertised version names the release tag the signature must carry,
    // so it is held to the tag's own shape: MAJOR.MINOR.PATCH, digits only.
    let Some(want_v) = strict_semver(latest) else {
        bail!(
            "version.json advertises {latest:?}, which is not a release version (MAJOR.MINOR.PATCH) \
             — refusing to install. The installed binary is untouched."
        );
    };
    let running_v = goat_selfupdate::parse_semver(running);
    if running_v == Some(want_v) && !args.force {
        println!("already up to date.");
        return Ok(());
    }
    if running_v.is_some_and(|r| want_v < r) && !args.allow_downgrade {
        bail!(
            "{base}/ advertises {latest}, OLDER than the running {running} — refusing to downgrade. \
             A surface that serves an older release can roll back a fix; if you mean to go back, \
             re-run with --allow-downgrade. The installed binary is untouched."
        );
    }
    // 2. Who vouches for the binary (a dry run checks this too), then the binary.
    let asset = asset_name()?;
    let prov = Provenance {
        signer: std::env::var(SIGNER_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                BUILT_BY
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
            }),
        cosign: on_path("cosign").or_else(|| on_path("cosign.exe")),
        trusted_root: match std::env::var_os(TRUSTED_ROOT_ENV).filter(|v| !v.is_empty()) {
            None => None,
            Some(p) => Some(pinned_root(PathBuf::from(p))?),
        },
        allow_unsigned: args.allow_unsigned,
        version: latest.to_string(),
    };
    if args.dry_run {
        let (_, vouched) = vouch(&client, base, &asset, &prov).await?;
        println!("{vouched}");
        println!(
            "dry-run: would install {latest} for {} from {base}/{asset} (not downloaded; the \
             binary itself is checked against this digest on a real run)",
            std::env::consts::OS
        );
        return Ok(());
    }
    let limits = Limits {
        max_binary: MAX_BINARY,
        deadline: std::env::var(DEADLINE_ENV)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .filter(|n| *n > 0)
            .map(Duration::from_secs)
            .unwrap_or(DOWNLOAD_DEADLINE),
    };
    let (bin, vouched) = fetch_verified(&client, base, &asset, &prov, &limits).await?;
    println!("{vouched}");

    // 3. Replace the installed binary next to the running one, atomically
    // (crates/goat-selfupdate): Unix renames over the running file; Windows,
    // which locks a running `.exe`, moves it to `<name>.old` first and puts it
    // back if the second move fails. The `.old` stays locked until this process
    // exits; the next `goat update` clears it.
    let exe = std::env::current_exe().context("resolve current executable path")?;
    let left_old = goat_selfupdate::install(&exe, &bin, &format!("goat-update-{latest}"))
        .map_err(|e| anyhow!("{e} (try the same shell that installed goat)"))?;

    println!("updated {running} → {latest} at {}", exe.display());
    if let Some(old) = left_old {
        println!(
            "(previous binary left as {}; removed on your next `goat update`)",
            old.display()
        );
    }
    println!("run `goat --version` to confirm.");
    Ok(())
}

/// Who must vouch for a download, and how.
pub(crate) struct Provenance {
    /// `<owner>/<repo>` whose release workflow must have signed SHA256SUMS.
    pub signer: Option<String>,
    /// The cosign binary, when one is installed.
    pub cosign: Option<PathBuf>,
    /// `--allow-unsigned`: no cosign is acceptable for this run.
    pub allow_unsigned: bool,
    /// The version being installed: the release tag the signature must name.
    pub version: String,
    /// A pinned trust root on this machine: verify offline against it.
    pub trusted_root: Option<PathBuf>,
}

pub(crate) struct Limits {
    pub max_binary: u64,
    pub deadline: Duration,
}

/// The binary for `asset` from `base`, and one line saying what vouched for
/// it. Nothing large is downloaded until the checksum, SHA256SUMS and the
/// signature have been read and checked.
pub(crate) async fn fetch_verified(
    client: &reqwest::Client,
    base: &str,
    asset: &str,
    prov: &Provenance,
    limits: &Limits,
) -> Result<(Vec<u8>, String)> {
    let bin_url = format!("{base}/{asset}");
    let (want, vouched) = vouch(client, base, asset, prov).await?;

    println!("downloading {bin_url} …");
    let resp = client
        .get(&bin_url)
        .send()
        .await
        .with_context(|| format!("download {bin_url}"))?
        .error_for_status()
        .context("binary request")?;
    let bin = match tokio::time::timeout(
        limits.deadline,
        read_capped(resp, limits.max_binary, &bin_url),
    )
    .await
    {
        Ok(r) => r?,
        Err(_) => bail!(
            "download of {bin_url} did not finish within {} s — refusing to install \
             (set {DEADLINE_ENV} for a slower link). The installed binary is untouched.",
            limits.deadline.as_secs()
        ),
    };

    // Verify BEFORE touching the installed binary.
    if let Err(m) = goat_selfupdate::verify(&bin, &want) {
        bail!(
            "checksum mismatch — refusing to install.\n  expected {}\n  got      {}\n\
             The download surface may be mid-deploy; retry shortly.",
            m.want,
            m.got
        );
    }
    Ok((bin, vouched))
}

/// What vouches for `asset` on `base`: its `.sha256`, which must agree with
/// SHA256SUMS, whose release signature must verify. Returns the digest the
/// binary must match and the line to print. Downloads nothing large.
async fn vouch(
    client: &reqwest::Client,
    base: &str,
    asset: &str,
    prov: &Provenance,
) -> Result<(String, String)> {
    let sha_url = format!("{base}/{asset}.sha256");
    let sha_txt = fetch_small(client, &sha_url).await?;
    // Checksum files are "<hex>  <name>" — take the first field.
    let want = goat_selfupdate::parse_checksum(&sha_txt)
        .ok_or_else(|| anyhow!("malformed checksum from {sha_url}"))?;
    let sums_url = format!("{base}/SHA256SUMS");
    let sums = fetch_small(client, &sums_url).await?;
    match listed_digest(&sums, asset) {
        Some(d) if d == want => {}
        Some(d) => bail!(
            "checksum disagreement — refusing to install: {asset}.sha256 says {want}, \
             SHA256SUMS says {d}. The installed binary is untouched."
        ),
        None => bail!(
            "SHA256SUMS at {base}/ does not list {asset} — refusing to install. \
             The installed binary is untouched."
        ),
    }
    let vouched = signature(client, base, &sums, prov).await?;
    Ok((want, vouched))
}

/// The digest SHA256SUMS lists for `asset`, lowercased.
fn listed_digest(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let (d, n) = (f.next()?, f.next()?);
        (n.trim_start_matches('*') == asset).then(|| d.to_lowercase())
    })
}

/// The signature over SHA256SUMS, checked with cosign against the release
/// workflow identity of `prov.signer`. Returns the line to print.
async fn signature(
    client: &reqwest::Client,
    base: &str,
    sums: &str,
    prov: &Provenance,
) -> Result<String> {
    let Some(cosign) = &prov.cosign else {
        if prov.allow_unsigned {
            return Ok(format!(
                "WARNING: signature NOT checked (--allow-unsigned, cosign not on PATH): \
                 only {base}/'s own checksum vouches for this binary"
            ));
        }
        bail!(
            "cannot check the release signature: cosign is not on PATH — refusing to install.\n\
             The checksum on {base}/ comes from the same server as the binary, so it cannot \
             show who built it. Install cosign (https://docs.sigstore.dev/cosign/system_config/installation/) \
             and re-run, or re-run with --allow-unsigned to trust this surface's checksum alone. \
             The installed binary is untouched."
        );
    };
    let signer = prov
        .signer
        .as_deref()
        .map(str::trim)
        .filter(|s| valid_repo(s))
        .ok_or_else(|| {
            anyhow!(
                "no release signer is known — refusing to install: this binary was not built by a \
                 release workflow, so it names none itself. Set {SIGNER_ENV}=<owner>/<repo> to the \
                 repository whose release workflow signs SHA256SUMS. The installed binary is untouched."
            )
        })?;
    if strict_semver(&prov.version).is_none() {
        bail!(
            "{:?} is not a release version — refusing to install",
            prov.version
        );
    }
    let identity = release_identity(signer, &prov.version);
    let pinned = prov.trusted_root.as_deref();

    // The Sigstore bundle format first (SHA256SUMS.sigstore.json, what cosign
    // can verify offline), the legacy cosign bundle otherwise. A surface that
    // withholds the first gains nothing online: the legacy bundle is checked
    // against the same exact identity. Offline (a pinned root) only the first
    // can be checked, so there it is required.
    let modern = fetch_small(client, &format!("{base}/{SIGSTORE_BUNDLE}"))
        .await
        .ok();
    let mut note = "";
    if let Some(bundle) = &modern {
        let out = run_cosign(cosign, sums, bundle, &identity, true, pinned).await;
        let too_old = matches!(&out, Ok(o) if !o.status.success() && cosign_lacks_bundle_format(&String::from_utf8_lossy(&o.stderr)));
        if !too_old || pinned.is_some() {
            return verdict(out, base, signer, &identity, prov, cosign, SIGSTORE_BUNDLE);
        }
        // This cosign cannot read the Sigstore bundle format; the legacy
        // bundle, checked the same way online, is as strong.
        note = "; this cosign does not read the Sigstore bundle format";
    } else if let Some(root) = pinned {
        bail!(
            "offline verification needs a goat-cli release signed in the Sigstore bundle format \
             (one that publishes {SIGSTORE_BUNDLE}): {base}/ serves no {SIGSTORE_BUNDLE} \
             for {TAG_PREFIX}{}, only the legacy {LEGACY_BUNDLE}, which cosign cannot check against \
             the pinned trust root {} — refusing to install. The installed binary is untouched.",
            prov.version,
            root.display()
        );
    }
    let bundle = fetch_small(client, &format!("{base}/{LEGACY_BUNDLE}"))
        .await
        .context("cannot read the release signature — refusing to install; the installed binary is untouched")?;
    let out = run_cosign(cosign, sums, &bundle, &identity, false, None).await;
    verdict(out, base, signer, &identity, prov, cosign, LEGACY_BUNDLE).map(|v| format!("{v}{note}"))
}

/// `cosign verify-blob` of `sums` with `bundle` against exactly `identity`,
/// bounded by COSIGN_TIMEOUT. `modern`: the bundle is in the Sigstore bundle
/// format. `pinned`: verify offline against that trust root (modern only).
async fn run_cosign(
    cosign: &std::path::Path,
    sums: &str,
    bundle: &str,
    identity: &str,
    modern: bool,
    pinned: Option<&std::path::Path>,
) -> std::io::Result<std::process::Output> {
    let dir = tempdir_unique().map_err(std::io::Error::other)?;
    let (sums_path, bundle_path) = (dir.join("SHA256SUMS"), dir.join("bundle.json"));
    let out = async {
        std::fs::write(&sums_path, sums)?;
        std::fs::write(&bundle_path, bundle)?;
        let mut c = tokio::process::Command::new(cosign);
        c.arg("verify-blob");
        if modern {
            c.arg("--new-bundle-format");
        }
        c.arg("--bundle").arg(&bundle_path).args([
            "--certificate-identity",
            identity,
            "--certificate-oidc-issuer",
            "https://token.actions.githubusercontent.com",
        ]);
        // One argument, `--trusted-root=<path>`: a path that looks like a flag
        // can never be read as one.
        if let (true, Some(r)) = (modern, pinned) {
            let mut a = std::ffi::OsString::from("--trusted-root=");
            a.push(r.as_os_str());
            c.arg(a).arg("--offline");
        }
        let child = c.arg(&sums_path).kill_on_drop(true).output();
        match tokio::time::timeout(COSIGN_TIMEOUT, child).await {
            Ok(r) => r,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "cosign verify-blob timed out",
            )),
        }
    }
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// What one cosign run means for the update: the line to print, or why not.
fn verdict(
    out: std::io::Result<std::process::Output>,
    base: &str,
    signer: &str,
    identity: &str,
    prov: &Provenance,
    cosign: &std::path::Path,
    bundle_name: &str,
) -> Result<String> {
    let o = match out {
        Ok(o) => o,
        Err(e) => bail!(
            "release signature not checked — refusing to install: cannot run {}: {e}. \
             The installed binary is untouched.",
            cosign.display()
        ),
    };
    let err = String::from_utf8_lossy(&o.stderr);
    let err = err.trim();
    if o.status.success() {
        return Ok(format!(
            "SHA256SUMS signed by the {signer} release workflow for {TAG_PREFIX}{} (cosign verify-blob, {bundle_name}{})",
            prov.version,
            prov.trusted_root
                .as_ref()
                .map(|r| format!(", offline against {}", r.display()))
                .unwrap_or_default()
        ));
    }
    if let Some(root) = &prov.trusted_root {
        if cosign_lacks_bundle_format(err) {
            bail!(
                "cannot check the release signature offline: this cosign ({}) does not support the \
                 Sigstore bundle format with a pinned trust root — refusing to install.\n  cosign said: {err}\n\
                 Install a current cosign, then re-run. The installed binary is untouched.",
                cosign.display()
            );
        }
        if pinned_root_refused(err) {
            bail!(
                "cannot check the release signature: cosign could not use the pinned trust root {} \
                 ({TRUSTED_ROOT_ENV}) — refusing to install.\n  cosign said: {err}\n\
                 The file is there but cosign did not accept it as a Sigstore TrustedRoot: replace it \
                 with a current trusted_root.json from Sigstore's TUF repository, then re-run. Until \
                 the signature can be checked, goat will not install this update. The installed \
                 binary is untouched.",
                root.display()
            );
        }
    } else if trust_root_unavailable(err) {
        bail!(
            "cannot check the release signature: cosign could not load the Sigstore trust root \
             (its TUF repository is unreachable from here) — refusing to install.\n  cosign said: {err}\n\
             On a host without internet access, either set {TRUSTED_ROOT_ENV} to a pinned Sigstore \
             TrustedRoot JSON kept on this machine (cosign then verifies offline; it needs a goat-cli \
             release signed in the Sigstore bundle format, one that publishes {SIGSTORE_BUNDLE}), \
             or seed cosign's trust root first: run `cosign initialize` on this account while it can \
             reach tuf-repo-cdn.sigstore.dev, or copy ~/.sigstore/root from a host that has, then \
             re-run. Until the signature can be checked, goat will not install this update. The \
             installed binary is untouched."
        );
    }
    bail!(
        "release signature refused — refusing to install: cosign verify-blob of {base}/SHA256SUMS \
         ({bundle_name}) against {identity}: {err}. The installed binary is untouched."
    )
}

/// cosign does not know the Sigstore bundle format or the pinned-root flag:
/// an old cosign (`unknown flag`), or cosign's own refusal to combine a
/// trusted root with a legacy bundle. Both are about cosign, not the release.
fn cosign_lacks_bundle_format(stderr: &str) -> bool {
    let low = stderr.to_ascii_lowercase();
    if CERT_TEXT.iter().any(|m| low.contains(m)) {
        return false;
    }
    low.lines().any(|l| {
        let l = strip_cosign_prefix(l.trim());
        l.starts_with("unknown flag: --new-bundle-format")
            || l.starts_with("unknown flag: --trusted-root")
            || l.starts_with("--trusted-root only supported with --new-bundle-format")
    })
}

/// The text of one cosign stderr line after its "Error: " or
/// "…error during command execution: " lead-in.
fn strip_cosign_prefix(l: &str) -> &str {
    let l = l
        .rsplit_once("error during command execution: ")
        .map_or(l, |(_, r)| r);
    l.strip_prefix("error: ").unwrap_or(l)
}

/// The one certificate identity a goat release signature may carry: the
/// release workflow, run for the release tag of `version`. A run from a branch
/// (`@refs/heads/…`), a pull request (`@refs/pull/…`), another workflow, or
/// another version's tag is a different identity and does not verify.
fn release_identity(signer: &str, version: &str) -> String {
    format!("https://github.com/{signer}/{RELEASE_WORKFLOW}@refs/tags/{TAG_PREFIX}{version}")
}

/// MAJOR.MINOR.PATCH, digits only (a release tag's semver), or None.
fn strict_semver(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.split('.');
    let mut n = || -> Option<u64> {
        let p = it.next()?;
        (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())?
    };
    let t = (n()?, n()?, n()?);
    it.next().is_none().then_some(t)
}

/// The pinned trust root named by GOAT_SIGSTORE_TRUSTED_ROOT, checked before
/// cosign sees it: a readable, non-empty JSON object, or the update refuses
/// and says which of those it is not.
fn pinned_root(p: PathBuf) -> Result<PathBuf> {
    let why = match std::fs::metadata(&p) {
        Err(e) => Some(format!("cannot be read ({e})")),
        Ok(m) if !m.is_file() => Some("is not a file".to_string()),
        Ok(m) if m.len() > MAX_SMALL * 16 => {
            Some(format!("is {} bytes, too large for a trust root", m.len()))
        }
        Ok(_) => match std::fs::read(&p) {
            Err(e) => Some(format!("cannot be read ({e})")),
            Ok(b) if b.iter().all(u8::is_ascii_whitespace) => Some("is empty".to_string()),
            Ok(b) => match serde_json::from_slice::<serde_json::Value>(&b) {
                Ok(serde_json::Value::Object(_)) => None,
                _ => Some("is not a JSON object (a Sigstore TrustedRoot is one)".to_string()),
            },
        },
    };
    match why {
        None => Ok(p),
        Some(w) => bail!(
            "the pinned trust root {} ({TRUSTED_ROOT_ENV}) {w} — refusing to install. Put a \
             current Sigstore trusted_root.json there, then re-run. The installed binary is untouched.",
            p.display()
        ),
    }
}

/// With a pinned root, cosign's failure was that root (not the signature):
/// the same veto on certificate text as [`trust_root_unavailable`], then any
/// line about the trusted root.
fn pinned_root_refused(stderr: &str) -> bool {
    let low = stderr.to_ascii_lowercase();
    if CERT_TEXT.iter().any(|m| low.contains(m)) {
        return false;
    }
    trust_root_unavailable(stderr)
        || ["trusted root", "trusted-root", "trustedroot"]
            .iter()
            .any(|m| low.contains(m))
}

/// Words that appear in cosign's errors that can quote certificate content.
const CERT_TEXT: &[&str] = &["identit", "subject", "certificate", "signature", "issuer"];

/// Where cosign reports a failure to load the Sigstore trust root: the start
/// of its own error text (after "Error: " or "…error during command
/// execution: "). These come from cosign's TUF client, before any
/// certificate is read.
const TRUST_ROOT_PREFIXES: &[&str] = &[
    "getting trusted root",
    "fetching trusted root",
    "loading trusted root",
    "unable to fetch trusted root",
    "initializing tuf",
    "error initializing tuf",
    "updating local metadata and targets",
    "error updating to tuf remote mirror",
];

/// cosign's failure was loading the Sigstore trust root (TUF), not the
/// signature itself. Decided only on cosign's own TUF error lines, never on
/// text that can carry certificate content: a certificate subject is chosen by
/// whoever got it issued (a repository named `evil/tuf`), so any mention of an
/// identity, certificate or signature means "the signature was refused".
fn trust_root_unavailable(stderr: &str) -> bool {
    let low = stderr.to_ascii_lowercase();
    if CERT_TEXT.iter().any(|m| low.contains(m)) {
        return false;
    }
    low.lines().any(|l| {
        let l = strip_cosign_prefix(l.trim());
        TRUST_ROOT_PREFIXES.iter().any(|p| l.starts_with(p)) || tuf_download_failed(l)
    })
}

/// cosign v3's TUF client: "failed to download 13.root.json" (and the same for
/// the other TUF metadata files), at the start of an error line.
fn tuf_download_failed(l: &str) -> bool {
    let Some(rest) = l.strip_prefix("failed to download ") else {
        return false;
    };
    let file = rest.split([' ', ',']).next().unwrap_or("");
    let file = file
        .trim_end_matches(':')
        .trim_matches(|c| c == '"' || c == '\'');
    let name = file.rsplit('/').next().unwrap_or(file);
    let name = name
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches('.');
    matches!(
        name,
        "root.json" | "timestamp.json" | "snapshot.json" | "targets.json" | "trusted_root.json"
    )
}

/// `<owner>/<repo>` in GitHub's own character set: nothing a regex or a
/// shell would read as more than a name.
fn valid_repo(s: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    matches!(s.split_once('/'), Some((o, r)) if ok(o) && ok(r) && !r.contains('/'))
}

fn tempdir_unique() -> Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("goat-update-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

fn on_path(tool: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(tool))
        .find(|p| p.is_file())
}

/// A small file (version.json, a checksum, the signature bundle): capped
/// whole in time and in size.
async fn fetch_small(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("request {url}"))?;
    let body = read_capped(resp, MAX_SMALL, url).await?;
    String::from_utf8(body).with_context(|| format!("{url} is not text"))
}

/// The body, refused as soon as it would pass `max` bytes: by its declared
/// length before reading, else while reading.
async fn read_capped(mut resp: reqwest::Response, max: u64, url: &str) -> Result<Vec<u8>> {
    if let Some(n) = resp.content_length() {
        if n > max {
            bail!("{url} is {n} bytes, larger than the {max}-byte limit — refusing to read it");
        }
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .with_context(|| format!("read {url} after {} bytes", out.len()))?
    {
        if out.len() as u64 + chunk.len() as u64 > max {
            bail!("{url} is larger than the {max}-byte limit — refusing to read more");
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A body with no declared length is still refused at the cap, while it
    /// is being read (tests/update_provenance.rs covers a declared length).
    #[tokio::test]
    async fn an_undeclared_length_is_capped_while_reading() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await;
            for _ in 0..64 {
                if s.write_all(b"100\r\n").await.is_err() {
                    return;
                }
                let _ = s.write_all(&[b'z'; 256]).await;
                let _ = s.write_all(b"\r\n").await;
            }
            let _ = s.write_all(b"0\r\n\r\n").await;
        });
        let url = format!("http://{addr}/goat");
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        assert_eq!(resp.content_length(), None);
        let e = read_capped(resp, 4096, &url).await.unwrap_err().to_string();
        assert!(e.contains("larger than the 4096-byte limit"), "{e}");
    }

    #[test]
    fn sha256sums_is_read_by_name() {
        let d = "a".repeat(64);
        let sums = format!(
            "{}  goat-linux-arm64\n{d}  goat-linux-amd64\n",
            "b".repeat(64)
        );
        assert_eq!(listed_digest(&sums, "goat-linux-amd64"), Some(d.clone()));
        assert_eq!(
            listed_digest(
                &format!("{}  *goat-linux-amd64\n", d.to_uppercase()),
                "goat-linux-amd64"
            ),
            Some(d)
        );
        assert_eq!(listed_digest(&sums, "goat-darwin-arm64"), None);
    }

    #[test]
    fn the_identity_is_the_release_workflow_on_the_versions_tag() {
        assert_eq!(
            release_identity("USAREUR-AF/goatcore", "0.5.16"),
            "https://github.com/USAREUR-AF/goatcore/.github/workflows/_release-binary.yml@refs/tags/goat-cli-v0.5.16"
        );
    }

    #[test]
    fn a_release_version_is_three_numbers_and_nothing_else() {
        assert_eq!(strict_semver("0.5.16"), Some((0, 5, 16)));
        assert_eq!(strict_semver("10.0.0"), Some((10, 0, 0)));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "1.2.3-rc1",
            "1.2.3+b",
            "1..3",
            "1.2.x",
            " 1.2.3",
            "1.2.3/..",
            "1234567890.0.0",
        ] {
            assert_eq!(strict_semver(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_trust_root_failure_is_told_apart_from_a_bad_signature() {
        // cosign's own TUF failure lines.
        for t in [
            "Error: getting trusted root: updating local metadata and targets: error updating to TUF remote mirror: dial tcp: lookup tuf-repo-cdn.sigstore.dev: no such host",
            "main.go:74: error during command execution: initializing tuf: updating local metadata and targets: i/o timeout",
            "Error: unable to fetch trusted root: Get \"https://tuf-repo-cdn.sigstore.dev/targets.json\": dial tcp 1.2.3.4:443: connect: network is unreachable",
        ] {
            assert!(trust_root_unavailable(t), "{t}");
        }
        // A refused signature, including one whose certificate subject was
        // chosen to look like a TUF or network failure (GHSA-32cp review R3).
        for t in [
            "Error: none of the expected identities matched what was in the certificate, got subjects [https://github.com/evil/tuf/.github/workflows/x.yml@refs/heads/getting trusted root]",
            "Error: none of the expected identities matched what was in the certificate, got subjects [https://github.com/evil/tuf-repo-cdn.sigstore.dev/.github/workflows/dial-tcp.yml@refs/heads/main]",
            "main.go:74: error during command execution: none of the expected identities matched what was in the certificate, got subjects [https://github.com/e/tuf/.github/workflows/initializing tuf.yml@x]",
            "Error: invalid signature when validating ASN.1 encoded signature",
            // A bare network line that is not cosign's TUF error: not enough.
            "dial tcp: lookup tuf-repo-cdn.sigstore.dev: no such host",
            "",
        ] {
            assert!(!trust_root_unavailable(t), "{t}");
        }
    }

    #[test]
    fn cosign_v3_wording_is_classified() {
        // Seen with cosign v3.0.6.
        assert!(trust_root_unavailable(
            "Error: failed to download 13.root.json"
        ));
        assert!(trust_root_unavailable("main.go:69: error during command execution: failed to download 13.root.json: Get \"https://tuf-repo-cdn.sigstore.dev/13.root.json\": dial tcp: i/o timeout"));
        assert!(trust_root_unavailable(
            "Error: failed to download timestamp.json, retrying"
        ));
        assert!(!trust_root_unavailable(
            "Error: failed to download the bundle"
        ));
        assert!(!trust_root_unavailable(
            "Error: failed to download 13.root.json: x509: certificate signed by unknown authority"
        ));
        let legacy = "Error: --trusted-root only supported with --new-bundle-format";
        assert!(cosign_lacks_bundle_format(legacy));
        assert!(cosign_lacks_bundle_format(
            "Error: unknown flag: --new-bundle-format"
        ));
        assert!(!trust_root_unavailable(legacy));
        // An identity error is never "cosign is too old", whatever the subject says.
        assert!(!cosign_lacks_bundle_format("Error: none of the expected identities matched what was in the certificate, got subjects [https://github.com/e/x/.github/workflows/unknown flag: --new-bundle-format]"));
    }

    #[test]
    fn a_signer_is_a_repository_name_and_nothing_more() {
        assert!(valid_repo("USAREUR-AF/goatcore"));
        assert!(valid_repo("some_org/goat.core"));
        for bad in [
            "", "goatcore", "a/b/c", "a/", "/b", "a/b.*", "a/(b)", "a /b", "a/b\n",
        ] {
            assert!(!valid_repo(bad), "{bad:?}");
        }
    }
}
