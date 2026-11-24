# Operator-action audit — making backend admin actions attributable

**Status:** Built + **live-fired on ZAGROS 2026-06-14** (M-068) — bus emit + encryption + restricted-read deny + per-person actor, plus the portal per-person identity + operator-read auditing, all proven on the live fabric. Roadmap legs (direct-ansible-bypass callback, host-sudo/MinIO→OpenObserve) remain per §8.6. **Finding:** F-207. **Anchors:** [ADR 1025](../adr/1025-zenoh-dynamic-tenant-authz-slot-pool.md) (read posture + encryption doctrine), [ADR 1035](../adr/1035-operator-namespace-ops-canonical.md) (`ops/`), [ADR 0811](../adr/0811-audit-log-integrity.md) (audit integrity), [`zenoh-audit-external-subscriber.md`](zenoh-audit-external-subscriber.md), [`observability-wide-events.md`](observability-wide-events.md).

## BLUF

The platform audits vendors and fabric data well, but **cannot say which backend operator did what**: portal
mutations record a shared literal `actor="operator"`, operator reads aren't audited, and the highest-privilege
work (cert mint, identity/ACL creation) runs out-of-band of the portal entirely. The fix is not to chase each
downstream system — it is to **emit one normalized operator-action event from every operator surface,
attributed to a person, into one stream**, instrumented at the chokepoint the operator already uses (`goat
admin`). The stream must be **operator-read-only** (vendors must not watch the admin trail), achieved by a
static restricted-read ACL tier plus payload encryption to the operator roster, per ADR 1025.

This is an **accountability** design (who did what), not an anti-tamper design. It does not try to stop a
malicious operator from later editing storage; if that bar is wanted, the WORM audit tier (ADR 0811) is the
separate mechanism, and §7 notes how to ride it.

## 1. What's broken today (verified)

| Surface | Action recorded? | Attributed to a person? |
|---|---|---|
| Portal vendor mutations (create/update/delete, slot-onboard) | yes — `audit_events` row | **no** — literal `"operator"` (shared token) |
| Portal operator reads (access graph, audit presign, snapshot, vendor-activity) | **no** — mutation-only table; access log omits principal | no |
| `goat admin onboard` → ansible (NetBird peer / Ory identity / ACL render) | **no** — runs locally, never touches the portal | no |
| `goat admin client onboard` → offline-CA `bundle-create` (cert mint) | **no** | no |
| SSH / sudo / direct Postgres / MinIO admin | per-surface host logs only | depends on per-person keys |

Root causes: a **single shared operator Bearer token** (`OperatorAPIKey`, one string); an `audit_events` table
that only writes on mutations and hard-codes `"operator"`; and the operator's real power living in
**locally-run tooling** that emits nothing central.

## 2. Principle

> One operator-action event schema. Emitted from every surface. Attributed to a person. Into one stream. Readable only by operators.

The leverage is **instrument the chokepoint, not the downstream systems.** `goat admin` is the binary that
*invokes* the out-of-band actions (it runs the ansible onboard and the offline-CA mint). Instrumenting it
captures those actions centrally even though they execute locally — without touching ansible, the CA, or the
portal's internals.

## 3. The event schema

A single protobuf message, `OperatorActionV1`, published once per action (and once more on completion when the
action is long-running):

| Field | Meaning |
|---|---|
| `actor` | the **person** (operator handle) — never a shared label |
| `surface` | `goat-cli` \| `portal` \| `ansible` \| `bundle-create` \| `host` \| `minio` |
| `action` | semantic verb (`admin.onboard`, `vendor.delete`, `cert.mint`, `access.topic.read`, …) |
| `target` | what it acted on (handle, vendor prefix, cert CN, topic key, host) |
| `env` | goatnet (`zagros` / `nova` / …) |
| `outcome` | `started` \| `ok` \| `failed` (+ error string on failure) |
| `host` / `src` | where it ran (operator laptop id, mgmt host) |
| `args_digest` | redacted/hashed argument summary — never raw secrets |
| `ts` | HLC-stamped emit time |

Topic: **`ops/audit/operator/<actor>`** (per-actor key so the existing rocksdb `ops/**` store + audit
subscriber persist and dedup it; co-located with the other `ops/` operator surfaces per ADR 1035).

