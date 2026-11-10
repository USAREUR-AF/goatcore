//! TLS hand-off helper for the WS listener (kTLS on Linux when the
//! kernel TLS ULP is available; userspace elsewhere or as fallback).
//!
//! Spec: ADR 0709 D8 — rustls + the `ktls` crate (Linux-only). After
//! the rustls handshake completes, we hand the AEAD session keys to
//! the kernel via `setsockopt(SOL_TLS, TLS_TX, ...)` + `TLS_RX`;
//! per-packet AEAD then runs in-kernel via `sendmsg`/`recvmsg`,
//! bypassing userspace TLS framing on the steady-state hot path.
//!
//! The `ktls` crate requires the `TcpStream` to be wrapped in
//! [`ktls::CorkStream`] BEFORE the rustls accept. The cork drains
//! cleanly during the hand-off; after `config_ktls_server` returns Ok,
//! the resulting [`ktls::KtlsStream`] delegates plaintext I/O to the
//! underlying TCP socket while the kernel handles AEAD.
//!
//! # Platform behavior
//!
//! - **Linux + TLS ULP available** (per F-023
//!   — Ubuntu/Debian/RHEL stock kernels): [`accept_tls`] does the
//!   kTLS hand-off, returns a [`ktls::KtlsStream`] boxed as
//!   [`AcceptedStream`].
//! - **Linux without TLS ULP** (Docker Desktop's LinuxKit VM, some
//!   hardened/embedded kernels): [`accept_tls`] falls back to userspace
//!   `acceptor.accept(...)` and returns a plain
//!   `tokio_rustls::server::TlsStream` boxed as [`AcceptedStream`].
//!   The probe runs once at first call and is cached in a
//!   [`OnceLock`].
//! - **non-Linux (macOS dev, Windows):** always userspace.
//!
//! Downstream code (tokio-tungstenite WS upgrade + reader/writer
//! halves) is generic over the [`WsTransport`] marker trait
//! (`AsyncRead + AsyncWrite + Unpin + 'static`) so the boxed dynamic
//! dispatch is invisible at the call site.
//!
//! # Validation status
//!
//! - macOS dev: userspace path, exercised by `listener/ws.rs::tests`
//!   integration tests.
//! - Linux + Docker Desktop (no TLS ULP): userspace path, exercised
//!   by `ops/goat-relay/docker-compose.yml`'s `tester` service. F-023
//!   surfaced here.
//! - Linux + real TLS ULP (production): kTLS path, awaiting Block 33B
//!   step 8 tier 4 bench-env smoke on a t3.micro (per
//!   [`docs/operations/goat-relay-bench-env-smoke.md`](../../../docs/operations/goat-relay-bench-env-smoke.md)).
//!
//! # ADR 0709 D8 fallback
//!
//! Manual `setsockopt` was the documented secondary fallback if the
//! `ktls` crate lacks something we need. Not yet wired — the runtime
//! probe + userspace fallback covers the operational gap until tier 4
//! validates the kTLS path on real production-shape Linux.

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

/// Marker trait combining the surface tokio-tungstenite + our
/// reader/writer halves require. Implemented blanketly for any type
/// that satisfies the bounds (kTLS streams, plain TlsStream).
pub trait WsTransport: AsyncRead + AsyncWrite + Unpin + 'static {}

impl<T> WsTransport for T where T: AsyncRead + AsyncWrite + Unpin + 'static + ?Sized {}

/// Accepted stream — boxed dynamic dispatch so the
/// `cfg(target_os = "linux")` branch can choose between kTLS and
/// userspace at runtime per F-023.
pub type AcceptedStream = Box<dyn WsTransport>;

/// Runtime probe: is the kernel TLS ULP module loaded? Cached in a
/// `OnceLock` after the first call. Logs the verdict so operators see
/// it in container logs.
#[cfg(target_os = "linux")]
fn ktls_available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let avail = std::fs::read_to_string("/proc/sys/net/ipv4/tcp_available_ulp")
            .map(|s| s.split_whitespace().any(|p| p == "tls"))
            .unwrap_or(false);
        if avail {
            eprintln!("ws_ktls: kernel TLS ULP available — using kTLS hand-off");
        } else {
            eprintln!("ws_ktls: kernel TLS ULP NOT available — using userspace TLS");
        }
        avail
    })
}

/// Verdict of the boot-time kTLS probe. Public so [`main`](crate::main)
/// can decide policy (loud-warn vs fail-fast) before the first listener
/// accepts.
///
/// Block 50A.5 production-readiness hardening — operators want to know
/// the verdict at startup, not at first-connection time. Today the
/// userspace fall-back is silent in most ways: the relay accepts traffic
/// either way, just without the kernel-AEAD optimization (~30% throughput
/// loss per the M-009 / M-019 numbers). This boot probe makes the
/// fall-back loud and gives operators the option to fail-fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KtlsBootVerdict {
    /// kTLS hand-off path is wired up and the kernel ULP is loadable.
    /// Steady-state hot path will use kernel-AEAD bypass.
    Active,
    /// Linux host but the `tls` ULP is not in
    /// `/proc/sys/net/ipv4/tcp_available_ulp` (Docker Desktop's LinuxKit
    /// VM, hardened/embedded kernels, missing `/etc/modules-load.d/goat-
    /// relay-tls.conf` per F-023). Relay falls back to userspace TLS;
    /// throughput cap drops by ~30%.
    Unavailable,
    /// non-Linux platform (macOS dev, Windows). kTLS path was never
    /// compiled in; userspace TLS is the only path. Not a regression.
    NotApplicable,
}

/// Boot-time probe entrypoint. Forces the [`OnceLock`] initialization in
/// [`ktls_available`] before any listener accepts, so the verdict is
/// visible in startup logs (and to the caller for policy decisions).
///
/// Idempotent — safe to call multiple times; subsequent calls return the
/// cached verdict without re-reading `/proc`.
pub fn probe_at_boot() -> KtlsBootVerdict {
    #[cfg(target_os = "linux")]
    {
        if ktls_available() {
            KtlsBootVerdict::Active
        } else {
            KtlsBootVerdict::Unavailable
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!(
            "ws_ktls: kTLS not applicable on this platform (target_os != linux); userspace TLS only"
        );
        KtlsBootVerdict::NotApplicable
    }
}

/// Run the rustls handshake against `tcp`, then (on Linux + TLS ULP
/// available) hand the AEAD session keys to the kernel via the `ktls`
/// crate. Returns a [`Box<dyn WsTransport>`](AcceptedStream) that
/// downstream code (tokio-tungstenite + reader/writer halves) treats
/// uniformly.
///
/// # Errors
///
/// Returns the underlying I/O error from rustls accept, or — on the
/// kTLS path — an `io::Error` wrapping the `ktls::Error` if the kTLS
/// hand-off fails after the runtime probe said it should be available
/// (e.g., cipher suite incompatibility).
pub async fn accept_tls(
    acceptor: TlsAcceptor,
    tcp: TcpStream,
) -> std::io::Result<AcceptedStream> {
    #[cfg(target_os = "linux")]
    {
        if ktls_available() {
            let corked = ktls::CorkStream::new(tcp);
            let tls = acceptor.accept(corked).await?;
            let ktls_stream = ktls::config_ktls_server(tls)
                .await
                .map_err(|e| std::io::Error::other(format!("kTLS hand-off failed: {e}")))?;
            return Ok(Box::new(ktls_stream));
        }
        // Fall through to userspace path.
    }

    let tls = acceptor.accept(tcp).await?;
    Ok(Box::new(tls))
}
