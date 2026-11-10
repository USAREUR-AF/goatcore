// `goat names` — self-claims tooling (ADR 1082 Amendment 2, design §7.3).
//
// publish-claim: sign "I am <name> at <ip>, as of now" with THIS
// context's leaf key and publish it to `ops/names/claim/<anchor>` on the
// fabric (consume-while-healthy). collect: the passive collector side —
// subscribe `ops/names/claim/**` and maintain a claims.json the goat-get
// tier serves beside the snapshot (serve-while-broken). The collector is
// untrusted transport by design: it gathers syntactically-valid envelopes;
// every consumer re-verifies signatures + snapshot bindings at read.

use crate::names::{ClaimArtifact, ClaimEnvelope, CLAIM_FORMAT};
use crate::profile::Profile;
use clap::{Args, Subcommand};
use p256::ecdsa::signature::Signer as _;

#[derive(Subcommand, Debug)]
pub enum NamesCmd {
    /// Sign and publish this device's own name→IP claim to the fabric.
    PublishClaim(PublishClaimArgs),
    /// Collect published claims into a claims.json for the get tier.
    Collect(CollectArgs),
    /// Resolve a mesh name through the fallback resolver and print the
    /// source (live / self-claim / snapshot / observed) — the diagnostic
    /// for the self-claims path.
    Resolve(ResolveArgs),
}

#[derive(Args, Debug)]
pub struct ResolveArgs {
    /// The mesh hostname to resolve.
    pub name: String,
    /// Port to attach (cosmetic — resolution is name-only).
    #[arg(long, default_value_t = 443)]
    pub port: u16,
}

#[derive(Args, Debug)]
pub struct PublishClaimArgs {
    /// Claimed name. Default: <netbird peer name>.<snapshot zone> — the
    /// zone-shaped rendering of this device's mesh peer name.
    #[arg(long)]
    pub name: Option<String>,
    /// Claimed IP. Default: this device's netbird mesh IP.
    #[arg(long)]
    pub ip: Option<String>,
    /// Claim TTL in seconds (claims exist for fast-moving IPs — keep short).
    #[arg(long, default_value_t = 3600)]
    pub ttl: u64,
    /// Also write the envelope JSON to this file (manual courier / tests).
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
    /// Build + print + (optionally) write the envelope without publishing.
    #[arg(long)]
    pub no_publish: bool,
}

#[derive(Args, Debug)]
pub struct CollectArgs {
    /// Output claims.json path (written atomically on every update).
    #[arg(long)]
    pub out: std::path::PathBuf,
    /// Listen window in seconds; 0 = follow until interrupted.
    #[arg(long, default_value_t = 30)]
    pub listen: u64,
}

pub async fn run(ctx: &super::Ctx, cmd: NamesCmd) -> anyhow::Result<()> {
    match cmd {
        NamesCmd::PublishClaim(args) => publish_claim(&ctx.profile, args).await,
        NamesCmd::Collect(args) => collect(&ctx.profile, args).await,
        NamesCmd::Resolve(args) => resolve(&ctx.profile, args).await,
    }
}

async fn resolve(profile: &Profile, args: ResolveArgs) -> anyhow::Result<()> {
    use crate::names::NameSource;
    match crate::names::resolve_host_port(profile, &args.name, args.port).await {
        Ok(r) => {
            let src = match r.source {
                NameSource::IpLiteral => "ip-literal".to_string(),
                NameSource::Live => "live (mesh DNS answered)".to_string(),
                NameSource::SelfClaim { age_secs } => {
                    format!("SELF-CLAIM, age {} (signed by the name's own anchor)", crate::names::age_human(age_secs))
                }
                NameSource::Snapshot(m) => {
                    format!("signed snapshot serial {}, age {}", m.serial, m.age_human())
                }
                NameSource::Observed { age_secs } => {
                    format!("NONCANONICAL observed record, age {}", crate::names::age_human(age_secs))
                }
            };
            println!("{} → {} via {}", args.name, r.addr.ip(), src);
            Ok(())
        }
        Err(f) => {
            println!("{} unresolved: {f}", args.name);
            std::process::exit(1);
        }
    }
}