**Retention — 1 year, dedicated store.** `goat audit ops` reads back via a GET against the router's
rocksdb, and the shared `ops/**` store keeps only `goat_zenoh_retention_days` (7d, the operational
replay window) — so audit history silently aged out at 7 days (found 2026-07-01: a 17-day-old live-fire
read back zero). Fixed with a **dedicated `ops_audit_operator` rocksdb storage** on `ops/audit/operator/**`
with its own `goat_zenoh_audit_retention_days` TTL (**default 365d**). Zenoh writes a matching sample
to *every* storage whose key-expr covers it, so operator actions land in both the 7d `ops` store and this
1y store; a GET on `ops/audit/operator/**` reads the full year back from the dedicated store (no CLI or
emit change). Cheap on disk — operator actions are small, infrequent, encrypted protobufs. This is the
*readable* retention floor; the audit-subscriber's MinIO tier (30d GOVERNANCE → cold COMPLIANCE) remains
the append-only WORM evidence record (ADR 0811). The `<env>` can override the TTL per group_vars.

## 4. Two layers: intent and effect

- **Intent layer — "what the operator told the system to do."** Captured at `goat admin` + portal + ansible
  callback + `bundle-create`. Rich, semantic, person-attributed. Emits `OperatorActionV1` to
  `ops/audit/operator/**`. This is where "who did what" is answerable.
- **Effect layer — "what actually executed on a box."** sudo/auth/auditd, MinIO audit webhook, optional
  pgaudit. Ships to the observability stack (OpenObserve, ADR 0815). Correlated to the intent layer by
  `actor` + `ts`.

For a single query pane you can either accept two stores correlated by actor+time (pragmatic, low effort) or
forward an effect-layer digest onto the bus so `ops/audit/operator/**` is the one stream (more work). Given
the accountability-not-forensics goal, two correlated stores is an acceptable v1.

## 5. Per-surface capture + identity

| Surface | Capture mechanism | Person identity comes from |
|---|---|---|
| `goat admin *` | emit hook wrapping each subcommand (start + finish); `goat` already has a Zenoh session | the active profile/context handle |
| Portal mutations + reads | replace the literal `"operator"` with the authenticated principal; add an audit row on the sensitive *reads*; log principal in the access middleware | per-person operator token (§6) |
| Direct `ansible-playbook` | custom callback plugin → `zenoh put`/rest-pub-bridge to `ops/audit/operator/ansible/<actor>` | invoking OS user + git committer + `operator` env |
| `bundle-create` (offline CA) | emit a `cert.mint` event (actor, ca-id, CN/subject, serial, ttl, outcome) | the operator running the mint |
| Host SSH/sudo | sudo I/O logging (+ optional auditd/tlog session record) → OpenObserve | per-person SSH key (no shared login) |
| MinIO admin | MinIO audit webhook → OpenObserve | already per-person via Ory OIDC SSO |

## 6. Per-person operator identity (the prerequisite)

No emission helps while the actor is `"operator"`. Required at every surface:

- **Portal:** change `OperatorAPIKey string` → a set of named operator keys (`{handle: key}`); `resolvePrincipal`
  returns `Principal{Operator:true, Actor:<handle>}`; handlers pass `principal.Actor` to `LogAuditEvent`
  instead of the literal. Backward-compatible: a single legacy key maps to a default handle.
- **`goat`:** the profile context handle is the actor (already exists for slot-alias rendering).
- **Hosts:** per-person SSH keys, no shared `goat` login, sudo not bare-root.
- **MinIO:** already per-person (Ory).

## 7. Keeping non-operators from watching the admin trail

This is the sharp constraint: the fabric is **community-read by default** — the router ACL
(`ops/ansible/roles/goat-zenoh/templates/config.json5.j2`) grants `trial-peer-r-all`
(`declare_subscriber`/`query` on `["**","**/@adv/**"]`) to *every* CA-rooted peer. A stream under `ops/**` is
therefore world-readable to vendors unless explicitly restricted. (The *write* side is already safe —
`operator-w-ops` grants ingress `put` on `ops/**` to the operator identity only.)

ADR 1025 prescribes the doctrine for exactly this: **a static restricted-read tier** for the rare read
exceptions, **and encryption — not permissions — for anything that must be truly secret.** Apply both:

