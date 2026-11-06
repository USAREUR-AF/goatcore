# ADR 1163 — Per-visit attribution for fenced surfaces

| Field | Value |
|---|---|
| Status | **Accepted.** Doors and emitter live on ZAGROS since 2026-09-06 through review (#754, #765); the 2026-09-05 deployment went ahead of its PRs and was reverted (#726). Other goatnets not enabled. |
| Amends | ADR 1073 (RBAC adherence — the door taxonomy), ADR 1090 (microscope — its v1 scope boundary) |
| Related | rbac-adherence, microscope, [operator-action-audit](../design/operator-action-audit.md), [audit-cold-storage-offload](../design/audit-cold-storage-offload.md), F-457 |

## Decision

Three things, in this order.

1. **airdefenders, brief and dataflows get authenticated doors.** An
   oauth2-proxy fence with an empty allowed-groups list: any identity the site
   Hydra will issue a token for passes, and no group is refused.
2. **`SurfaceVisitV1` records one authenticated request per row**, age-encrypted
   to the operator roster, published onto `ops/audit/surface/<surface>/<actor>`.
3. **Microscope reads it as an eighth source**, deriving counts and times from
   the key path without decrypting anything.

## Why a door, when the door refuses nobody

The three surfaces were `mesh-only`: a Traefik IP allowlist on the mesh range
was the only gate, so every caller that reached the mesh was served and no
identity was ever recorded. The permissions console meanwhile rendered them as
denied to every group but PLATFORM-ADMIN, which is what surfaced this.

Making them attributable does not require refusing anyone, and refusing people
was not wanted: these are a game and two briefing decks, and the operator's
position is that everyone should be able to open them. But **a surface with no
door produces no consent grant and no session**, so it is invisible to the
identity machinery no matter how much of that machinery exists. The door is not
the access control here. The door is the instrument.

The cost to a visitor is one silent redirect round-trip per surface. Kratos
holds a 24-hour SSO session, so the second surface someone opens
re-authenticates without ever showing them a password prompt. Cookies stay
locked per-FQDN so a stolen one cannot be replayed against a sibling console —
widening the cookie domain would remove the round-trip and that protection
together, and is rejected.

## Why the consent grant is not enough

Hydra records one grant per (person, client), with `handled_at` moving to the
last time it was exercised. That answers *has this person reached brief, and
when last*. Repeat visits collapse into one row, so it can never answer *how
many times* or *on which days* — and `skip_consent` clients, which every
internal surface is, never show a human a screen at all.

`SurfaceVisitV1` is the per-request half. The two are complementary and neither
subsumes the other:

| | consent grant | SurfaceVisitV1 |
|---|---|---|
| Survives the emitter being down | yes | no |
| Survives the grant being revoked | **no — vanishes from the API** | yes |
| Answers "how many times" | no | yes |

Read together they separate *never came* from *came, and the grant was later
revoked*. Apart, neither can. That is the F-177 silent-denial class applied to
attribution, and it is why this ADR adds a source rather than replacing one.

## Why the door's log is the source

Every field comes from oauth2-proxy's own request log, which is the only place
holding both the validated identity and the request line.

The alternatives were considered and rejected. **The application** cannot be the
source: two of the three surfaces are static file servers, so there is no code
to instrument, and adding some would mean shipping an application to serve a
deck. **The Traefik access log** could carry the identity header in one config
line, but that log deliberately excludes client identity — the config says so,
and notes User-Agent is withheld for the same reason — and reversing that is a
policy change that should not ride along inside a feature.

The request log format is pinned to explicit JSON rather than parsed from the
positional default, which is ambiguous the moment a field is empty and which
upstream may reorder in a minor release without anything failing loudly.

## The capture gate, and who opened it

Microscope's design doc draws a line this ADR crosses:

> Not surveillance infrastructure beyond what already exists. v1 composes
> streams we already record… New capture is explicitly out of v1 and gated on
> its own justification.

This is new capture: it records which person opened which page, every time,
across every fenced surface. The justification is the operator's, on the record:
they asked for visit counts and history, having been shown what the consent
grant does and does not give, and confirmed the shape below before it was built.
Recording it here is the gate working as intended — the doc did not forbid the
capture, it required someone to own it.

Four constraints follow from that, and they are the reason the shape is what it
is:

- **Age-encrypted to the operator roster**, the same envelope
  `OperatorActionV1` uses. A mis-set read ACL leaks opaque ciphertext, not a
  browsing trail. The emitter holds only public recipients and cannot read back
  what it writes. An empty roster file is a hard refusal, not a fallback.
- **Query strings are dropped.** A query can carry the content of a search,
  which is a different and more sensitive thing than the fact of a visit.
- **Unauthenticated requests are never recorded.** Probers, scrapes and
  api-route 401s all arrive with no user; traffic that never involved a person
  must not appear as a person visiting a page.
- **Microscope does not decrypt.** Counts and times come from the key path;
  the per-request detail needs a roster key, which the service does not hold.
  Watching is itself audited, as every microscope look already is — so a query
  about who visited what is visible to a later query about the watcher.

## Consequences

**It generalises for free.** Eleven surfaces sit behind a fence after this
change. The same emitter and the same event give visit history for all of them
with no per-surface work; building it for three would have been the strange
choice.

**Retention is inherited, not invented.** The audit-subscriber already persists
`ops/audit/**` to the MinIO governance tier at 30 days, then to cold storage,
and microscope already reads that tier. No storage was built here.

**One new door class, not three copies.** Eight surfaces already carry their own
oauth2-proxy sidecar, each re-stating the same knobs and each fixed separately
when one bit. `oauth2-proxy-surface` carries them once. The eight are
deliberately not migrated: rewriting eight working fences to prove a refactor is
how a fence stops working.

**A door that is not routed to changes nothing, and a route pointed at a door
that is not running takes the surface down.** They are deployed in one run, in
that order, with the Hydra registration verified between them.

## Alternatives rejected

**Leave the surfaces ungated and infer visits from the Traefik access log.** No
identity is recorded there by design; the surfaces would stay unattributable.

**Gate them to a group.** Refuses people the operator wants admitted, and buys
nothing this needs — attribution comes from authentication, not authorization.

**Widen the cookie domain to remove the redirect round-trip.** Removes the
protection that a stolen cookie cannot be replayed against a sibling console.
The round-trip is silent; the protection is not replaceable.

**Publish visits in plaintext.** Cheaper to read, and turns any read-ACL mistake
into a disclosed browsing trail. The encryption is the reason this capture is
acceptable at all.