async fn publish_claim(profile: &Profile, args: PublishClaimArgs) -> anyhow::Result<()> {
    // Identity: the anchor comes from THIS context's leaf — the same
    // extraction rule verifiers apply, so a claim we can't verify is a
    // claim we refuse to mint.
    // read_classified (not a bare `?`) so a missing/unreadable cert gives
    // the same path-naming ProfileMissing/ProfileDenied every other command
    // gets, instead of a bare `No such file or directory`.
    let cert_pem = profile.read_classified(&profile.cert_path())?.into_bytes();
    let anchor = crate::names::leaf_identity_from_pem(&cert_pem)
        .ok_or_else(|| anyhow::anyhow!("context cert carries no goat identity (CN anchor / URI-SAN org=/vendor=)"))?;

    // Name + IP default to this device's mesh peer identity, zone-shaped
    // against the verified snapshot (the name consumers will ask for).
    let (nb_name, nb_ip) = netbird_self().await;
    let ip = args
        .ip
        .or(nb_ip)
        .ok_or_else(|| anyhow::anyhow!("no --ip and the netbird daemon reported none"))?;
    let name = match args.name {
        Some(n) => n,
        None => {
            let zone = crate::names::verified_zone(profile)
                .ok_or_else(|| anyhow::anyhow!("no verified snapshot to derive the zone from — pass --name"))?;
            let peer = nb_name
                .ok_or_else(|| anyhow::anyhow!("netbird daemon reported no peer name — pass --name"))?;
            format!("{peer}.{zone}")
        }
    };

    let artifact = serde_json::to_vec(&ClaimArtifact {
        format: CLAIM_FORMAT.to_string(),
        name: name.clone(),
        ip: ip.clone(),
        anchor: anchor.clone(),
        issued_at_unix: crate::names::now_unix(),
        ttl_seconds: args.ttl,
    })?;
    let key_pem = profile.read_classified(&profile.key_path())?;
    let sig_bytes = sign_artifact(&key_pem, &artifact)?;
    use base64::Engine as _;
    let b64 = &base64::engine::general_purpose::STANDARD;
    let env = ClaimEnvelope {
        artifact_b64: b64.encode(&artifact),
        sig_b64: b64.encode(&sig_bytes),
        leaf_pem: String::from_utf8(cert_pem)?,
    };
    let env_json = serde_json::to_vec(&env)?;

    if let Some(out) = &args.out {
        std::fs::write(out, &env_json)?;
        println!("wrote envelope to {}", out.display());
    }
    println!("claim: {name} → {ip} (anchor {anchor}, ttl {}s)", args.ttl);
    if args.no_publish {
        return Ok(());
    }
    let key = format!("ops/names/claim/{anchor}");
    let session = super::pubsub::open_session_for(profile).await?;
    session
        .put(&key, env_json)
        .await
        .map_err(|e| anyhow::anyhow!("publish {key}: {e}"))?;
    // Give the router a beat to route before the session drops.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    session.close().await.map_err(|e| anyhow::anyhow!("close: {e}"))?;
    println!("published to {key}");
    Ok(())
}

async fn collect(profile: &Profile, args: CollectArgs) -> anyhow::Result<()> {
    let session = super::pubsub::open_session_readonly(profile).await?;
    let sub = session
        .declare_subscriber("ops/names/claim/**")
        .await
        .map_err(|e| anyhow::anyhow!("subscribe: {e}"))?;
    println!(
        "collecting ops/names/claim/** → {} ({})",
        args.out.display(),
        if args.listen == 0 { "follow".to_string() } else { format!("{}s window", args.listen) }
    );
    let mut merged = load_existing(&args.out);
    let deadline = (args.listen > 0)
        .then(|| tokio::time::Instant::now() + std::time::Duration::from_secs(args.listen));
    loop {
        let sample = if let Some(d) = deadline {
            match tokio::time::timeout_at(d, sub.recv_async()).await {
                Ok(r) => r,
                Err(_) => break, // window elapsed
            }
        } else {
            sub.recv_async().await
        };
        let Ok(sample) = sample else { break };
        let bytes = sample.payload().to_bytes();
        match serde_json::from_slice::<ClaimEnvelope>(&bytes) {
            Ok(env) => {
                if let Some((claim_name, issued)) = envelope_key(&env) {
                    upsert(&mut merged, env, &claim_name, issued);
                    write_atomic(&args.out, &merged)?;
                    println!("claim for {claim_name} (issued_at {issued}) — {} held", merged.len());
                }
            }
            Err(e) => eprintln!("ignored non-envelope payload on {}: {e}", sample.key_expr()),
        }
    }
    write_atomic(&args.out, &merged)?;
    println!("done — {} claim(s) in {}", merged.len(), args.out.display());
    Ok(())
}

