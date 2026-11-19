# Onboarding notifications — plan (for a fresh session)

**Status:** the **email** half of increments 1–3 is BUILT (2026-08-11, two changes —
see the two "Built" sections below); what remains planned is increment 2's **chat**
channel, which is still gated on the operator's channel/room decision.

## Built: candidate status read (2026-08-11)

The candidate's own invite link doubles as their status page. Before this, a
candidate who revisited their link after submitting was told *"this link has
already been used — ask your sponsor for a new invite"* while their request sat
in the queue: a fire-and-forget submit with no surface to check.

- `GET /api/onboard/preview` on a **consumed** invite resolves the invite's
  `proposal_id` and returns a `submission` block: a closed candidate-phase
  vocabulary (`pending-sponsor / pending-admin / provisioning / complete /
  rejected / issue`) + submitted/vouched/decided timestamps — never the raw
  queue status, never manifest content. The invite token is the read capability.
  (`internal/handlers/onboard_public.go`, `candidateSubmissionStatus`.)
- `web/static/onboard.html` renders it as a four-step progress strip; the
  submit-success message tells candidates to keep the link.
- The submit itself now emits an `onboard_submit` audit event (actor = the
  candidate's email, target = invite token prefix + proposal id) — previously
  `/audit`'s first trace of a candidate was the sponsor's decision.
- `/onboarding` with no `?status` lands **sponsor sessions on `proposed`**
  (their vouch gate) instead of `sponsor_approved` (the admin gate).

## Built: reviewer notifications, queue badges, queue age (2026-08-11)

The second half — the two transitions that leave a human waiting who is not
looking at the queue. All of it is console-native: the trigger points live here
now, so nothing posts to the portal and no new service appears.

**Five emails** (`internal/handlers/onboard_notify_ops.go`, plus
`internal/handlers/operatorkeys_notify.go` for the operator-key pair):

| Transition | Who is mailed | Audit action |
|---|---|---|
| submit → `proposed` | the proposal's `sponsor` | `onboard_email_sponsor_submit` |
| vouch → `sponsor_approved` | `ONBOARD_ADMIN_NOTIFY_EMAILS` | `onboard_email_admin_vouched` |
| either gate → `rejected` | the candidate | `onboard_email_rejected` |
| operator key → `active` | the requester | `operator_key_email_approved` |
| operator key → `rejected` | the requester | `operator_key_email_rejected` |

- Each carries a **deep link** to the queue at the stage the decision is waiting
  on (`/onboarding?status=proposed` / `?status=sponsor_approved`), composed from
  `consoleBase` — the console's own host, never `PORTAL_PUBLIC_URL`, which 404s
  on human paths. No configured base renders the stage in **words** rather than
  a link to nowhere.
- **A sponsor is mailed only when the recorded `sponsor` looks like an address.**
  A participant sponsor's value is a UUID org anchor and machine mints record
  labels like `operator` — mailing either is impossible, so those proposals
  produce no notification and no audit row claiming one.
- **The admin list is configured, not derived.** `ONBOARD_ADMIN_NOTIFY_EMAILS`
  (comma-separated; blank = off) in the console role env. `ADMINS` is a
  login-group name, not a mailbox; there is nothing in the identity plane to
  derive an address from.
- **The rejection body is the one increment 3 already wrote** — `notify.go`'s
  `candidateEmail("rejected")` had been unreachable since the extraction. It
  carries the reviewer's note when one was left.
- **The operator-key pair was added 2026-08-27** on the same contract, in
  `operatorkeys_notify.go`, after F-403:
  approving an operator key IS the grant, but the key still has to be
  collected once from the requester's own page, and both decision doors
  rendered only to the DECIDER. The two-person rule makes that a total
  blackout when two admins approve each other — each sees a decision land and
  neither gets a signal about their own. The deep link here is
  `/operator-key`, not the queue: the person being mailed has an action to
  take, not a decision to make. Recipient falls back to the onboarding record
  for machine-door rows filed with a handle and no email. **Revoke is
  deliberately not mailed** — de-escalation is self-evident to the holder.
- **Best-effort by contract, and audited either way.** Every send happens AFTER
  the transition is recorded, so a mail failure cannot fail a submit or a vouch.
  A disabled mailer is still audited (`result: error`, "mailer disabled") —
  someone was owed a message and did not get one, which is exactly the thing
  that must not be silent. An empty recipient list audits nothing, because
  nothing was owed.

**Increment 1's badges + self-refresh**, minus the chat leg:

- `GET /api/onboard/proposals/counts` → `{"proposed":N,"sponsor_approved":M}`.
  Gated by `onboardGate` — the same door as the queue page, NOT the machine key
  gate — so a sponsor session's counts are scoped to their own invitees. A count
  is a read like any other: telling a sponsor 40 are pending when they may see 2
  discloses the other 38.
- `web/static/onboard-queue.js` polls it every 20 s, updates the tab badges, and
  reloads only when a gate count actually moved AND that stage is the one on
  screen. The reload preserves the selected row and stage because both are
  already in the URL — no client-side re-render to keep in sync.

**Queue age (the D5 gap).** Rows now carry `created_at`, rendered as a relative
age. An `approved` row that has waited more than 10 minutes gets a marker whose
title says what it actually means: **a healthy onboard-runner claims an approved
proposal within seconds, so the runner is probably down** — the problem is not
the candidate. Age math is client-side; the server passes the timestamp.

**The enrolled flip is audited** (`internal/enrollwatch`): `onboard_enrolled`,
actor `enroll-watch`, before/after states. It is the one lifecycle transition
nobody decides — the watcher OBSERVED the setup key being used — so the audit
row is the only record it happened, and the actor must not read as an operator.

**Still not built here:** increment 2's chat push (the operator's channel/room
decision below is unmade), and any "minted / failed" note on the runner result.

