//! QUIC transport (UDP path of `rels://`).
//!
//! Spec section: §2 of `docs/design/netbird-relay-wire-protocol.md`.
//!
//! **Headline:** the relay's QUIC path uses **RFC 9221 unreliable datagrams**,
//! NOT QUIC streams. Each relay-protocol message rides as one datagram.
//! No retransmit, no in-order guarantees, no per-message HOL blocking. This
//! is a major contrast to the WS (TCP) path, where 1 message = 1 binary
//! frame on a reliable stream.
//!
//! Constants extracted from upstream + a single client-side helper for the
//! `rels://` → `udp://host:443` URL preparation. The actual QUIC I/O loop
//! lives at the consuming binary crate (Block 33B work — runtime + QUIC
//! library decision land there; candidate libraries `quinn`, `s2n-quic`,
//! `quiche`).

use crate::error::{ProtocolError, Result};

/// ALPN string advertised on the QUIC TLS handshake. Constant `NBalpn` in
/// upstream (`shared/relay/tls/alpn.go:3`).
pub const ALPN: &str = "nb-quic";

/// Conservative initial QUIC packet size (bytes). Upstream constant
/// `QUICInitialPacketSize` (`shared/relay/constants.go:10`). IPv6-safe MTU
/// floor; effective per-datagram payload is approximately this minus QUIC
/// packet overhead (~30 bytes), giving roughly ~1200 bytes of relay-message
/// payload per datagram.
pub const INITIAL_PACKET_SIZE: u16 = 1232;

/// Client-side `KeepAlivePeriod` for QUIC connections to the relay
/// (`shared/relay/client/dialer/quic/quic.go:43`).
pub const CLIENT_KEEP_ALIVE: core::time::Duration = core::time::Duration::from_secs(30);

/// Client-side `MaxIdleTimeout` for QUIC connections to the relay
/// (`shared/relay/client/dialer/quic/quic.go:44`).
pub const CLIENT_MAX_IDLE: core::time::Duration = core::time::Duration::from_secs(4 * 60);

/// Server-side `max_idle_timeout` for QUIC connections. Set equal to the
/// client's [`CLIENT_MAX_IDLE`] so the negotiated idle timeout — the
/// minimum of the two endpoints' advertised values (RFC 9000 §10.1) — is
/// the intended 4 minutes, NOT quinn's much shorter default. Without the
/// server advertising this, the negotiated window collapses to quinn's
/// default (tens of seconds), which idles out low-traffic relayed flows
/// even though the Go client requested 4 minutes (`quic.go:47`). This is
/// the QUIC half of F-217 (relay idle-session flap).
pub const SERVER_MAX_IDLE: core::time::Duration = CLIENT_MAX_IDLE;

/// Server-side `keep_alive_interval` for QUIC connections. The server
/// actively PINGs idle connections so liveness does not ride solely on the
/// client's (unreliable, datagram-carried) keepalive. Mirrors the client's
/// [`CLIENT_KEEP_ALIVE`] cadence and is well under half of
/// [`SERVER_MAX_IDLE`], so several consecutive keepalives must be lost
/// before an idle connection is reaped.
pub const SERVER_KEEP_ALIVE: core::time::Duration = CLIENT_KEEP_ALIVE;

/// Application-level error code used for graceful close (server side
/// `CloseWithError(0, "normal closure")`). When the remote peer closes
/// with this code, the local side should map it to a "peer closed cleanly"
/// outcome rather than an error (parity with WS's `StatusNormalClosure`).
pub const CLOSE_NORMAL_ERROR_CODE: u64 = 0x0;

/// Default port for `rels://` URLs without an explicit port.
pub const DEFAULT_RELS_PORT: u16 = 443;

/// Default port for `rel://` URLs without an explicit port.
pub const DEFAULT_REL_PORT: u16 = 80;

/// Estimated maximum relay-message payload that fits in one QUIC datagram,
/// after subtracting typical QUIC packet overhead from
/// [`INITIAL_PACKET_SIZE`]. This is a defensive lower bound for callers
/// deciding whether to send via QUIC vs falling back to WS.
///
/// Concrete production implications (per spec §2 asymmetry note):
/// - `Auth` messages (≤212 bytes) always fit.
/// - `AuthResponse` with typical short URLs (~50 bytes) always fits.
/// - `Transport` messages with WireGuard-MTU-bound payloads (~1500 bytes)
///   may exceed this if the path MTU is tight.
/// - Long peer-state lists (244 peers × 36 = 8784 bytes) DO NOT fit;
///   would fail with `ErrMessageTooLarge` if attempted via QUIC.
pub const ESTIMATED_DATAGRAM_PAYLOAD: u16 = INITIAL_PACKET_SIZE - 32;

