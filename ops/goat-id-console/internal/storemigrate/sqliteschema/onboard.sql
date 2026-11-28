-- onboardstore schema — the persistent store for the three-stage delegated
-- candidate onboarding (ADR 1062/1071) plus the console's separate durable
-- self-service queues, including probation reactivation requests (ADR 1089).
-- The org/slot/cert registry stays in goat-zenoh-portal.
--
-- Backed by jmoiron/sqlx over modernc.org/sqlite (pure-Go, no CGO). All SQL is
-- raw, no ORM. SQLite opened WAL; writes serialize at the file lock — trial
-- onboarding throughput (handfuls per hour) is comfortably inside that.

-- onboard_proposals is the inert pre-mint queue (ADR 1061). A proposal mints
-- nothing; a sponsor vouches (stage 2) then an admin approves (stage 3), at
-- which point the runner writes the manifest and runs `goat admin onboard`.
CREATE TABLE IF NOT EXISTS onboard_proposals (
    -- Opaque random id (prop-<hex>), assigned at submit.
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    -- The authenticated submitter who vouches for this onboarding:
    -- "<operator-handle>" for an operator, "<prefix>" for a participant.
    sponsor         TEXT NOT NULL,
    -- Human-readable label for sponsor (a participant's sponsor is its UUID
    -- org anchor; this is the portal display_name resolved at mint). Display
    -- only — provenance/scoping match on sponsor, never on this.
    sponsor_name    TEXT NOT NULL DEFAULT '',
    -- Target goatnet — selects ops/operators/<env>/.
    env             TEXT NOT NULL DEFAULT '',
    -- Denormalized from the manifest for listing / de-dup / display.
    handle          TEXT NOT NULL,
    email           TEXT NOT NULL DEFAULT '',
    -- Lifecycle: proposed | sponsor_approved | approved | rejected |
    -- onboarding | onboarded | failed, plus the offboard leg (goat#372):
    -- offboard-requested | offboarding | offboarded | offboard-failed.
    status          TEXT NOT NULL DEFAULT 'proposed',
    -- The validated manifest object, JSON-encoded (the onboard input).
    manifest_json   TEXT NOT NULL,
    -- Last lint verdict (JSON), so the admin view shows it without re-linting.
    lint_json       TEXT NOT NULL DEFAULT '{}',
    -- Operator who approved/rejected + when + free-text note. decided_by is
    -- stamped at EACH decision, so on an onboarded row it is the FINAL admin
    -- approver (the sponsor-stage actor lives in sponsor_approved_by).
    decided_by      TEXT,
    decided_at      INTEGER,
    note            TEXT NOT NULL DEFAULT '',
    -- Stage-2 provenance: who recorded the sponsor vouch (an admin may act at
    -- the sponsor gate, so this can differ from sponsor) + when. '' / 0 on
    -- rows that predate the column or never passed the sponsor gate.
    sponsor_approved_by TEXT NOT NULL DEFAULT '',
    sponsor_approved_at INTEGER NOT NULL DEFAULT 0,
    -- 1 for a retroactive import: the person was onboarded BEFORE the
    -- three-stage system existed, so provenance fields (sponsor, approvals)
    -- were never recorded — the UI explains rather than showing blanks.
    retroactive     INTEGER NOT NULL DEFAULT 0,
    -- Carries an escalated admin tier (iam-admin/platform-admin) → the admin
    -- queue flags it for extra scrutiny (ADR 1062 Amendment 1).
    escalated       INTEGER NOT NULL DEFAULT 0,
    -- 1 when the manifest is complete + approvable; 0 for a minimal-burden draft
    -- (blank tier/affiliation) the admin completes.
    complete        INTEGER NOT NULL DEFAULT 1,
    -- JSON array of enrollee petitions to change a sponsor-locked field
    -- (field/sponsor_value/requested_value) — recorded + flagged, never
    -- auto-applied (ADR 1062 Amendment 1). '[]' when none.
    petitions       TEXT NOT NULL DEFAULT '[]',
    -- The effective tier the proposal carries ('' for a draft with no tier
    -- yet), denormalized so the stage-2 sponsor gate can compare it against the
    -- approver's sponsorship cap without re-parsing the manifest (ADR 1071).
    tier            TEXT NOT NULL DEFAULT '',
    -- The NetBird enrollment setup key the runner minted (written back on the
    -- onboarded result). Persisted so the welcome package can hand it over even
    -- when email delivery is unavailable — otherwise it is shown once and lost.
    setup_key       TEXT NOT NULL DEFAULT '',
    -- The fabric bundle's mesh download path (/api/org-bundle/<id>/<tok>.tar.gz)
    -- the runner reports on the onboarded result; the welcome package renders it
    -- as a mesh link (portal base + this path) so the enrollee pulls their own
    -- cert bundle over the private mesh — no operator hand-off.
    bundle_path     TEXT NOT NULL DEFAULT '',
    -- JSON array of the GitHub repo-collaborator grants attempted at provision
    -- ({owner, repo, permission, outcome}), recorded fail-open so the welcome
    -- package can surface which repos the person was added to
    -- (docs/design/onboarding-github-repo-grants.md). '[]' when none / feature off.
    repo_grants     TEXT NOT NULL DEFAULT '[]',
    -- JSON object of per-onboardee repo-grant OVERRIDES (the "audible" layer on top
    -- of the on-rails policy engine): {add:[{repo,permission}], remove:[{repo}]}.
    -- Set at sponsor-approve / admin-approve / provision to add a repo the policy
    -- wouldn't grant, or drop one it would. The effective set = policy ⊕ overrides
    -- (docs/design/onboarding-github-repo-grants.md §2). '{}' when none.
    repo_overrides  TEXT NOT NULL DEFAULT '{}',
    -- Collaboration-environment account the runner provisioned (collab.enabled),
    -- recorded read-only for mesh-plane AWARENESS — the mesh tracks who has collab
    -- access but never governs it. '' when not provisioned. The one-time setup link
    -- is deliberately NOT stored (credential-bearing; used once for the welcome
    -- email). docs/design/collab-onboarding-provisioner.md.
    collab_account  TEXT NOT NULL DEFAULT '',
    -- 1 once the collab account was provisioned/activated best-effort; 0 otherwise.
    collab_activated INTEGER NOT NULL DEFAULT 0,
    -- Where affiliation.id came from: 'sponsor' (the invite pinned a registered
    -- org at mint), 'typed' (derived from the organisation the candidate wrote),
    -- 'operator' (an admin attached or created it at review). '' on rows that
    -- predate the column, and those are read as a candidate's claim: the review
    -- page asks, because approving an unchecked claim provisions a person into
    -- an organisation's roster, revoke cascade and data-sharing decisions.
    org_source      TEXT NOT NULL DEFAULT '',
    -- Who brought an `archived` row back to `approved` (RestoreProposal) and
    -- when; the mark that tells a restored person from a fresh approval. '' / 0
    -- when never restored; a later restore overwrites.
    restored_by     TEXT NOT NULL DEFAULT '',
    restored_at     INTEGER NOT NULL DEFAULT 0,
    -- Who queued the REAL cut (offboard-requested, goat#372) + when. The
    -- first requester wins; the runner carries the handle into the revoke.
    offboard_requested_by TEXT NOT NULL DEFAULT '',
    offboard_requested_at INTEGER NOT NULL DEFAULT 0,
    -- The requester's Kratos identity id, from the authenticated session
    -- ('' when the session carried none); names the approver in the runner's
    -- admin.offboard operator-action record (goat#1910).
    offboard_requested_by_identity TEXT NOT NULL DEFAULT '',
    -- The runner's operator-action outcome for the cut: 'recorded', or
    -- 'failed: <why>' when the admin.offboard record did not reach the audit
    -- fabric or names no approver. '' before a runner reports one.
    offboard_operator_audit TEXT NOT NULL DEFAULT '',
    -- The departure the cut records (ADR 1272): reason class, text, day.
    offboard_reason_class TEXT NOT NULL DEFAULT '',
    offboard_reason_text TEXT NOT NULL DEFAULT '',
    offboard_effective TEXT NOT NULL DEFAULT '',
    -- When a runner claimed this row (onboarding/offboarding); the requeue
    -- sweep's anchor for a claim a dead runner abandoned (goat#977). 0 when
    -- unclaimed or requeued.
    claimed_at      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS onboard_proposals_status_idx ON onboard_proposals(status);
CREATE INDEX IF NOT EXISTS onboard_proposals_env_idx    ON onboard_proposals(env);

-- onboard_invites is the "presigned one-time link" store (ADR 1062). A sponsor
-- (authenticated) mints an invite that PRE-BINDS the access level (tier/
-- affiliation/site/machine-quota); the candidate opens the public one-time link
-- and fills only their own identity. On submit the token is consumed
-- (single-use), a proposal is created, and the invite is dead. The token itself
-- is never stored — only its SHA-256 hash — so a database read never yields a
-- usable link. Single-use is enforced by durable state (consumed_at), not by
-- signature alone, so a replay after submit fails closed.
CREATE TABLE IF NOT EXISTS onboard_invites (
    -- Opaque random id (inv-<hex>), assigned at mint.
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    -- The authenticated minter who vouches for this candidate:
    -- "<operator-handle>" for an operator, "<prefix>" for a participant.
    sponsor         TEXT NOT NULL,
    -- Human-readable label for sponsor (see onboard_proposals.sponsor_name).
    sponsor_name    TEXT NOT NULL DEFAULT '',
    -- Target goatnet — selects ops/operators/<env>/.
    env             TEXT NOT NULL DEFAULT '',
    -- SHA-256 hex of the one-time token. The plaintext token is returned once
    -- at mint and never persisted.
    token_hash      TEXT NOT NULL,
    -- Pre-bound access binding, JSON-encoded {tier, affiliation, site,
    -- machine_quota, sponsor_max_tier, ...}. The candidate can NEVER set these —
    -- capability rides the server-held invite, never the public form (ADR 1062 §2).
    prebound_json   TEXT NOT NULL DEFAULT '{}',
    -- TTL; a link opened after this is 'expired' and fails closed.
    expires_at      INTEGER NOT NULL,
    -- Lifecycle: invited | submitted | expired | revoked | archived.
    -- `archived` is cold storage for a link that was already dead (revoked, or
    -- past its TTL) and has been for at least the sweep window. It changes
    -- nothing about usability — ConsumeInvite only ever accepts 'invited' — it
    -- moves the row off the working list.
    status          TEXT NOT NULL DEFAULT 'invited',
    -- Single-use marker (set at consume) + the proposal it produced, for
    -- provenance (which invite / which sponsor a proposal came from).
    consumed_at     INTEGER,
    proposal_id     TEXT,
    -- Free-text note (e.g. "for DEMO-ORG liaison officer").
    note            TEXT NOT NULL DEFAULT '',
    -- Delivery record. The address the invite was ADDRESSED to (typed at mint,
    -- or on a later send from the invite page); '' when the sponsor took the
    -- link and handed it over out of band, and on rows minted before this
    -- column existed. It is the intended recipient, not proof of receipt.
    candidate_email TEXT NOT NULL DEFAULT '',
    -- When the console last released the link toward that address, and how:
    -- 'email' (the console sent it) | 'eml' (composed .eml downloaded for the
    -- operator's own mail client — the console never saw it leave). NULL/''
    -- means the console never sent anything.
    sent_at         INTEGER,
    sent_via        TEXT NOT NULL DEFAULT '',
    -- When the invite was revoked, so "how long has this been dead?" can be
    -- answered for a revoked row the way expires_at answers it for an expired
    -- one. NULL on rows revoked before this column existed; the staleness
    -- helper falls back to expires_at for those.
    revoked_at      INTEGER,
    -- When the stale-invite sweep archived it. Archiving is terminal for an
    -- invite: the token is dead either way, and the way back is minting a new
    -- invite, not resurrecting a link somebody may already hold.
    archived_at     INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS onboard_invites_token_idx  ON onboard_invites(token_hash);
CREATE INDEX IF NOT EXISTS        onboard_invites_status_idx ON onboard_invites(status);
CREATE INDEX IF NOT EXISTS        onboard_invites_sponsor_idx ON onboard_invites(sponsor);

-- ssh_keys — self-service SUBMIT, admin-gated INSTALL.
--
-- A platform-admin uploads their PUBLIC key here; it grants nothing until an
-- admin approves it. That split is the whole point: the console is reachable
-- with an Ory session, and if upload auto-authorized, compromising a web
-- session would escalate straight to a root-capable shell on production. The
-- two gates are the same shape onboarding already uses.
--
-- state: submitted -> approved -> installed | rejected | revoked
CREATE TABLE IF NOT EXISTS ssh_keys (
  id            TEXT PRIMARY KEY,
  created_at    INTEGER NOT NULL,
  handle        TEXT NOT NULL,
  email         TEXT NOT NULL DEFAULT '',
  env           TEXT NOT NULL DEFAULT '',
  key_type      TEXT NOT NULL,           -- ssh-ed25519 | ssh-rsa | ecdsa-...
  key_body      TEXT NOT NULL,           -- base64 body; the identity of the key
  comment       TEXT NOT NULL DEFAULT '',-- cosmetic, never matched on
  fingerprint   TEXT NOT NULL DEFAULT '',-- SHA256:… — what sshd logs at login
  status        TEXT NOT NULL DEFAULT 'submitted',
  decided_by    TEXT NOT NULL DEFAULT '',
  decided_at    INTEGER NOT NULL DEFAULT 0,
  installed_at  INTEGER NOT NULL DEFAULT 0,
  expires_at    INTEGER NOT NULL DEFAULT 0, -- 0 = no expiry recorded
  note          TEXT NOT NULL DEFAULT '',
  -- Installer skip annotation (display-only). A best-effort report about
  -- why an approved key was skipped at install time; never a state change
  -- and cleared on install, revoke and resubmit.
  skip_reason       TEXT NOT NULL DEFAULT '',
  skip_reported_by  TEXT NOT NULL DEFAULT '',
  skip_reported_at  INTEGER NOT NULL DEFAULT 0
);
-- One live row per (handle, key body): re-submitting the same key is an
-- update, never a second grant to reason about.
CREATE UNIQUE INDEX IF NOT EXISTS ssh_keys_handle_body ON ssh_keys (handle, key_body);

-- operator_keys — self-service REQUEST, admin-gated APPROVE.
--
-- An operator key (op_…) makes a Portal admin action attributable to a named
-- person rather than to one shared credential. Issuing one used to mean editing
-- a gitignored envrc on somebody's laptop and redeploying the portal, so the
-- path of least resistance was to share a key that already worked — defeating
-- the attribution the key exists to provide. The friction was never the
-- control; an admin approving is.
--
-- The portal does NOT read this table. It reads the on-host key directory the
-- console writes, and re-reads it while running, so approving grants and
-- revoking removes within one reload interval.
--
-- state: requested -> active -> revoked
--        requested -> rejected
CREATE TABLE IF NOT EXISTS operator_keys (
  id            TEXT PRIMARY KEY,
  created_at    INTEGER NOT NULL,
  handle        TEXT NOT NULL,
  email         TEXT NOT NULL DEFAULT '',
  env           TEXT NOT NULL DEFAULT '',
  reason        TEXT NOT NULL DEFAULT '',   -- what the person says they need it for
  status        TEXT NOT NULL DEFAULT 'requested',
  fingerprint   TEXT NOT NULL DEFAULT '',   -- sha256:… — identifies the key in logs without being one
  holder_identity_id TEXT NOT NULL DEFAULT '',
  -- The live value, held ONLY between approval and its single delivery to the
  -- requester, then emptied. A console database that permanently held live
  -- bearer tokens would be a strictly worse thing to lose than one holding
  -- fingerprints.
  secret        TEXT NOT NULL DEFAULT '',
  decided_by    TEXT NOT NULL DEFAULT '',
  decided_at    INTEGER NOT NULL DEFAULT 0,
  delivered_at  INTEGER NOT NULL DEFAULT 0, -- 0 = not yet collected
  revoked_by    TEXT NOT NULL DEFAULT '',
  revoked_at    INTEGER NOT NULL DEFAULT 0,
  note          TEXT NOT NULL DEFAULT ''
);
-- At most one live key per person per goatnet. Two live keys for one person is
-- two things to revoke, and one of them gets missed.
CREATE UNIQUE INDEX IF NOT EXISTS operator_keys_live
  ON operator_keys (handle, env)
  WHERE status IN ('requested', 'active');

-- onboard_actions is the follow-up queue for an already-onboarded proposal or
-- for a mesh hold whose identity is independent of onboarding.
-- The runner's main loop claims proposals in `approved`; nothing in that shape
-- can ask it to do one more thing for a person later (a lost phone). A row here
-- is such an ask: the console enqueues it, the runner claims it exactly once,
-- runs the matching `goat admin onboard` verb, and writes the result back. One
-- Kinds: reissue_handset (ADR 1175) and reissue_device (an expired or used-up
-- device setup key); a new kind is a new constant, not a new table.
CREATE TABLE IF NOT EXISTS onboard_actions (
    -- Opaque random id (act-<hex>), assigned at enqueue.
    id              TEXT PRIMARY KEY,
    proposal_id     TEXT NOT NULL DEFAULT '',
    kind            TEXT NOT NULL,
    -- queued | claimed | delivering | done | failed
    status          TEXT NOT NULL DEFAULT 'queued',
    -- Who asked (console actor email) — the audit trail's actor, not the runner.
    requested_by    TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    claimed_at      INTEGER,
    claimed_by      TEXT NOT NULL DEFAULT '',
    -- Random generation changed on every claim; stale runners cannot finish
    -- a lease after it has been requeued and claimed again.
    claim_token     TEXT NOT NULL DEFAULT '',
    finished_at     INTEGER,
    -- JSON object the runner reported on completion (never a credential; the
    -- handset key it produced is persisted on the proposal, not here).
    result_json     TEXT NOT NULL DEFAULT '{}',
    error           TEXT NOT NULL DEFAULT '',
    -- JSON object the console attached at enqueue (reissue_device: {"uses": N}).
    params          TEXT NOT NULL DEFAULT '{}',
    -- Immutable runner scope for actions which do not belong to a proposal
    -- (release_hold). Proposal-backed actions leave these blank and inherit
    -- the proposal's env/handle through the query.
    target_env      TEXT NOT NULL DEFAULT '',
    target_handle   TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS onboard_actions_status_idx   ON onboard_actions(status);
CREATE INDEX IF NOT EXISTS onboard_actions_proposal_idx ON onboard_actions(proposal_id);

-- probation_reactivation_requests is deliberately separate from onboarding:
-- these people are already members. A row records an ask only; it performs no
-- NetBird mutation. The hold's entered_at is part of the identity so a retry
-- for the same current hold is idempotent while a later, distinct hold may be
-- petitioned independently.
CREATE TABLE IF NOT EXISTS probation_reactivation_requests (
    id              TEXT PRIMARY KEY,
    peer_id         TEXT NOT NULL,
    peer_name       TEXT NOT NULL,
    handle          TEXT NOT NULL DEFAULT '',
    hold_entered_at INTEGER NOT NULL,
    requested_at    INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'open',
    decided_by      TEXT NOT NULL DEFAULT '',
    decided_at      INTEGER NOT NULL DEFAULT 0,
    decision_reason TEXT NOT NULL DEFAULT ''
);

CREATE UNIQUE INDEX IF NOT EXISTS probation_request_per_hold
  ON probation_reactivation_requests(peer_id, hold_entered_at);
CREATE INDEX IF NOT EXISTS probation_requests_status_time
  ON probation_reactivation_requests(status, requested_at);
