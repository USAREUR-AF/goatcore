//! `goat-ingress --config <file>`: run the ingress adapter.
//! `goat-ingress --config <file> --check`: validate the file and the
//! adapter's identity, then exit.
//!
//! Operator page: docs/operations/ingress-adapter.md.

use goat_ingress::{run_source, Config, Stats};
use goat_provenance::{publish_cert, Signer};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn usage() -> ! {
    eprintln!("usage: goat-ingress --config <file> [--check]");
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "goat_ingress=info,zenoh=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut config_path: Option<PathBuf> = None;
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => {
                config_path = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage()))
            }
            "--check" => check = true,
            "-h" | "--help" => usage(),
            _ => usage(),
        }
    }
    let config = Config::load(&config_path.unwrap_or_else(|| usage()))?;
    let sources = config.sources()?;
    let signer = Signer::from_files(&config.cert_path(), &config.key_path())
        .map_err(|e| anyhow::anyhow!("adapter identity: {e}"))?;
    let router = config.router_endpoint()?;
    for s in &sources {
        tracing::info!(
            source = %s.slot, listen = %s.listen, sender = ?s.sender,
            key = %s.key_pattern(),
            "declared source"
        );
    }
    if check {
        println!(
            "ok: adapter {} ({}), {} source(s), router {router}",
            signer.anchor,
            signer.cn,
            sources.len()
        );
        return Ok(());
    }

    let mut zc = zenoh::Config::default();
    zc.insert_json5("mode", "\"client\"")
        .map_err(|e| anyhow::anyhow!("zenoh config mode: {e}"))?;
    let endpoints = serde_json_list(&router);
    zc.insert_json5("connect/endpoints", &endpoints)
        .map_err(|e| anyhow::anyhow!("zenoh config endpoint: {e}"))?;
    let tls = panoscope_core::tls::TlsConfig {
        enabled: true,
        root_ca_certificate: config.ca_roots_path().display().to_string(),
        connect_certificate: config.cert_path().display().to_string(),
        connect_private_key: config.key_path().display().to_string(),
    };
    panoscope_core::tls::apply_tls_to_zenoh_config(&mut zc, &tls)?;
    let session = zenoh::open(zc)
        .await
        .map_err(|e| anyhow::anyhow!("open a fabric session to {router}: {e}"))?;

    match publish_cert(&session, &signer).await {
        Ok(true) => tracing::info!(key = %goat_provenance::cert_key(&signer.anchor), "published the adapter certificate"),
        Ok(false) => tracing::info!("adapter certificate already published"),
        Err(e) => tracing::warn!("could not publish the adapter certificate (subscribers will grade INVALID until it is): {e}"),
    }

    let signer = Arc::new(signer);
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut tasks = Vec::new();
    let mut all_stats = Vec::new();
    for s in sources {
        let sock = tokio::net::UdpSocket::bind(s.listen)
            .await
            .map_err(|e| anyhow::anyhow!("source {}: bind {}: {e}", s.slot, s.listen))?;
        let stats = Arc::new(Stats::default());
        all_stats.push((s.slot.clone(), stats.clone()));
        tracing::info!(source = %s.slot, listen = %s.listen, "receiving");
        tasks.push(tokio::spawn(run_source(
            session.clone(),
            signer.clone(),
            s,
            sock,
            stats,
            stop_rx.clone(),
        )));
    }

    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.tick().await;
    loop {
        tokio::select! {
            _ = shutdown_signal() => break,
            _ = tick.tick() => {
                for (slot, st) in &all_stats {
                    tracing::info!(
                        source = %slot,
                        published = st.published.load(Ordering::Relaxed),
                        wrong_sender = st.wrong_sender.load(Ordering::Relaxed),
                        lint_dropped = st.lint_dropped.load(Ordering::Relaxed),
                        failed = st.failed.load(Ordering::Relaxed),
                        "totals"
                    );
                }
            }
        }
    }
    let _ = stop_tx.send(true);
    for t in tasks {
        if let Ok(Err(e)) = t.await {
            tracing::error!("source stopped: {e}");
        }
    }
    let _ = session.close().await;
    Ok(())
}

/// `tls/a:1, tls/b:2` -> `["tls/a:1","tls/b:2"]`.
fn serde_json_list(router: &str) -> String {
    let items: Vec<String> = router
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", items.join(","))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