/// Prepare a `rel://` or `rels://` URL for QUIC dial — strips scheme, adds
/// default port if absent, returns `host:port` form. Mirrors upstream's
/// `prepareURL` (`shared/relay/client/dialer/quic/quic.go:74-100`).
///
/// # Errors
///
/// Returns [`ProtocolError::Parse`] if the input scheme is neither `rel://`
/// nor `rels://`, or if the host component is malformed.
///
/// # Examples
///
/// ```
/// use goat_relay_protocol::transport::quic::prepare_dial_url;
/// assert_eq!(
///     prepare_dial_url("rels://relay-rlb.legacy.example:33073").unwrap(),
///     "relay-rlb.legacy.example:33073"
/// );
/// // No port — default 443 for rels://
/// assert_eq!(
///     prepare_dial_url("rels://relay-rlb.legacy.example").unwrap(),
///     "relay-rlb.legacy.example:443"
/// );
/// // No port — default 80 for rel://
/// assert_eq!(
///     prepare_dial_url("rel://relay-rlb.legacy.example").unwrap(),
///     "relay-rlb.legacy.example:80"
/// );
/// // IPv6 host without explicit port preserves brackets in host:port output
/// assert_eq!(
///     prepare_dial_url("rels://[2001:db8::1]").unwrap(),
///     "[2001:db8::1]:443"
/// );
/// ```
pub fn prepare_dial_url(url: &str) -> Result<String> {
    let (host_part, default_port) = if let Some(rest) = url.strip_prefix("rels://") {
        (rest, DEFAULT_RELS_PORT)
    } else if let Some(rest) = url.strip_prefix("rel://") {
        (rest, DEFAULT_REL_PORT)
    } else {
        return Err(ProtocolError::Parse(format!(
            "unsupported scheme: {url} (expected rel:// or rels://)"
        )));
    };

    if host_part.is_empty() {
        return Err(ProtocolError::Parse(format!("empty host in {url}")));
    }

    // If the host already contains a port (last colon NOT inside brackets),
    // pass it through. Otherwise append the default port, bracketing IPv6
    // literals the same way Go's `net.JoinHostPort(strings.Trim(host, "[]"), port)` does.
    if has_explicit_port(host_part) {
        Ok(host_part.to_owned())
    } else {
        let trimmed = host_part.trim_matches(|c| c == '[' || c == ']');
        if trimmed.is_empty() {
            return Err(ProtocolError::Parse(format!("empty host in {url}")));
        }
        if trimmed.contains(':') {
            Ok(format!("[{trimmed}]:{default_port}"))
        } else {
            Ok(format!("{trimmed}:{default_port}"))
        }
    }
}