## Built: the admin branch withheld the admin's own key (2026-08-11)

A platform-admin onboardee got a welcome email with no portal API key in it and
a 401 on his first sign-in. The key had been minted and was in the artifacts
that very email was rendered from. `internal/welcome/welcome.go`'s Portal
section switched on `r.AdminOperator` FIRST, and that branch printed no key at
all — it said the operator key "arrives out of band … REQUEST it from an admin"
and never fell through to the inline-key branch. Increment 7 recorded this as a
deliberate carve-out ("operators use an OPERATOR key … the inline branch must
not hijack them"), and a test pinned it.

**The carve-out conflated two different credentials.** They are additive:

| | what it is for | where it comes from |
|---|---|---|
| `vp_…` scoped key | signing INTO the Portal | minted at onboard, in the welcome email + the bundle's `goat-cli.cbor` |
| `op_…` operator key | the operator surfaces, `goat admin` | self-service request at the console's `/operator-key`; an admin approves, you collect once |

An admin tier does not make the first key inert, so the branch now prints it
inline for admins exactly as it does for developers, THEN explains the operator
key as the second, separate credential. `CredentialDeferred` semantics are
unchanged — a deferred admin still has no key to print and is still sent to
`/fabric-credential`.

**Two stale claims fixed alongside, because the same reader meets both.** The
guide's operator topic and its admin checklist still said the operator key was
hand-issued, "not self-service yet", and needed a Portal restart to take effect.
It has been self-service since the operator-action-audit work: request →
approve → collect, and approval IS the grant because the portal hot-reloads its
key directory. Both now say so and link the page.

**And the bundle contradicted the email.** `ops/goat-zenoh-portal`'s
`internal/certmint` rendered a `NOTES.md` into every minted tarball saying the
portal API key "is delivered out-of-band by the operator separately" — written
before the welcome email carried keys (increment 7), and read by exactly the
people the email had just handed one to. It now says where the key actually is
(the welcome email, and a second copy on the last line of the bundle's
`goat-cli.cbor` before `portal_endpoint`), how to use it (paste into Sign in, or
`goat profile init <cbor>`), and repeats the do-not-rotate-to-refresh warning.

## Built: candidate status read — the trigger points after extraction

The plan below predates the portal→console extraction; its trigger points live
in the console today (`handleOnboardSubmit`, `internal/handlers/onboarding.go`
`decideProposal` — the shared choke point for BOTH the web decide route and the
machine `/api/onboard/proposals/{id}/…` routes, so a hook placed there covers
both doors and cannot drift).

The email channel substrate note still holds: the
**email channel now exists**. `ops/goat-id-console/internal/mailer/` (AWS SES over
SMTP/587, tiny, `Enabled()` false when unconfigured) ships in the id-console and
already sends the **invite** email (#1447), and `terraform-zagros/ses.tf` provisions
the sender identity. Wiring the SMTP creds into a console is part of increment 3.
The sponsor/admin gates are still pull-only with no pending count.
This doc plans four increments; they compose.

Companion: [`three-stage-candidate-onboarding.md`](three-stage-candidate-onboarding.md)
(the flow + states) · [ADR 1062](../adr/1062-three-stage-delegated-candidate-onboarding.md)
· [ADR 1071](../adr/1071-sponsorship-authorization-at-onboarding.md) (who may
sponsor/vouch — the sponsor pool increments 1–2 notify).

## The two hand-off points that need a notification

The proposal lifecycle is `proposed → sponsor_approved → approved → onboarded` (+ `rejected`).
Two transitions create a waiting human:

1. **Candidate submits → `proposed`.** The **sponsor** (recorded on the proposal as
   `sponsor`) now needs to review + vouch. → notify that sponsor.
2. **Sponsor vouches → `sponsor_approved`.** An **admin** now needs to review + provision.
   → notify the admin group.

(`approved → onboarded` is done by the runner automatically; no human wait, but a
"minted / failed" note is a nice-to-have.)

## Increment 1 — in-portal count + self-refresh — BUILT 2026-08-11, in the CONSOLE

Built as described, in the console rather than the portal (the queue moved
there) — see "Built: reviewer notifications, queue badges, queue age" above for
what shipped and how the counts endpoint is gated. The original plan:

The cheap win: make the queues self-evident whenever someone is in the portal.

- **Pending-count endpoint.** Add `GET /api/onboard/proposals/counts` → `{proposed: N,
  sponsor_approved: M}` (a cheap `SELECT status, count(*) … GROUP BY status`). Operator-
  gated like the list. (Or the pages can just read the lists they already fetch and count
  client-side — simplest, no new endpoint; the endpoint is only worth it for the nav badge
  on pages that don't already load the list.)
- **Nav badges** on the portal `index.html` Admin group: "Sponsor review (N)",
  "Onboarding queue (M)". Poll the counts endpoint every ~20 s; hide the badge at 0.
- **Self-refresh** the two queue pages: `setInterval(loadList, 20000)` (preserve the
  selected row + scroll). Turns "remember to check" into "the number is right there."
- **Scope:** static HTML/JS + one small Go handler. One portal image + deploy. ~1–2 hrs.
- **Limit:** only reaches someone already looking at the portal. That's what increment 2 is for.

## Increment 2 — push to chat — the EMAIL leg is built; chat is still open

The reviewer notifications now go out by **email** from the console (above), so
the two trigger points and the deep-link wording are settled and proven. What
this section still describes is the *chat* channel, which remains gated on the
operator's decision below. Building it means adding a second delivery leg at the
same two call sites, not re-deciding what to say.

Reach a human who isn't watching the page. The fleet already runs **Matrix (tuwunel +
hedwig)** and an **ZAGROS Zulip** (`chat.zagros`, Ory SSO) — reuse one, don't build a notifier.

- **Trigger points (portal, server-side):** on `handleOnboardSubmit` success (→ sponsor
  notify) and on `handleSponsorApproveProposal` success (→ admin notify). Post a short
  message with a **deep link** to the review page + the handle/env + the escalation/petition
  flags (so an escalated or change-requested candidate is visibly flagged in chat, matching
  the in-page badges).
    - *"New candidate awaiting sponsor review: `jane-doe` (zagros) → &lt;link&gt;"*
    - *"Ready for admin provisioning: `jane-doe` (zagros, ESCALATED) → &lt;link&gt;"*
- **Targeting:** the proposal records `sponsor`, so the sponsor message can @-mention /
  DM that specific sponsor; the admin message goes to a shared onboarding room/stream.
- **Delivery, keeping the portal clean:** the portal is mesh-only + holds no chat creds
  today. Prefer a **fire-and-forget POST to a webhook** (a room/stream incoming webhook, or a
  tiny relay) over embedding a chat SDK — a webhook URL in portal config, best-effort, never
  blocks the submit/approve response, logs on failure. Mirrors how other on-fleet posts work.
- **Config:** `PORTAL_NOTIFY_WEBHOOK_URL` (+ optional per-role room override) in the portal
  role env; feature-flag off by default so no-webhook envs are a no-op.

### The one decision that's genuinely the operator's

Before building increment 2, pick:
1. **Channel** — Matrix room vs Zulip stream (Zulip is already SSO'd at `chat.zagros` and
   stream/topic-shaped, which fits "onboarding / <env>" well; Matrix is the sovereign default).
2. **Room model** — one shared "onboarding" room/stream for both gates, vs per-sponsor DMs
   for the sponsor notification + a shared admin room. (Recommend: shared stream with a
   per-env topic + @-mention the sponsor, rather than DMs — discoverable + auditable.)

## Increment 3 — candidate-facing email at every stage (operator-requested 2026-07-08)

**Status: BUILT in code (2026-07-08), deploy + live-fire pending.** Console:
`POST /api/notify` (shared `NOTIFY_TOKEN` bearer — server-to-server, the portal
can't present oauth2-proxy group headers) + the four templates below +
generalized `mailer.Send`. Portal: `internal/notify` fire-and-forget client +
hooks at submit / sponsor-approve / both rejects / runner-result, config
`PORTAL_NOTIFY_CONSOLE_URL` + `PORTAL_NOTIFY_TOKEN` (off by default). Runner:
parses the NetBird setup key from the mint output and carries it in the
`/result` writeback (`setup_key`), so the final email can include it — never
persisted on the proposal. Ansible: console role NOTIFY_* + portal role
PORTAL_NOTIFY_* env, ZAGROS group_vars wired to `ZAGROS_ONBOARD_NOTIFY_TOKEN`.

**Companion shipped in the same change — durable audit interim ("v1.5" of ADR
0860's v2):** the console's audit page now merges the PORTAL's durable
`audit_events` (new operator-gated `GET /api/audit/events`) for onboarding
actions — the local ring is per-container, so a redeploy empties it; and console-NATIVE actions (the
Kratos mutations only it performs) get a durable append-only JSONL sink
(`AUDIT_BACKEND=file`, bind-mounted, survives redeploys). No new backend
service; full v2 (evidence-grade retention) remains gated on a compliance
driver and should ride the existing MinIO Object-Lock tier, not Postgres.

Increments 1–2 tell the *reviewers* work is waiting. Increment 3 tells the
**candidate** where their onboarding stands — they are the one person with no
portal access and no chat account, so email is the only channel that reaches them
(the same reasoning that put the invite on SES).

**The four sends** (candidate email comes from the submission; it is on every
proposal):

1. **Submit → `proposed`.** "Your submission is in — it's with your sponsor for
   vouching. 2 approvals remain." Sent from the submit handler after the proposal
   is stored (same place increment 2 posts the sponsor webhook).
2. **Sponsor vouches → `sponsor_approved`.** "Your sponsor vouched for you — now
   with the admin for final approval. 1 approval remains."
3. **Rejected** (either gate). "Your onboarding was not approved" + the gate +
   any reviewer note. (Completeness — don't leave a candidate waiting forever.)
4. **Runner reports `onboarded`.** The **slim bundle email** — deliberately thin
   because everything else is downloadable once they're on the mesh:
   - the NetBird setup key + the one-line `netbird up --management-url … --setup-key …`
   - the mesh download surface (`get.<site>…`) for goat-cli, and the portal URL
   - the Ory login pointer
   - a short **"can't reach the mesh?"** self-service recovery block — the
     `netbird status` → one clean `down/up` → re-enroll-with-your-key ladder,
     plus the "second device on the same network" is-it-me-or-the-network test.
     This is the one thing a **locked-out** user actually needs and can't fetch
     from a mesh-only surface (isitup/portal/relay-health all require the mesh
     that's failing them). Mirror the canonical copy in the operator-onboard
     bundle template (`ops/ansible/templates/operator-onboard/bundle.md.j2`,
     "If you can't reach mesh services") — keep the two in sync.
   - explicitly NOT in the email: certs and their private keys — those stay on
     the mesh-only bundle URL (`/api/org-bundle/…`, valid for `PORTAL_BUNDLE_TTL`, 24 h by default), which the
     email references as "fetch after you're on the mesh". Email is
     plaintext-hostile territory; the setup key is single-purpose, quota-capped
     (usage_limit), and revocable, so it is an acceptable email payload.
   - **AMENDED 2026-08-06 (increment 7): the portal API key IS in the email.**
     This rule originally said "certs/api_keys" together. Holding the portal key
     to the cert standard did not keep it off email — it kept it away from the
     *enrollee*, who then obtained it by a worse route. See increment 7.

**Where it runs.** The transitions fire in the **portal** (submit / vouch / approve
/ runner-result handlers), but the SES creds + mailer live in the **id-console**.
Options: (a) portal grows its own copy of the tiny mailer + SES creds via the
ADR 1070 keystore-dir pattern; (b) portal POSTs a fire-and-forget notify to the
id-console, which owns all mail. **Recommend (b)** — one mail-owning component,
one credential home, and the portal keeps zero SMTP surface; it is the same
fire-and-forget-webhook shape as increment 2, just aimed at the console instead
of chat. Best-effort always: a failed send never blocks the transition; log +
audit-event the failure.

**Config:** console `SMTP_*` env from the SES terraform outputs (wire into the
id-console role + group_vars; secret via the operator secret set); portal
`PORTAL_NOTIFY_CONSOLE_URL` feature-flag, off by default.

**Step counting:** derive "N approvals remain" from the state machine
(`proposed`=2, `sponsor_approved`=1, `approved/onboarding`=0 — mint in progress),
never hardcode in templates.

## Increment 4 — role-change (escalation) email — LIVE on ZAGROS, live-fired 2026-07-17

**The trigger is one action: saving a group change on the console user page.**
There is no separate "send email" step — an admin edits the identity's login
groups, clicks "Save groups", and the person is emailed. One save = one email;
a save that changes nothing sends nothing; an add-then-revert (two saves)
sends two emails. Live-fired 2026-07-17 on a test identity: a group was added
and then reverted via the console patch endpoint, both `access_change_email`
audit rows returned `ok`, and both emails delivered (an "access upgrade" and
an "access change").

A group patch in the console (`handleUserPatchGroups` — the in-band role-change
lever, e.g. developer → platform-admin) now emails the person whose access
changed. Two requirements, both explicit in the template:

1. **Upgrade ≠ new provisioning.** The email opens by acknowledging the change
   applies to their EXISTING identity — nothing re-provisioned; mesh enrollment,
   login password, CLI profile, and fabric bundle keep working. Already-done
   onboarding steps render as "(already done)", and the one actionable nuance is
   stated: sign out/in so a fresh session carries the new groups claim.
2. **Still the complete package.** The body is the full tier-correct reference,
   rendered by the same `internal/welcome` section code as new-enrollee
   provisioning (`welcome.RenderUpgrade`), so the two paths cannot drift.
   Console-plane capabilities (admin console + operator-setup sections) are
   recomputed from the NEW group set; mesh/fabric capabilities come from the
   onboarded proposal when one exists — never fabricated.

Mechanics (`ops/goat-id-console/internal/handlers/access_change.go`):
`sendAccessChangeEmail` fires only on an effective group change, tenant realm
only (obs-IdP group flips notify nobody). No one-time secrets — no setup key, no
recovery link; the person already has a password. Best-effort by contract: a
send failure never fails the patch; every attempt is audited as
`access_change_email` with the group before/after. The subject says "upgraded"
only when groups were added; a pure reduction is an "access change".

Addendum: admin-tier welcome packages (fresh and upgrade) additionally carry an
"onboarding other people — the two doors" section — full onboarding (Invite)
vs bare login (New user), the welcome email fires on admin approval (not
invite send), and the mesh-role vs login-group vocabulary split — so a new
admin learns the distinction from their own onboarding email rather than from
a stranded batch. The guide section links the guide's `identity-admin` topic
so the email stays sufficient on its own while the long-form reference has a
stable home. It links the GUIDE HOST, not the console: the console's own
`/docs/onboarding` copy sat behind the sponsorship gate, so a good half of the
people told to bookmark it could not open it. Where a goatnet has no guide
host, the pointer falls back to the console's own copy of the same handler
(`/public/onboarding-guide`) rather than rendering nothing. A catch-up variant exists
for people whose access predates these emails: the "Email role reference"
console action / operator API sends the complete current-role reference with
explicit nothing-changed-now framing (audited `access_reference_email`).

## CLI/web parity — the role change is one behaviour, two front doors

Increment 4 shipped the role-change email on the **web** save path. But the same
grant driven from the **CLI / `operator-onboard.yml`** reconciled Ory groups
directly and fired no email, applied no last-admin guard, and applied no step-up
policy — so "grant platform-admin" behaved differently depending on which door
you used. That drift is closed by making both doors funnel through one core.

- **One shared core.** `applyGroupChange`
  (`ops/goat-id-console/internal/handlers/group_change.go`) is now the single
  implementation of a login-group patch: vocabulary fence → last-admin guard →
  step-up policy → Kratos patch → `patch_groups` audit → best-effort
  role-change email. `handleUserPatchGroups` (web) is a thin wrapper over it.
- **Machine siblings of the web routes.** `POST /api/users/groups` and
  `POST /api/users/state` (`users_api.go`) are the operator-key equivalents of
  `POST /users/{id}/groups` and `/state`. They resolve the identity by email
  (the dedup key), run the same core / patch, and attribute the change to the
  operator handle in the audit. Step-up (AAL2) is deliberately **not** applied
  on the machine path — the operator key is itself the authorization (ADR 1071,
  operator keys uncapped); the audit records `step_up_verified=false` with the
  operator handle, so a platform-admin grant via `goat admin` is fully
  attributed, never silent. The web path keeps its AAL2 session requirement.
- **The CLI face.** `goat admin role set --email <e> --groups <csv>` (plus
  `reactivate` / `deactivate`) calls those endpoints
  (`crates/goat-cli/src/cmd/admin.rs`, `PortalClient::patch_user_groups` /
  `patch_user_state`). `--groups` is required so an omitted flag can never
  silently clear someone; escalated groups warn locally and are attributed.
- **Keep-status stays where it was.** The catch-up "complete reference" email is
  still `/api/onboard/email-reference` (operator-key) and the console action;
  net-new people still flow through the candidate `notify.go` `onboarded` email.
- **Invite delivery got the same treatment.** The machine mint
  `POST /api/onboard/invite` previously created the invite + link but had no
  email step — only the session-gated web page (`/onboarding/invite/email/send`)
  could deliver it, so an operator-key/automated mint stranded the link. The
  mint now takes an optional `email`: when set it delivers the one-time
  nomination link via the same `Mailer.SendInvite` the web page uses
  (best-effort, audited `onboard_email_invite`, link still returned on failure).
  `goat admin invite --email <candidate>` exposes it. A machine-driven onboard
  is now self-contained end to end.
- **`step_up_verified` reflects reality.** The audit's step-up flag is set true
  only when AAL2 was actually enforced+passed (the web path). The operator-key
  machine path records `false` — the operator key is the authorization, no AAL2
  occurs, and the audit must not imply one.

Net: a role change through the console and the identical change through
`goat admin role` (or the onboard playbook, which calls the same endpoint) now
produce the same guard, the same audit shape, and the same email.

## Increment 5 — welcome-package clarity + accuracy pass (issue #152)

**Content-only rework of the `internal/welcome` renderer — no new infra, same
role/access gating, same pure formatter.** The previously generated package got
real enrollees stuck silently at almost every step and was wrong/stale in
places; the rewrite corrects each step against the failures operators actually
hit onboarding. Every concrete value still comes only from the caller's
`Catalog` / `Recipient` / `Artifacts` (setup key, recovery link+code, bundle
URL, handle) — nothing personal is baked into the code, guarded by
`TestRender_noHardcodedTemplateSecrets`.

What changed, by step:

- **Intro** — a plain-language "what is the mesh" paragraph (it's like a VPN;
  nothing loads until NetBird is connected), replacing the one-liner.
- **Step 1 (mesh join)** — per-OS install, Windows-no-`sudo` note, `service
  install`/`start`, and the silent-ignore trap made explicit: `netbird up` while
  connected to another mesh is silently ignored, so `netbird down` first; plus a
  `netbird status -d` verification whose "which mesh" check is templated from the
  configured `MgmtURL`. The keyless / no-mgmt degradation branches are unchanged.
- **Step 2 (login)** — the recovery **code's 1-hour expiry** and the
  "won't-load == it's the mesh, not the link" note, shown only when a code is
  present.
- **Step sequence** — the steps follow the reference order mesh, login,
  surfaces, CLI, fabric: **surfaces are STEP 3 and the goat CLI is STEP 4** (the
  two were swapped in the earlier package). The Sandbox cross-reference in Step 5
  tracks the surfaces step number.
- **Step 3 (surfaces)** — retitled "Your surfaces" (the SSO note moved out of
  the title into the body). The Portal is carved out of the SSO-password set with
  a role-aware note: operators use their OOB operator key, everyone else a scoped
  `vp_` key (the earlier package wrongly told everyone to use the Step-2 password
  on the Portal).
- **Step 4 (CLI)** — the PATH move (`sudo mv goat /usr/local/bin/` etc., the #1
  "command not found" cause), the macOS re-sign *reason*, Windows dual-download +
  checksum, `goat tutor`/`--list`, and "goat does NOT replace NetBird".
- **Step 5 (fabric)** — the "prove it works" demo is now actually runnable:
  unpack + `cd`, `pip install eclipse-zenoh protobuf` (the ModuleNotFoundError
  fix), subscriber-first ordering (publisher-only drill), `goat doctor`, and a
  private-key no-cloud-sync warning. Publisher vs read/subscribe gating retained.
- **Operator setup** — realigned to the reference layout: cert install
  (`mkdir`/`cp`/`chmod 400`), env with `export` + `exec $SHELL` + `echo` verify +
  a "what each is for" table, the everyday CLI plus the web-only Portal `.html`
  paths, and the optional onboarding-others workflow (mint a per-person NetBird
  token from the dashboard → clone `USAREUR-AF/goatcore` → roster recipients →
  `cp` the manifest example into the empty `ops/operators/<env>/` → `validate.py`
  → `goat admin onboard`), closing with the manifest field vocabulary. The SSH
  login is corrected from the wrong per-handle form to the SHARED operator
  account (new optional `Recipient.MgmtSSHUser`; blank → generic instruction,
  never a fabricated per-handle login, always with the no-attribution caveat),
  and the stale CLI verbs (`invites ls|revoke`, `queue …|sponsor-approve`) are
  replaced with the current surface (`invite`, `sponsorship … --max-tier`). A new
  `Catalog.Dashboard` surface (derived like the others) feeds the NetBird-token
  step. The `ZAGROS_`-prefixed env-var names remain literal (matches the reference
  for ZAGROS; see the latent-limitation note below for non-ZAGROS goatnets).

Repository access is deliberately **not** hardcoded — the renderer faithfully
lists whatever `RepoGrants` it is given, so any "missing repo" is a
provisioning-data gap (what was recorded at grant time), not a renderer bug.

Tests live alongside the code (`internal/welcome/welcome_test.go`): the two
assertions that had pinned the *old buggy* behavior (stale `queue` verb,
per-handle SSH) were updated to the corrected forms, and new tests cover each
step's additions plus the no-personal-secrets guard.

**Deliberate divergences from a single hand-written reference** (the generator
is tier-general, multi-goatnet, and serves both new + upgrade sends, so it does
NOT byte-match one person's static email):

- **Conditional sections stay conditional** — browse/observer/developer never
  get the admin console surface, admin-onboarding guide, or operator setup; the
  Portal note swaps operator-key vs `vp_` by role; the fabric step is publisher
  vs read/subscribe; upgrade mode reframes each step as "already done".
- **Dynamic values are templated, never frozen** — every URL, the handle, the
  bundle link, recovery link+code, and the setup key come from
  `Catalog`/`Recipient`/`Artifacts`; missing values degrade to an instruction
  rather than a fabricated literal.
- **Goatnet display name** is uppercased for the title / mesh prose / email
  subject (e.g. `zagros` → `ZAGROS`) via `welcome.DisplayName`, the single source of
  truth shared by the body and the subject. The lowercase config value stays the
  `env` identifier for paths and `--env`.
- **Shared mgmt SSH account** renders concretely only when `MgmtSSHUser` is
  wired from per-goatnet config (not yet populated in `recipientFromProposal`);
  until then it degrades to the generic shared-account instruction. This keeps
  the per-goatnet account name out of the code, consistent with the fail-closed
  "config, not hardcoded default" principle.
- **Manifest field vocabulary** is reproduced compactly for operator
  convenience; `ops/operators/validate.py` remains the enforced source of truth,
  and the email points at it — if the grammar changes, update both.

**Latent limitation (NOT a divergence from the reference):** the operator env-var
names `ZAGROS_PORTAL_OPERATOR_API_KEY` / `ZAGROS_NB_API_TOKEN` are emitted as literal
`ZAGROS_`-prefixed strings, so the ZAGROS email matches the reference exactly — but
the same literals render for *every* goatnet, which is wrong for a non-ZAGROS one.
This predates this change (the old operator section already hardcoded `ZAGROS_`)
and is left as-is: templating the prefix from the goatnet needs slug sanitizing
(a hyphenated slug is not a valid shell variable name), which is out of scope
for this issue. Fix when the generator first serves a non-ZAGROS operator email.

## Increment 6 — never send an incomplete welcome package

**Why the keyless branch existed at all.** It is correct for manifests that
never mint a key (login-only / fabric-only). The defect was applying it to a
mesh-enrolled person, where it is not a degradation but a dead end.

**The two changes.**

1. **Hold, don't degrade.** `sendWelcomeEmail` now refuses to send when the
   manifest has `netbird.enabled` and no setup key is in hand: it logs a WARN and
   records the audit event with result `held` plus the reason, instead of `ok`. A
   held send is visible; a keyless send looked successful. The predicate is
   `needsSetupKey` (manifest-driven, unit-tested) — non-mesh manifests are
   unaffected and still send immediately. A malformed manifest sends rather than
   holds, so a parse failure can't wedge delivery on something invisible.
2. **A resend path.** `POST /onboarding/{id}/welcome/resend` (onboardGate,
   four-segment route so it stays distinct from the `{action}` decide route)
   re-renders and mails the package via `welcome.RenderReissue`. An optional
   `setup_key` field supplies a key the console never learned and **persists** it,
   so the stored proposal and every later render agree. It refuses (409) for a
   mesh-enrolled proposal with no key from either source, naming the bundle path
   where the key lives.

**The reissue banner.** A second welcome package landing unannounced reads as a
second identity or a phishing attempt, and the recipient cannot tell which copy
to follow. `RenderReissue` prepends a banner **above the title line** —
supersedes-the-earlier-email, not a new account, not an access change, discard
the earlier one, already-completed steps need no redoing — and, when a key is
present, names the key as what was missing. `Render` (first send) is unchanged
and banner-free; `TestRender_freshPackageHasNoRepeatBanner` holds that line.

**Note on persistence.** Increment 3 above says the setup key is "never persisted
on the proposal." That has not been true since the `setup_key` column landed (it
backs the welcome download); the resend path now also writes it. The key is
single-purpose, quota-capped, and revocable — the same reasoning that makes it
the acceptable email payload makes it acceptable at rest on the proposal.

## Increment 7 — the welcome email carries the portal API key

**Why it surfaced only now.** Nothing regressed. Before the console cutover
(2026-07-12) operators hand-delivered bundles, and the key rode along inside one.
On the automated rails the email is the delivery vehicle, and it never carried
the key — so the more the team used the intended path, the more people arrived
without one. A gap that had always existed became visible when its accidental
workaround went away.

**The damage the workaround did.** Admins closed the gap with `goat admin
rotate`, which re-issues the cert in place and returns a NEW `api_key`. Every
such rescue silently invalidated the key sitting in that person's own bundle —
producing the support thread's *"the vp_ from the .cbor file does not work"* and
*"I received an email with a new vp_ key that does not match my bundle"*. The
manual fix was destroying the artifact people were being told to dig through.

**The chain, now closed.** `goat admin onboard` prints `api_key: vp_…` → the
runner scrapes it (`portalAPIKeyRe`, anchored on the `vp_` shape so it can never
capture an adjacent field) into `mintResult` → the `/result` writeback carries
`portal_api_key` → `sendWelcomeEmail` passes it into `welcome.Artifacts` → the
Portal section renders the key inline.

**Three Portal branches, all honest:**
- **operator** — ~~unchanged: operators use an OPERATOR key delivered out of
  band, so the inline branch must not hijack them.~~ **CORRECTED 2026-08-11 —
  see "The admin branch withheld the admin's own key" below.** That rule read as
  a careful carve-out and was a defect: an admin-tier onboardee has BOTH a
  scoped `vp_` key (which is what the Portal sign-in box takes) and an operator
  `op_` key (for the operator surfaces). Suppressing the first because they hold
  the second left a platform admin with nothing to paste.
- **has a key** — the key itself, inline, plus a pointer that it is the same
  value as the last line of the bundle's `goat-cli.cbor`, plus an explicit *do
  not ask for a rotation to get a fresh one — rotating makes this key stop
  working*.
- **no key** (no fabric leg) — says so plainly: the
  Portal will not admit you, that is expected, ask for an access change rather
  than a hand-issued key. Previously these people were sent hunting for a key
  that had never been minted.

**Storage.** Passed through for the email only, never persisted — the same
treatment as `collab_setup_link`, and unlike `setup_key`. It is broader and
longer-lived than the setup key, so it does not sit at rest on the proposal.
Consequence: the welcome *download* and the resend endpoint do not carry it
unless an operator supplies it.

**Amends increment 3.** The original "explicitly NOT in the email: certs/api_keys"
bundled two very different secrets. The cert's private key still never goes near
email. The portal API key is a scoped, revocable, rotatable bearer token whose
entire purpose is to be pasted into a sign-in box by its holder; withholding it
from email did not protect it, it just routed people through an admin who then
rotated it.

## Increment 8 — split the package: credentials by email, walkthrough on a page

**The problem.** The welcome package carries two kinds of content with opposite
shelf lives: a handful of one-time secrets, and a walkthrough read once. Mixing
them buried the secrets — enrollees hunting for a setup key were scrolling past
per-OS install instructions to find it. Increments 6 and 7 fixed *whether* the
credentials were present; this one fixes whether they can be *found*.

**The cut line is forced, not chosen.** Everything on the guide page is
reachable only once the person is on the mesh. Step 1 — installing NetBird and
joining — can never move there, because until it is done the page does not load.

| | |
|---|---|
| **Email** | identity + environment + mesh join + the credentials + a link to the guide |
| **Guide** | CLI install, the surfaces, publishing, operator setup, troubleshooting |

**Why not the docs site.** `docs.<site>` serves a **pinned goat-cli release
tarball**, so onboarding content hosted there would only change when the CLI
cuts a release — the wrong cadence for content that changes when onboarding
changes. The console already knows the per-goatnet URLs and the tier vocabulary,
so it renders the page itself.

**Why it needs no login.** The page carries no credential *values* and is
identical for everyone at a tier, so there is nothing on it to leak between two
people who can both already reach the mesh. **Reachability is the control**: it
is published on a mesh-only host behind `mesh-only-allowlist`. Anything
person-specific stays in the email. The test enforcing this matches the *shape*
of each secret (`vp_` keys, UUID setup keys, one-time bundle tokens, recovery
links and codes) rather than the vocabulary — naming a credential is necessary
("paste your portal API key"); carrying one is the defect.

**Its own host, not a path on `onboard.<site>`.** That host is deliberately
confined to the machine realm (`PathPrefix(/api/onboard/)`, bearer-gated
in-app), with human surfaces kept behind `admin.auth.*`. The guide is a human
page, so it gets `guide.<site>.<zone>` with its own `PathPrefix` confining the
host to exactly that page. DNS comes from `dns_extra_hosts` in each goatnet's
group_vars, which regenerates the mesh zone (F-197 keeps declared-but-
unresolvable names from deploying dark).

**The email after the split** (developer, ZAGROS): 136 lines → **85**. What
survived is exactly the set a person cannot get anywhere else — the mesh-join
command with their setup key, the login link, the portal API key, and the
one-time fabric bundle URL (which keeps its own heading and its "fetch this
first, the link expires" framing, because moving the walkthrough must not move
the expiring link with it). What left is everything that needs the mesh anyway:
the surfaces tour, the CLI install, the publishing walkthrough, the
onboard-others and operator sections.

**Config-gated, so rollback is a config change.** `Catalog.Guide` blank → the
email renders every section inline exactly as before. A goatnet that has not
deployed the guide is unaffected, and reverting is unsetting a derived URL
rather than reverting code. The pre-existing test suite was re-pointed at that
no-guide catalog, so it still guards the full-email contract, and the slim path
got its own tests.

**Two things the tests caught that review would probably not have.** The derived
guide URL was initially the bare host, which would have 404'd against a router
pinned to `PathPrefix(/public/onboarding-guide)`. And Step 2 told the reader
their password worked for "every web surface **below**" — true until the
surfaces moved, then a dangling reference to a list no longer in the email.

**Shipped in three ordered changes**, so the email never points at something
that is not yet reachable: the page (inert on its own), then the routing and
DNS, then the email slim.

## Increment 9 — the slot→person linkage (ADR 1051), finally sent

**The portal was never the problem.** It accepts `identity_id` when a slot is
created, validates the UUID, stamps it via `StampSlotIdentity`, exposes an
operator-gated audited `PUT /api/slots/{anchor}/identity` to set it after the
fact, and logs a WARNING on every unlinked onboard:

> slot onboarded with no recorded identity linkage — ownership for this slot
> will be derived by name join (alias↔handle), the fallback the linkage exists
> to retire

That warning had been firing unread. **The break was the sending side:** the
Rust portal client's `NewVendor` has no `identity_id` field, so the native mint
could not send what the server was waiting for.

**Why the linkage is written after the mint, not during it.** The slot is
created before the Kratos identity exists: the native mint runs NetBird, then
Zenoh, then Ory. So the stamp happens once both halves exist. (Note for anyone
reasoning about that order: for a FABRIC onboard an Ory failure is not a
"partial" — it rolls the Zenoh slot back and bails, so the onboard is atomic.
The "partial" outcome applies to onboards with no slot to orphan.)

**Both paths stamp, deliberately.** The console stamps on its `/result`
writeback (covering console-driven onboards); the CLI stamps after its Ory leg
(covering an operator running `goat admin onboard` directly). Fixing only one
leaves the other silently producing unlinked slots — which is precisely how the
existing gap accumulated. `PUT` is idempotent, so both firing is a no-op and a
race is harmless.

**Fail-soft, and refuse-to-guess.** A stamp failure costs an
inferred-by-name-join ownership lookup — today's behaviour — and never fails an
onboard that otherwise succeeded. But when an email resolves to MORE THAN ONE
Kratos identity, both paths refuse rather than pick: a wrong binding asserts
that one person owns another's namespace, and ownership is what revocation
targets. No linkage beats a wrong one.

**Consumers this unblocks.** Ownership questions ("which slot is this person's")
stop depending on a name join — the gap that left microscope unable to connect
an email to a slot. It is also a prerequisite for mint-on-collect
(`docs/project/tee-ups/fabric-credentials-mint-on-collect.md`), where the
console must resolve a logged-in person to their slot to collect on their
behalf.

**Remaining:** the unlinked person and machine slots need a
backfill, dry-run first — a bad email→identity match there is the wrong-owner
failure this design refuses to risk at mint time.

## Increment 10 — role-differentiated guide, layered depth, worked examples

**What was wrong with increment 8's guide.** It was ONE document with sections
hidden by tier, which is not the same as documenting a role's process. Two
consequences, both real: `iam-admin` and `platform-admin` rendered
byte-identical pages, and the **site network admin — a role with its own rules
(ADR 1113) — had no page at all.**

**Five roles, each with its own path.** browse / developer / identity admin /
platform admin / network admin. A role owns an ordered sequence in its own
words; the index stays light and continuous. Network admin is a FIFTH VIEW
rather than an overlay: the entitlement is orthogonal to onboarding tier in the
grammar, but a reader wants one page about their job.

**Depth is one click away, never inline.** Nine topics (`credentials`, `cli`,
`surfaces`, `fabric`, `identity-admin`, `operator`, `network`, `gotchas`,
`repos`, `troubleshooting`) reachable at `?role=…&topic=…`. Every topic is
readable by every role — the page is unauthenticated and credential-free, so
the role list is about RELEVANCE, not restriction, and a platform admin reading
the browse path is a good outcome.

**Time-sensitivity is a first-class element, in both surfaces.** A `Deadline`
renders as its own marked block, never as another sentence in a paragraph — an
expiry buried in prose is an expiry someone reads after it has passed. Every
deadline states its recovery, so a missed window does not read as a locked door.
The EMAIL now hoists all of them into a `!! TIME-SENSITIVE` block ABOVE the
steps, because a reader who skims reaches step 5 well after a 15-minute window
has closed. It renders only when the package actually carries something
expiring, so it never cries wolf.

**A worked-example catalogue that survives renames.** Nine hello-galaxy repos,
mapped to the roles each is worth. Entries carry a SLUG and a description, never
a URL: repositories get renamed and move between organisations — a live
migration here — and a page of hardcoded links rots into a page of 404s that
teaches readers the docs are stale. Links are composed at render time from
`GUIDE_REPO_BASE_URL`; unset renders names WITHOUT links plus instructions to
ask. A standing note says access may need requesting and names may have moved.

**An anti-patterns topic.** The traps that have already cost days: rotating to
hand somebody a key (it invalidates the copy in their own bundle), guessing who
owns a namespace by recognising a handle, reading a withheld email as a delivery
failure, and creating identities by hand to move faster.

## What is left (updated 2026-08-11)

1. **Deploy + live-fire.** Set `ONBOARD_ADMIN_NOTIFY_EMAILS` in the goatnet's
   group_vars, redeploy the console, and walk one candidate end to end on ZAGROS:
   submit → sponsor mail lands → vouch → admin mail lands → reject a throwaway
   and confirm the candidate mail. Read `/audit` for the three
   `onboard_email_*` rows — a `result: error` there is the honest signal that a
   notification was owed and not delivered.
2. **Confirm the chat channel + room model** with the operator, then add chat as
   a second leg at the same two call sites (increment 2).
3. **The runner-result note** ("minted / failed") for the admin who approved —
   still a nice-to-have, still unbuilt.