/// Syntactic key for collector-side dedup: (claimed name, issued_at).
/// NOT verification — consumers verify at read; the collector only keeps
/// the freshest envelope per name so the file stays bounded.
fn envelope_key(env: &ClaimEnvelope) -> Option<(String, i64)> {
    use base64::Engine as _;
    let artifact = base64::engine::general_purpose::STANDARD
        .decode(env.artifact_b64.trim())
        .ok()?;
    let claim: ClaimArtifact = serde_json::from_slice(&artifact).ok()?;
    Some((claim.name.to_ascii_lowercase(), claim.issued_at_unix))
}

fn upsert(merged: &mut Vec<ClaimEnvelope>, env: ClaimEnvelope, name: &str, issued: i64) {
    merged.retain(|e| match envelope_key(e) {
        Some((n, i)) => !(n == name && i <= issued),
        None => false, // drop unparseable strays
    });
    if !merged.iter().any(|e| matches!(envelope_key(e), Some((n, i)) if n == name && i > issued)) {
        merged.push(env);
    }
}

fn load_existing(path: &std::path::Path) -> Vec<ClaimEnvelope> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_atomic(path: &std::path::Path, envs: &[ClaimEnvelope]) -> anyhow::Result<()> {
    let body = serde_json::to_vec_pretty(envs)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// This device's netbird peer name + mesh IP, best-effort from the local
/// daemon (`netbird status --json`). The peer name is the FQDN's first
/// label set — the mesh-zone rendering appends the snapshot zone.
async fn netbird_self() -> (Option<String>, Option<String>) {
    let out = tokio::process::Command::new("netbird")
        .args(["status", "--json"])
        .output()
        .await;
    let Ok(out) = out else { return (None, None) };
    if !out.status.success() {
        return (None, None);
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return (None, None);
    };
    let name = v
        .get("fqdn")
        .and_then(|f| f.as_str())
        .and_then(|f| f.split('.').next())
        .map(|s| s.to_string());
    let ip = v
        .get("netbirdIp")
        .and_then(|i| i.as_str())
        .map(|i| i.split('/').next().unwrap_or(i).to_string());
    (name, ip)
}

/// Sign the artifact with whatever key the context holds: Ed25519 (the
/// fleet's zenoh leaf keys — raw 64-byte signature) or P-256 (DER).
/// Matches verify_claim's dual-algorithm acceptance exactly.
fn sign_artifact(pem: &str, artifact: &[u8]) -> anyhow::Result<Vec<u8>> {
    {
        use ed25519_dalek::pkcs8::DecodePrivateKey as _;
        if let Ok(k) = ed25519_dalek::SigningKey::from_pkcs8_pem(pem) {
            use ed25519_dalek::Signer as _;
            return Ok(k.sign(artifact).to_bytes().to_vec());
        }
    }
    {
        use p256::pkcs8::DecodePrivateKey as _;
        if let Ok(k) = p256::ecdsa::SigningKey::from_pkcs8_pem(pem) {
            let sig: p256::ecdsa::Signature = k.sign(artifact);
            return Ok(sig.to_der().to_bytes().to_vec());
        }
    }
    use p256::elliptic_curve::SecretKey;
    let sk: SecretKey<p256::NistP256> = SecretKey::from_sec1_pem(pem)
        .map_err(|e| anyhow::anyhow!("context key is not Ed25519 PKCS#8, P-256 PKCS#8, or SEC1 EC PEM: {e}"))?;
    let k = p256::ecdsa::SigningKey::from(sk);
    let sig: p256::ecdsa::Signature = k.sign(artifact);
    Ok(sig.to_der().to_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_for(name: &str, issued: i64) -> ClaimEnvelope {
        use base64::Engine as _;
        let artifact = serde_json::to_vec(&ClaimArtifact {
            format: CLAIM_FORMAT.to_string(),
            name: name.into(),
            ip: "100.127.0.67".into(),
            anchor: "0123456789abcdef0123456789abcdef".into(),
            issued_at_unix: issued,
            ttl_seconds: 3600,
        })
        .unwrap();
        ClaimEnvelope {
            artifact_b64: base64::engine::general_purpose::STANDARD.encode(artifact),
            sig_b64: "c2ln".into(),
            leaf_pem: "-".into(),
        }
    }

    #[test]
    fn collector_keeps_freshest_per_name() {
        let mut merged = vec![env_for("a.example", 100), env_for("b.example", 100)];
        upsert(&mut merged, env_for("a.example", 200), "a.example", 200);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|e| envelope_key(e) == Some(("a.example".into(), 200))));
        // A stale duplicate never displaces a fresher held claim.
        upsert(&mut merged, env_for("a.example", 150), "a.example", 150);
        assert!(merged.iter().any(|e| envelope_key(e) == Some(("a.example".into(), 200))));
        assert_eq!(merged.len(), 2);
    }
}
