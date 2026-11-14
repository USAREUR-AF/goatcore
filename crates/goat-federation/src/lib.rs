// Federation package v1 — docs/design/federation-package-spec.md.
//
// The specification is the authority and this is one implementation of it. The
// byte-exact conformance vectors in ops/federation/vectors/ are NORMATIVE: if
// this code and a vector disagree, this code is wrong.
//
// WHY THIS IS NOT `ops/federation/reference/internal/fedpkg`. That reference
// implementation is Go, in its own module, under an `internal/` path — which
// is unreachable from Rust twice over, once by language and once by Go's own
// visibility rule. There is no shared object here to reuse and no way to
// produce one without turning the reference into a service or a fork, both of
// which the specification's own framing rejects ("a partner must be able to
// implement this from scratch, in any language, without running our
// software"). So this is a second implementation, held to the vectors rather
// than to the reference — which is the arrangement the vectors were built for.
//
// PRIVILEGE BOUNDARY, visible in the module graph rather than only in prose:
//
//   json / package / compose / diff   pure. No I/O, no clock, no environment.
//                                     `now` and the verifying keys are
//                                     PARAMETERS, so composing a partner's
//                                     package provably cannot touch anything.
//   anchor                            pure. Picks the trust anchor out of PEM
//                                     bytes that `derive` read.
//   derive                            reads host state through a `Probe`:
//                                     this machine, or a host the caller
//                                     reaches (goatbuilder's SSH probe).
//                                     Only the commands `is_read_only_probe`
//                                     lists. Never writes.
//   render                            builds configuration text in memory. The
//                                     caller decides whether it reaches disk.
//   verbs                             the six federation verbs over the modules
//                                     above, shared by every front end.

pub mod anchor;
pub mod compose;
pub mod derive;
pub mod diff;
pub mod json;
pub mod package;
pub mod render;
pub mod verbs;

#[cfg(test)]
mod vectors;