/// Returns true if `host` already has an explicit `:port` suffix outside
/// any IPv6 bracket pair. Mirrors the heuristic in `net.SplitHostPort`.
fn has_explicit_port(host: &str) -> bool {
    if let Some(end_bracket) = host.rfind(']') {
        // Bracketed IPv6: only an explicit port if there's a colon after `]`.
        host[end_bracket + 1..].starts_with(':')
    } else {
        // Bare host: explicit port only when there is a single colon (`host:port`).
        // Multiple colons indicate an unbracketed IPv6 literal without a port,
        // which falls through to the `JoinHostPort` branch and gets bracketed.
        host.matches(':').count() == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpn_constant_matches_upstream() {
        assert_eq!(ALPN, "nb-quic");
    }

    #[test]
    fn initial_packet_size_matches_upstream() {
        assert_eq!(INITIAL_PACKET_SIZE, 1232);
    }

    #[test]
    fn keep_alive_and_idle_match_upstream() {
        assert_eq!(CLIENT_KEEP_ALIVE.as_secs(), 30);
        assert_eq!(CLIENT_MAX_IDLE.as_secs(), 4 * 60);
    }

    #[test]
    fn server_idle_matches_client_so_negotiated_window_is_four_minutes() {
        // The negotiated idle timeout is the MIN of both ends (RFC 9000
        // §10.1). If the server advertised less than the client's 4-minute
        // request, idle flows would be reaped early. Equal values
        // keep the negotiated window at the intended 4 minutes.
        assert_eq!(SERVER_MAX_IDLE, CLIENT_MAX_IDLE);
        assert_eq!(SERVER_MAX_IDLE.as_secs(), 4 * 60);
    }

    #[test]
    fn server_keepalive_is_safely_under_half_the_idle_window() {
        // Several consecutive keepalives must be lost before an idle
        // connection times out; require interval < idle/2 so at least one
        // loss is tolerated.
        assert!(SERVER_KEEP_ALIVE.as_secs() * 2 < SERVER_MAX_IDLE.as_secs());
    }

    #[test]
    fn close_normal_error_code_is_zero() {
        assert_eq!(CLOSE_NORMAL_ERROR_CODE, 0x0);
    }

    // Compile-time invariants — `const _: () = assert!(...)` is the
    // canonical Rust idiom for a constant-expression assertion. Was
    // formerly a runtime `#[test]` with `assert!(<const>)`, which
    // clippy correctly flags as `assertions_on_constants`.
    const _: () = assert!(ESTIMATED_DATAGRAM_PAYLOAD < INITIAL_PACKET_SIZE);
    const _: () = assert!(ESTIMATED_DATAGRAM_PAYLOAD >= 1100);

    #[test]
    fn prepare_url_with_explicit_port() {
        assert_eq!(
            prepare_dial_url("rels://relay-rlb.legacy.example:33073").unwrap(),
            "relay-rlb.legacy.example:33073"
        );
    }

    #[test]
    fn prepare_url_default_port_rels() {
        assert_eq!(
            prepare_dial_url("rels://relay-rlb.legacy.example").unwrap(),
            "relay-rlb.legacy.example:443"
        );
    }

    #[test]
    fn prepare_url_default_port_rel() {
        assert_eq!(
            prepare_dial_url("rel://relay-rlb.legacy.example").unwrap(),
            "relay-rlb.legacy.example:80"
        );
    }

    #[test]
    fn prepare_url_ipv6_with_brackets_and_port() {
        // Bracketed IPv6 with explicit port should pass through.
        assert_eq!(
            prepare_dial_url("rels://[2001:db8::1]:33073").unwrap(),
            "[2001:db8::1]:33073"
        );
    }

    #[test]
    fn prepare_url_ipv6_brackets_no_port_gets_default() {
        // Bracketed IPv6 with no port should append default and stay a valid SocketAddr.
        let dialed_rels = prepare_dial_url("rels://[2001:db8::1]").unwrap();
        assert_eq!(dialed_rels, "[2001:db8::1]:443");
        assert!(
            dialed_rels.parse::<std::net::SocketAddr>().is_ok(),
            "dial target {dialed_rels:?} must parse as a valid SocketAddr"
        );

        let dialed_rel = prepare_dial_url("rel://[2001:db8::1]").unwrap();
        assert_eq!(dialed_rel, "[2001:db8::1]:80");
        assert!(
            dialed_rel.parse::<std::net::SocketAddr>().is_ok(),
            "dial target {dialed_rel:?} must parse as a valid SocketAddr"
        );
    }

    #[test]
    fn prepare_url_unbracketed_ipv6_no_port_brackets_with_default() {
        // Mirrors Go's `net.JoinHostPort("2001:db8::1", "443")`.
        let dialed = prepare_dial_url("rels://2001:db8::1").unwrap();
        assert_eq!(dialed, "[2001:db8::1]:443");
        assert!(dialed.parse::<std::net::SocketAddr>().is_ok());
    }

    #[test]
    fn prepare_url_rejects_empty_host() {
        for bad in ["rels://", "rel://", "rels://[]", "rel://[]"] {
            assert!(
                matches!(prepare_dial_url(bad), Err(ProtocolError::Parse(_))),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn prepare_url_rejects_unsupported_scheme() {
        for bad in [
            "https://example.com",
            "udp://example.com",
            "example.com",
            "rels:example.com", // missing //
        ] {
            assert!(
                matches!(prepare_dial_url(bad), Err(ProtocolError::Parse(_))),
                "should reject {bad}"
            );
        }
    }
}