1. **Restricted-read ACL tier on `ops/audit/operator/**`** (as built). This is the first read carve-out in
   `ops/` (everything there is community-read today); the `release/<set>/**`-shaped pattern ADR 1025 blesses.
   The exact mechanism is dictated by Zenoh 1.9.0's **two** precedence layers (verified from source,
   `net/routing/interceptor/authorization.rs`): *within one subject* deny-tree-first wins; *across multiple
   matching subjects* allow-from-any wins (the decision loop breaks on the first `Allow`). Because the
   universal read rule is bound to one subject (`trial-peer-read`, matching every TLS peer), the carve-out is:
   - Add ONE `deny` rule (`put`,`delete`,`declare_subscriber`,`query`,`declare_queryable`,`reply` on
     `ops/audit/operator/**`) bound to **`trial-peer-read` only**. Within that subject, deny beats its own `**`
     allow → the prefix is denied for every peer.
   - **No allow rule is needed for the operator/infra peers** — they each ALSO match a *dedicated* subject
     (`operator`, `infra-*`) whose policy already carries `trial-peer-r-all`/`trial-peer-deliver-all` on `**`
     (⊇ the prefix); across-subject allow-from-any rescues them. Vendors/slots match `trial-peer-read` for
     reads only (their own subjects grant writes), so they stay denied. That boundary — "has a dedicated
     operator/infra subject" vs "vendor/slot" — *is* the operator-vs-vendor line.
   - **The audit subscriber gets a new dedicated `audit-subscriber` subject** (CN `audit-subscriber`) bound to
     the universal read+deliver rules, so it keeps persisting `ops/audit/operator/**` past the deny (without
     this the deny would create an F-203-class silent audit gap).
   - The load-bearing deny vector is **egress delivery** (`put`/`delete`), not the subscriber declaration: a
     vendor subscribes on `**` (which the narrower deny key doesn't match at declare-time), but each delivered
     sample's concrete key *is* included by `ops/audit/operator/**`, so router→subscriber delivery is denied.
   - **It does not affect other `ops/` subtrees** — `ops/fabric/health/**`, `ops/time/**`, `ops/mesh/**`,
     `ops/presence/**` are different prefixes; their community-read consumers (`goat health`, panoscope) are
     untouched.
2. **Payload encryption to the operator roster.** Encrypt `OperatorActionV1` bodies to the age recipients
   already defined in `40-dev-ca-operators.envrc` (the operator-roster identity tier). Even if the read ACL is
   mis-set or later widened, the body is ciphertext to a non-operator. Operators read via a decrypt-aware tool
   (`goat audit ops` / a dedicated reader); panoscope shows the envelope, not the body — acceptable for a
   sensitive stream.

> **Precedence verification — DONE (source-level).** The deny/allow precedence the carve-out relies on was
> verified by reading Zenoh 1.9.0's `policy_decision_point` + the across-subject decision loop in
> `net/routing/interceptor/authorization.rs` (and its own `tests/acl.rs::test_pub_sub_allow_then_deny`): deny
> wins within a subject, allow-from-any wins across subjects. The render-check
> (`ops/ansible/roles/goat-zenoh/tests/render-check.py`) now asserts the carve-out renders in both ACL
> modes. **Still required before "resolved" (the standard last step for every feature here):** a live-fire on
> an ZAGROS deploy — restart the router with the new ACL, then prove a *vendor-cert* session is denied the
> subscribe/delivery on `ops/audit/operator/**` while an *operator-cert* session and the audit subscriber are
> allowed. The encryption
> layer (2) holds regardless of the ACL, which is why doctrine pairs them.

## 8. Build status (what shipped in code)

1. ✅ **Per-person operator identity** — portal `PORTAL_OPERATOR_API_KEYS` (`handle:key,…`, legacy single key
   maps to handle `operator`); `Principal.Actor` resolved constant-time; `actorOf(r)` replaces the literal
   `"operator"` at every mutation `LogAuditEvent`. `goat` actor handle resolves env `GOAT_OPERATOR_HANDLE` >
   profile `operator_handle` > context name > `"operator"`.
2. ✅ **`goat admin` emit hook** — `admin.onboard`/`admin.revoke`, `vendor.create`/`vendor.delete`,
   `cert.mint` (client onboard) emit `OperatorActionV1` to `ops/audit/operator/<actor>`; best-effort with a
   local fallback log so a down bus never loses the record and never blocks the action.
3. ✅ **Cert-mint event** — emitted from `goat admin client onboard` after `bundle-create` succeeds (chokepoint,
   so the offline-CA mint is captured without changing `bundle-create`).
4. ✅ **Restricted-read ACL tier + encryption** (§7) — deny on `trial-peer-read`, dedicated `audit-subscriber`
   subject, render-check coverage; payload age-encrypted to the operator roster. Precedence verified from
   Zenoh source; on-wire live-fire remains (deploy step).
5. ✅ **Portal read auditing + access-log principal** — the operator-only reads (access-topic, audit-presign,
   admin snapshot/vendor-activity, progress, trashgoat list) now write an audit row; the access log carries a
   `principal` label.
6. ⏳ **Not yet built** (roadmap, called out so it isn't read as covered): the ansible callback plugin for the
   *direct* `ansible-playbook` bypass path, and host sudo/auth + MinIO-audit → OpenObserve shipping. Today a
   direct (non-`goat`) playbook run or a raw SSH/sudo action is captured only by git history / host logs, not
   the unified stream. The `goat admin` chokepoint covers the normal path.

## 9. As-built contract (keep in sync with code)

- **Schema:** `ops/zenoh-trial-peers/operator-action-audit/proto/operator_action_v1.proto` (canonical) +
  hand-written prost struct in `crates/goat-cli/src/cmd/audit.rs` (lock-step, no codegen — same convention as
  the health meters).
- **Topic:** `ops/audit/operator/<actor>` where `<actor>` is the per-person operator handle (operators have no
  UUID anchor — the handle is their stable machine id; this is the one place a handle is a path segment, and
  it is an operator handle, not a vendor slot alias, so the ADR 1039 alias-never-in-machine-paths invariant
  for *vendor* slots is unaffected).
  - **Session actors.** An action approved in the identity console and executed by the onboard-runner
    (`admin.offboard`, `admin.hold.release`) is filed under `kratos-<identity id>`: the approving admin's
    Kratos identity id, taken by the console from the authenticated session (the OIDC `sub` oauth2-proxy
    forwards), never looked up from an onboarding record by email, since any record can carry an admin's
    email (GHSA-3m4r). It is a valid kebab actor (43 characters) and cannot collide with an operator handle,
    which the manifest schema caps at 32. `goat audit ops` prints it whole and `--actor kratos-<id>` lists one
    admin's console approvals; mapping it to a name needs the Kratos admin API, or the console's
    `onboard_offboard_cut` / `release_hold` event, which carries the email beside the identity id.
  - **Executor.** The runner's own record of the same cut (`admin.revoke-native`, actor = the runner id) names
    the executor; `admin.offboard` carries it only inside the hashed `args_digest`.
- **Runner fallback and visibility.** `goat audit emit` is the fallible path and keeps no fallback of its own.
  When it fails for an `admin.offboard` record, the onboard-runner appends the record, in the CLI's fallback-log
  shape, to `--audit-fallback` (default `<repo-root>/out/audit/operator-actions.log`, mode 0600), and writes
  `operator_audit: failed` with the reason back on the offboard result. The console stores it on the row,
  shows "Offboarded, operator-action record not written" on the card, and audits the result as `partial`.
  The access cut never waits on the audit record.
- **On-wire:** `age(roster, prost_encode(OperatorActionV1))` — the whole serialized message is age-encrypted
  (binary, non-armored) before `session.put` (CongestionControl::Block). Ciphertext is what the audit
  subscriber persists.
- **Env:** `GOAT_OPERATOR_HANDLE` (actor), `GOAT_OPERATOR_AGE_RECIPIENTS` (encrypt roster, comma/newline
  `age1…`), `GOAT_OPERATOR_AGE_IDENTITY` / `GOAT_DEV_CA_AGE_IDENTITY` (read-side decrypt). Portal:
  `PORTAL_OPERATOR_API_KEYS`.
- **Read path:** `goat audit ops` — query/subscribe `ops/audit/operator/**`, decrypt with the operator age
  identity, render (or `--json`); undecryptable samples shown as such, never abort.
- **Fallback log:** `<profile dir>/audit/operator-actions.log` (0600) — plaintext JSON appended when emit
  fails (no recipients / bus down), so nothing is silently lost.
- **ACL:** `ops/ansible/roles/goat-zenoh/templates/config.json5.j2` — `audit-operator-read-deny` rule on
  `trial-peer-read` + `audit-subscriber` subject/policy; both ACL modes; covered by `tests/render-check.py`.
  nova-lan-zenoh is **not** carved out (it runs no operator-action surface); apply there if it ever does.

## Console mesh move review

The identity console emits `mesh_move_declaration_review` through its
existing audit path when it first saves a governed confirmation result.
The event identifies the actor, target identity and peer, site, source and
destination groups, and operation ID. It records whether the declaration was
ready or preparation failed. It omits the YAML and planner credentials.
Replaying a saved confirmation reuses the operation ID and adds no audit
event. The converger owns the separate deployment-time membership change.

## What an operator key authorizes, and who may hold one

An operator key is not a larger version of the `vp_` portal API key every
onboarded person holds. The two answer different questions, and they must not be conflated.

| | portal API key (`vp_<slot-anchor>_…`) | operator key (`op_…`) |
|---|---|---|
| Who holds one | every onboarded person who publishes | console admins and named services |
| Scope | that person's own slot | the whole goatnet, on both planes |
| How it arrives | minted with the certificate when the person collects their fabric credential, shown once | requested in the console, approved by a different admin, collected once |
| What it is for | signing in to the Portal, their own topics, their own machines, their own labels and grants | administering other people's orgs, slots, machines and logins |

**Both planes.** The portal resolves any key in the directory to
`Principal{Operator: true}` (`auth.go`), and this console's machine API resolves
the same keys as operator principals (`onboard_api.go`) — where the operator key
IS the authorization, with no step-up. A key therefore carries the Portal's
whole admin API *and* the console's: login groups, account state, invites, the
org registry, onboarding intake. There is no read-only key and no single-org
key; scope inside the key does not exist.

**So eligibility is the same question the console's own admin gate asks.** The
request route refuses any login the permissions matrix ([ADR 1073](../adr/1073-rbac-adherence-principle.md))
does not grant `admin` on `id-admin-console`, a reason is mandatory, and the
approval queue shows each pending requester's admin groups — the fact an
approver most needs and the one the queue used to omit. Services get their key
from their own Ansible role, never through the queue.

**A key may not raise its own holder.** `POST /api/users/groups` refuses a change
to the identity behind the calling key. Every legitimate use of that route is an
admin changing somebody else; without the refusal, a key holder could grant
themselves `PLATFORM-ADMIN` and the audit would carry their handle on both
halves of it.

**A demotion takes the key.** Sponsorship and the operator key follow the level
([platform-admin-lifecycle](platform-admin-lifecycle.md)), and offboarding
revokes keys as one of its planes (ADR 1187).

## Issuing a key no longer means a redeploy

The operator key is what makes an action attributable, so the friction of
getting one is not a neutral detail: while issuing meant editing a gitignored
envrc on somebody's laptop and redeploying the portal, the path of least
resistance was to share a key that already worked — which is precisely the
attribution this design exists to prevent.

The portal now re-reads its operator-key directory
(`PORTAL_OPERATOR_API_KEYS_DIR`, every `PORTAL_OPERATOR_API_KEYS_RELOAD`,
default 30s). Two tiers, deliberately different:

| tier | source | lifetime | who may change it |
|---|---|---|---|
| env | `PORTAL_OPERATOR_API_KEYS` | frozen at boot | the deploying laptop |
| directory | files under the keys dir | live | anything that can write the directory |

Env wins on a handle collision, so the directory can never redefine a handle
the deployment pinned, and an empty directory cannot revoke the break-glass
key. Within the directory, **adding a file grants and removing one revokes** —
revocation is a delete, effective within one interval.

The failure direction is the part worth stating. A reload that cannot READ the
directory keeps the last good set and records why: losing a bind mount for a
moment is an availability blip, and turning it into a fleet-wide
deauthentication would be an outage. A reload that reads successfully and finds
fewer keys does shrink the set, because that is revocation and treating it as
an error would make revoking impossible. "Could not read" and "read, and it
says less" are different events.

This retires an earlier rule that human keys must stay env-managed "so they
remain removable". That was correct while the directory was startup-only — a
file nobody notices the removal of is a credential nobody can retire. With the
directory live, it is the better home for human keys, and it is what lets the
console (and `goat admin operator-key`) issue and revoke one without touching a
laptop or restarting anything.

### Requesting one, both ways

Same engine, two front doors — the console for a person acting for themselves,
the API for an operator acting for somebody else (typically at the point of
onboarding them).

| what | console | API |
|---|---|---|
| request | `GET/POST /operator-key` | `POST /api/onboard/operator-keys` |
| collect (once) | `GET /operator-key/collect/{id}` | `GET /api/onboard/operator-keys/{id}/collect?handle=` |
| queue | `GET /onboarding/operator-keys` | `GET /api/onboard/operator-keys?status=` |
| decide | `POST /operator-keys/{id}/{action}` | `POST /api/onboard/operator-keys/{id}/{approve\|reject\|revoke}` |
| reconcile | shown on the queue | `POST /api/onboard/operator-keys/reconcile?apply=` / `goat admin operator-key reconcile` |

On the browser form the handle comes from the SESSION and never the form: a
form-supplied handle would let anyone with a console session park a request
under somebody else's name and have an admin approve a key that then attributes
the requester's actions to that person — which inverts the point of an
attributable key. The API accepts a handle from an OPERATOR caller (its own
principal is already operator-equivalent, and the approval gate is what makes
naming somebody else safe) — but until #1373 the API had no self-service path
at all, so `goat admin operator-key request` was circular for anyone who did
not already hold an operator key: the one command a first-time platform-admin
would reach for needed the thing it was supposed to get them.

**The API door now also admits a self-request, the machine-realm sibling of
the browser form's own rule.** A non-operator caller — a participant's `vp_`
key, resolved live against the portal by `onboardAPIGate` the same way every
other onboard-API route resolves one, and sponsorship-authorized (ADR 1071,
which platform-admin/iam-admin already carry automatically) — may request or
collect an operator key ONLY for themselves. "Themselves" is the ROSTER
handle, not the principal's `Handle`: on a slot-pool goatnet that is the
32-hex org anchor, and a key filed under it would name the key file and every
attributed action after a hex string that ADR 1182's revoke-by-handle never
finds. The door binds through the login the portal recorded on the identity
(ADR 1072) and the same email → handle resolver the browser pages use
(`operatorKeySelfHandle` → `handleForLoginEmail`); a token that resolves to
no onboarded person is refused (`unbound_principal`, HTTP 403) with a message
that names the console page. A handle naming anyone else is refused
(`self_only`, HTTP 403), never silently substituted. The door also starts
under the holder rule that [platform-admin-lifecycle](platform-admin-lifecycle.md)
§ Operator key states and the browser door does not yet enforce: the
participant's sponsorship cap, which follows their level (ADR 1182), must be
`platform-admin` (`not_platform_admin`, HTTP 403, audited with the recorded
level), and a request says what admin task needs it (`missing_reason`, HTTP
400). Gating the browser door and the approvals the same way is a separate
change. Beyond that this is not a new authorization concept, just the API
door growing the same "for yourself" case the console door already had — an
operator caller is still the only way to act on somebody else's behalf,
unchanged. See `ops/goat-id-console/internal/handlers/operatorkeys_api.go`
and panoscope-core's `PortalError::HttpWithBody`, whose `code` carries the
API's `{"error":{code,message}}` code through so the CLI can tell these apart
from "no operator key at all".

Collection is bound to the owner on both paths, and happens exactly once.

**A DIFFERENT admin approves — the two-person rule (#210).** "An admin
approves" always meant a second human. Now enforced, in the store so both doors share one rule: approve and
reject refuse with `ErrOperatorKeySelfDecision` (HTTP 403) when the decider
matches the row's owner. The browser door compares the session actor (email
plus resolved handle); the API door compares the acting operator principal's
handle (per-handle keys, F-207) to the target handle. A refused attempt still
emits `operator_key_self_decision_refused` — an attempt is signal.
**Self-revoke stays allowed**: removing your own live key is de-escalation,
and slowing that down only hurts credential hygiene.

**A decision mails the requester.** Approving is the grant — the file
lands and the portal reloads within seconds — but the key still has to be
COLLECTED, once, from the page it was requested on, and until 2026-08-27
nothing told the requester approval had happened. Both doors rendered their
result to the DECIDER. The two-person rule sharpens that into a total blackout:
when the only two admins on a goatnet approve each other's requests, each
watches a decision succeed and neither gets a signal about their own.

Approve and reject now mail the requester from both doors
(`internal/handlers/operatorkeys_notify.go`), on the same contract as the
reviewer notifications in [`onboarding-notifications.md`](onboarding-notifications.md):
sent AFTER the decision is recorded and the key file written, best-effort — a
mail failure never fails a decision that already granted access — and audited
either way (`operator_key_email_approved` / `operator_key_email_rejected`), so
a notification that reached nobody is visible rather than silent. The approve
body leads with the collect step and the once-only warning and says the key is
already live, because the failure it exists to end is a person waiting for an
install step that does not exist. The recipient is the row's email, falling
back to the onboarding record for machine-door rows filed with a handle and no
email; no resolvable address means nobody is mailed and nothing is audited as
sent. **Revoke is deliberately not mailed** — it is de-escalation and already
self-evident to the holder, whose key stops working.

**The uncollected sweep.** The approval mail covers the common case;
someone who does not read it still ends up holding a live key they have never
seen. The admin queue now names them — `status=active`, `delivered_at=0`, older
than an hour — with the age of the approval, because a day-old row is a person
who missed the mail and a month-old row is a key that should be revoked.
The store query is `Store.UncollectedOperatorKeys`.

Still not closed: the sweep is a banner on a page an admin has to visit. Nothing
alerts on it, so a goatnet nobody is administering accumulates them silently.

**A non-match is only evidence when the two identities were comparable.** The
first cut of the rule compared whatever forms each side happened to carry and
read "no form matched" as "a different person" — which is false whenever the
two sides live in different dimensions. A key requested through the machine
door (`goat admin operator-key request x`) records a handle and no
email; an admin whose login resolves to no handle presents only an email.
Those two could not have matched however identical the people behind them
were, so the rule passed and the requester could approve their own key from
the browser — the same failure, one door over. Two changes close it, both in
`Store.OperatorKeyDecisionCheck`, the single call approve, reject AND the
queue's row rendering all make:

- **The owner side is resolved before comparing.** When the row carries no
  email, `ProposalEmailForHandle` looks one up from the owner's onboarding
  record, so a handle-filed row and an email-known session meet in the same
  dimension. This is `ActiveProposalForEmail` run the other way round, with
  the same definition of "active".
- **A pair that still cannot be compared is REFUSED**, with
  `ErrOperatorKeyUnverifiedDecider` (HTTP 403, API code `unverified_decider`,
  audited as `operator_key_unverified_decider_refused`). An unverifiable
  second person is not a second person, and the fail-closed stance already
  applied to a decider presenting no identity at all. The refusal names both
  dimensions and points at `goat admin operator-key`, whose principal is
  always a handle — so the operator is redirected, not dead-ended. The queue
  renders those rows without controls for the same reason: the page must
  never offer a button the store will refuse.

**State is checked before the two-person rule.** A row that is already decided
cannot be decided again by anybody, so an owner re-clicking a stale Approve
gets `ErrOperatorKeyWrongState` (409), not "that is your own request" — the
refusal has to name the reason the click actually failed.

**The queue is admin-only and discoverable.** The queue and its decisions sit
behind `adminGate` (matrix-driven per ADR 1073), NOT the sponsor-admitting
`onboardGate` the rest of `/onboarding` takes — sponsorship says nothing about
fitness to grant portal-operator credentials. Discovery: the onboarding review
dashboard renders an **"Operator keys"** tab at the end of the stage strip,
after a divider (the stage tabs narrate one candidate's journey; this is a
cross-cutting queue, not a lifecycle stage), with the pending count in the
existing pill idiom — quiet at zero, never vanishing. Admin sessions only;
sponsor markup carries neither tab nor count. On the queue, an admin's own
pending row stays visible but renders "Your request — another admin must
decide" in place of controls, from the same call the store enforces with — as
does the third case, a row this session cannot be compared against, which says
so and names the machine door instead of offering a control that would 403.

**Single-admin bootstrap (break-glass).** On a goatnet with exactly one admin,
requester ≠ decider deadlocks the first key — deliberately. The paths around
it already exist and stay: an API decision under a *different* operator
principal (e.g. the onboard-runner's key via `goat admin operator-key`), or a
direct file drop into the key directory — which reconcile then reports as
"not issued here", i.e. visible, not silent. The refusal message names this
path, so a sole admin is redirected at the moment of refusal rather than
dead-ended. The two-person rule keys on decider-vs-row-owner only — the
principal that files a request is not recorded on the row and is not part of
the rule — so what makes the one-principal sequence detectable after the fact
is attribution: the request event and the decision event are each recorded
under the operator principal that made the call, so a request filed and then
approved under the same key reads as one principal on both halves.

**Approval IS the grant, and the wording must not borrow the SSH page's.** An
SSH key still needs an installer run against hosts, so approved there is
genuinely not access. An operator key is live as soon as the file lands and the
portal reloads. Saying "not yet" here would leave people waiting for a step that
does not exist.

**Write-then-report, remove-then-record.** Approving writes the key file before
reporting success and rolls the approval back if the write fails, because a row
saying active with no file is a person holding a credential the fabric refuses.
Revoking removes the file BEFORE the record changes, because the reverse — a row
saying revoked while the file is still there — is access the record claims was
taken away.

**Reconcile is on the queue, unprompted — and splits three ways.** The record
and the directory can drift, and the three kinds of drift call for different
actions:

| case | meaning | what happens |
|---|---|---|
| revoked here, still granted | the record says access was removed and it was not | `--apply` removes the file |
| not issued here | a grant with no request behind it — a service's own deploy, or a key predating this queue | reported, never touched |
| recorded but not granted | somebody holds a key the portal refuses | reported; not repairable |

The middle row is not a nicety. The first version of reconcile treated every
file without an active row as an unexplained grant and offered to delete it;
run against a live goatnet it classified the onboard-runner's key and
pre-registry human keys that way. Applying it would have cut the runner off
from the portal. **Ownership is the dividing line: a handle this console has a
row for is one it may enforce; a handle it has never heard of belongs to
somebody else.**

The last row is not repairable by design. The console stops holding the secret
after delivery, and manufacturing a replacement would hand somebody a
credential nobody approved — so it says revoke and re-issue.

Reused, not rebuilt: the `audit_events` table, the `ops/` namespace + three-tier audit subscriber, the
operator age roster (`40-dev-ca-operators.envrc` + the Go `filippo.io/age` pattern), MinIO OIDC SSO,
OpenObserve.

## 10. Considered alternative — a separate admin/audit Zenoh router

Weighed and **declined for now** (2026-06-14): run operator/admin traffic on a *dedicated* zenoh router
instead of the shared fabric + restricted-read tier. Decision: **single router**, keep it simple; revisit only
on a forcing requirement.

Why single-router wins today:
- **Confidentiality is already met by encryption, not topology.** The payload is age-encrypted to the operator
  roster, so a separate router's topology isolation is redundant for *content*.
- **It's the established doctrine.** ADR 1025 (write-integrity layer + static restricted-read tiers +
  encryption for true secrets) and the brokerless principle ([`zenoh-brokerless-consequences.md`]) — a parallel
  plane is the "central second component" that doc warns to re-justify before adding.
- **Cost.** A second router is another cert / endpoint / rocksdb / audit-subscriber / monitoring target **per
  goatnet**, operators dual-homed (two sessions + cert sets), and it forfeits the two things that made this
  cheap: emit is free because `goat` already holds the main session, and persistence is free because the
  existing three-tier audit subscriber already captures `ops/**`. Also a new ops/onboarding path against the
  ADR 1016 complexity budget.

The honest residual single-router leaves open: **metadata.** Encryption protects content robustly; the
*existence/timing/actor-key* of `ops/audit/operator/**` samples is protected only by the ACL carve-out holding
(an F-177/F-205-class regression risk, guarded by `render-check.py`; a leaked envelope is opaque ZIDs +
ciphertext). A separate router would close even that.

Flip to a separate plane only if: (a) admin-*metadata* secrecy becomes a hard requirement; (b) the audit plane
must survive a degraded main fabric (continuity — weak here, the operator administers *that* fabric); or (c) a
separate classification/trust enclave is introduced (then use the existing standalone-goatnet / federated-peering
pattern, ADR 1020 / Block 72). Nothing built here blocks that later split.

## 11. Non-goals

- Tamper-proofing against a malicious operator editing storage (that's ADR 0811 WORM, a separate bar —
  riding `ops/audit/operator/**` does get append-only WORM persistence for free via the existing subscriber,
  noted but not the design goal).
- Real-time alerting / SIEM correlation (the stream enables it; building it is later).
- Changing the access-control model — access is sound; this is purely about attribution + confidentiality of
  the audit trail.
