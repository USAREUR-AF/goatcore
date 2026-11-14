# ADR 1229 — A partner's new package under the same key is confirmed within confirmation 1

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-13 |
| Deciders | operator |
| Repo scope | goat core (`crates/goatbuilder/src/federate/confirm.rs`, `ops/federation/standup/directory.py`, `docs/design/federation-key-rotation.md`) |
| Amends | [ADR 1225](1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) decision 3 (confirmation 1 also covers a partner's new package signed by the key already bound) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (the peering) · [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) (a channel is a cache of the binding, never its source) · [`federation-key-rotation.md`](../design/federation-key-rotation.md) §5 · [`federation-package-spec.md`](../design/federation-package-spec.md) §5.1 · [`peering-directory-service.md`](../design/peering-directory-service.md) |

## TL;DR (for a layperson)

When a partner changes the key its network link uses, it publishes a new signed description
of itself, a "package". Our side has to take that new package in place of the old one, or the
link between the two sides stays down. The package is signed by a key both operators already
confirmed by reading it aloud, but a signature alone cannot say which of two of the partner's
packages is the current one. So the two operators compare the new package's fingerprint on the
same call where they already compare keys, and only then does the command use it. This keeps
the number of human decisions as it was: it is part of the first one, not a new one.

## Context

A peering binds the partner's verifying key in phase 5, by both operators reading it aloud
(ADR 1225 decision 3, confirmation 1). Every package the partner publishes after that is
signed by the same key. When the partner rotates its WireGuard key (key K4 in
[`federation-key-rotation.md`](../design/federation-key-rotation.md) §2), its new package
differs from the old one only in `transport_offer.public_key`, and verifies under the bound
key.

Today our side never takes that package. The directory step keeps the package already on disk
whenever it still verifies under the bound key, and only warns that "the file on disk is what
composes" (`ops/federation/standup/directory.py:1320-1331`). The agreement does not carry the
transport key (`federation-package-spec.md` §8.1), so its digest does not change and
confirmation 2 does not stop either. Our phase 11 then keeps the partner's old peer key, and the
tunnel stays down.

Placing the new package automatically, because it verifies, would let a directory, or anyone
who can write the file on the hand-exchange path, replay an older package the partner signed.
A package's `issued` member is a date only (`federation-package-spec.md`, line 50), so two
packages of one day cannot be ordered by anything in them.

## Decision

- A package from the partner that verifies under the bound key and differs from the one on disk
  is a **new package**, whether the directory offers it or a person places it on the hand
  path (`FED_NO_DIRECTORY=1`).
- Phase 5 does not place a new package by itself. It shows which members changed and the new
  package's digest, and the two operators compare that digest on the call, as confirmation 1
  already does for the verifying key, the CA fingerprint and the agreement digest. A person
  confirms it: `yes` at a terminal, or `--accept-partner-package <digest>`, where the digest
  must equal the new package's.
- Only then is the old package moved aside to `<work>/theirs.canonical.json.previous-<UTC time>`
  and the new one placed, and the phases after 5 are planned again. Without the confirmation the
  run stops and names the digest, as it does for a package held back from the partner's
  pointer. `--yes` never confirms it.
- The confirmation is journaled with confirmation 1's kind, bound to the new package's digest. A
  later new package is a new question.
- This is part of confirmation 1. ADR 1225 decision 3 still has exactly two human
  confirmations, identity and admission, and identity now also covers "this is the partner's
  current package".

## Alternatives considered

**Place the new package automatically when it verifies under the bound key.** No prompt and no
ADR change, with the old package moved aside and the replacement journaled. Rejected: a
signature says who signed a package, not which of two signed packages is current, and `issued`
cannot order two of one day. A directory could replay an older package and have it composed,
which the identity confirmation exists to prevent.

**A third confirmation, separate from confirmation 1.** Rejected: it adds a human decision to a
command whose design keeps exactly two, for a question of the same kind (is this what the
partner says it is) that confirmation 1 already asks on the same call.

**Order packages by a new member (a sequence number or a timestamp).** Rejected for now: it
changes the package format both parties must read, so an older partner would not honour it,
and a version skew between two sovereigns is the normal case. It stays an option in the
package specification's own revisions.

## Consequences

**Positive.** A partner's transport rotation reaches our side, through a step the operator
already knows. A replayed or stale package cannot be placed by a directory or a file copy. The
two-confirmation shape of ADR 1225 is unchanged.

**Negative.** A partner's rotation needs the two operators on a call again, for one digest. A
run without a person and without `--accept-partner-package` stops at phase 5 until someone
confirms.

## Assessor view (SCA-R)

This keeps the partner's identity and its current transport material under a human
confirmation over a trusted channel, which maps to IA-3 (device identification and
authentication) and SC-12 (cryptographic key establishment and management): new key material
from a partner is accepted only after an out-of-band comparison, never because an untrusted
channel served it. The record of each confirmation, bound to the digest, supports AU-2 and
AU-10. Residual risk: an operator who confirms a digest without reading it on the call; the
compensating control is the same as for the existing key confirmation, the journaled digest and
the operator's own procedure.

## Trigger conditions for revisit

- The package specification gains a member that orders a party's packages and every supported
  goatcore release honours it: the confirmation could then be replaced by an automatic check.
- Report signing for the bilateral acceptance session is built and partners sign their
  rotation announcements: revisit whether the announcement can stand in for the call.
