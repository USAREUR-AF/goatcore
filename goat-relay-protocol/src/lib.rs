//! Wire-protocol library for the Netbird relay (`rels://`).
//!
//! Clean-room implementation of the protocol specified in
//! `docs/design/netbird-relay-wire-protocol.md`. This crate is a pure
//! parse/serialize/state-machine library — no I/O, no async runtime. The async
//! transport layer lives in the consuming binary crate (Block 33B work).
//!
//! See also: `docs/project/block-33-progress.md` for working state, and
//! `docs/project/implementation-plan.md` §"Block 33" for the surrounding
//! engineering plan.
//!
//! # License posture
//!
//! Apache-2.0 (project default for own code). The protocol semantics this
//! crate implements are derived from reading upstream `netbirdio/netbird/`
//! source — `relay/` (AGPLv3) for routing and lifecycle semantics, and
//! `shared/relay/messages/` (BSD-3-Clause) for wire-format definitions.
//! Per Oracle v Google / Sega v Accolade, protocols are not copyrightable
//! subject matter; this crate transliterates no upstream code.
//!
//! # Module layout
//!
//! Mirrors the spec doc's section structure:
//!
//! - [`transport`] — outer transport layer (WebSocket and QUIC).
//! - [`framing`] — relay-level message framing within a transport stream.
//! - [`messages`] — typed message bodies (auth, peer-state, payload, close).
//! - [`handshake`] — auth-FSM helpers (Validator trait + HandshakeRequest).
//! - [`keepalive`] — in-conn HealthCheck state machines (Sender + Receiver).
//! - [`error`] — protocol error types.

#![doc(html_root_url = "https://docs.rs/goat-relay-protocol/0.1.0")]

pub mod error;
pub mod framing;
pub mod handshake;
pub mod keepalive;
pub mod messages;
pub mod transport;

/// Crate version string, exposed for build/runtime introspection.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constant_matches_cargo() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }
}
