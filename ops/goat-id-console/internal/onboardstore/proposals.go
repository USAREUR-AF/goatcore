package onboardstore

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/jmoiron/sqlx"
)

// Proposal is one inert onboarding-intake proposal (ADR 1061). It mints
// nothing; a sponsor vouches then an operator approves, at which point the
// manifest is written and `goat admin onboard` runs.
type Proposal struct {
	ID        string `db:"id"         json:"id"`
	CreatedAt int64  `db:"created_at" json:"created_at"`
	Sponsor   string `db:"sponsor"    json:"sponsor"`
	// SponsorName is the human-readable label for Sponsor (a participant
	// sponsor's Sponsor is its UUID org anchor). Copied from the invite at
	// consume / stamped at intake. Display-only; '' on pre-column rows.
	SponsorName  string  `db:"sponsor_name"  json:"sponsor_name,omitempty"`
	Env          string  `db:"env"           json:"env"`
	Handle       string  `db:"handle"        json:"handle"`
	Email        string  `db:"email"         json:"email"`
	Status       string  `db:"status"        json:"status"`
	ManifestJSON string  `db:"manifest_json" json:"manifest_json"`
	LintJSON     string  `db:"lint_json"     json:"lint_json"`
	DecidedBy    *string `db:"decided_by"    json:"decided_by,omitempty"`
	DecidedAt    *int64  `db:"decided_at"    json:"decided_at,omitempty"`
	Note         string  `db:"note"          json:"note"`
	// OffboardRequestedBy/At record the admin whose step-up-verified click
	// queued the real cut (goat#372) + when. The runner carries the handle
	// into the revoke leg; the FIRST requester wins and is never overwritten
	// by a concurrent second click. '' / 0 when never requested.
	OffboardRequestedBy string `db:"offboard_requested_by" json:"offboard_requested_by,omitempty"`
	OffboardRequestedAt int64  `db:"offboard_requested_at" json:"offboard_requested_at,omitempty"`
	// OffboardRequestedByIdentity is that admin's Kratos identity id, taken
	// from the authenticated session at the click (never looked up from a
	// record), so the runner can write the operator-action record under the
	// person who approved the cut. '' when the session carried no identity id.
	OffboardRequestedByIdentity string `db:"offboard_requested_by_identity" json:"offboard_requested_by_identity,omitempty"`
	// OffboardReasonClass/Text/Effective are the departure the cut records
	// (ADR 1272): why the person left (new-position | left-service | other),
	// one line of text (required for other), and the day it took effect
	// (YYYY-MM-DD). Written in the same guarded UPDATE as the requester, so
	// they always describe the queued cut. '' on rows queued before the
	// console asked for a reason.
	OffboardReasonClass string `db:"offboard_reason_class" json:"offboard_reason_class,omitempty"`
	OffboardReasonText  string `db:"offboard_reason_text" json:"offboard_reason_text,omitempty"`
	OffboardEffective   string `db:"offboard_effective" json:"offboard_effective,omitempty"`
	// OffboardOperatorAudit is the runner's report on the cut's
	// operator-action record: "recorded", or "failed: <why>". '' until a
	// runner that reports it writes the result back.
	OffboardOperatorAudit string `db:"offboard_operator_audit" json:"offboard_operator_audit,omitempty"`
	// SponsorApprovedBy/At record who acted at the SPONSOR gate (an admin may
	// act there, so it can differ from Sponsor) + when. decided_by is stamped
	// at each decision, so on an onboarded row it is the FINAL admin approver.
	// '' / 0 on rows predating the column.
	SponsorApprovedBy string `db:"sponsor_approved_by" json:"sponsor_approved_by,omitempty"`
	SponsorApprovedAt int64  `db:"sponsor_approved_at" json:"sponsor_approved_at,omitempty"`
	// Retroactive marks a row imported for someone onboarded BEFORE the
	// three-stage system existed — provenance was never recorded; the UI
	// explains that instead of rendering blanks.
	Retroactive bool `db:"retroactive" json:"retroactive,omitempty"`
	// Escalated marks a proposal carrying an escalated admin tier
	// (iam-admin/platform-admin) so the admin queue flags it for extra scrutiny
	// (ADR 1062 Amendment 1).
	Escalated bool `db:"escalated" json:"escalated"`
	// Complete is false for a minimal-burden draft whose manifest couldn't be
	// fully assembled yet (blank tier/affiliation) — the admin completes it.
	Complete bool `db:"complete" json:"complete"`
	// Petitions is a JSON array of {field, sponsor_value, requested_value}: the
	// enrollee asked to change a sponsor-locked field. The effective manifest
	// keeps the sponsor's value; the petition is recorded + highlighted for
	// scrutiny at both review gates (ADR 1062 Amendment 1). "[]" when none.
	Petitions string `db:"petitions" json:"petitions"`
	// OrgSource records where affiliation.id came from at intake: "sponsor" when
	// the invite pinned a registered org, "typed" when it was derived from the
	// organisation the candidate wrote. Empty on rows that predate the column,
	// and on those the review page treats the id as claimed — an admin
	// confirming a membership that was already true costs a click; the reverse
	// provisions someone into an organisation they never belonged to.
	OrgSource string `db:"org_source" json:"org_source,omitempty"`
	// Tier is the effective tier the proposal carries ('' for a draft with no
	// tier yet), denormalized so the stage-2 sponsor gate can compare it against
	// the approver's sponsorship cap without re-parsing the manifest (ADR 1071).
	Tier string `db:"tier" json:"tier,omitempty"`
	// SetupKey is the NetBird enrollment key the runner minted, written back on
	// the onboarded result and persisted so the welcome package can hand it over
	//. '' until the runner reports it, or on legacy rows.
	SetupKey string `db:"setup_key" json:"setup_key,omitempty"`
	// HandsetKey is the second, OBSERVER-only NetBird key minted for the
	// person's phones and tablets (ADR 1175), persisted like SetupKey so the
	// welcome package and a resend can hand it over. '' until the runner
	// reports it, on a manifest with handset_limit 0, or on rows that predate it.
	HandsetKey string `db:"handset_key" json:"handset_key,omitempty"`
	// BundlePath is the fabric bundle's mesh download path the runner reported;
	// the welcome package renders it as a link (portal base + this path). '' when
	// absent (legacy rows / browse tier with no fabric cert).
	BundlePath string `db:"bundle_path" json:"bundle_path,omitempty"`
	// RepoGrantsJSON is the JSON array of GitHub repo-collaborator grants
	// attempted at provision ({owner, repo, permission, outcome}), recorded
	// fail-open so the welcome package can surface them. "[]" when none / feature
	// off (docs/design/onboarding-github-repo-grants.md).
	RepoGrantsJSON string `db:"repo_grants" json:"repo_grants,omitempty"`
	// RepoOverridesJSON is the JSON object of per-onboardee repo-grant OVERRIDES
	// (the "audible" layer): {add:[{repo,permission}], remove:[{repo}]}. Set at
	// sponsor-approve / admin-approve / provision; the effective grant set is the
	// on-rails policy composed with these (docs/design/onboarding-github-repo-grants.md
	// §2). "{}" when none / on pre-column rows.
	RepoOverridesJSON string `db:"repo_overrides" json:"repo_overrides,omitempty"`
	// CollabAccount is the collaboration-environment account the runner provisioned
	// (collab.enabled), written back on the onboarded result. Recorded read-only for
	// mesh-plane awareness (the mesh tracks who has collab access, never governs it).
	// '' when not provisioned / on legacy rows. The one-time setup link is not stored.
	CollabAccount string `db:"collab_account" json:"collab_account,omitempty"`
	// CollabActivated is true once the collab account was provisioned best-effort.
	CollabActivated bool `db:"collab_activated" json:"collab_activated,omitempty"`
	// RestoredBy/At record the admin who brought this row back from `archived`
	// (RestoreProposal) and when. A restored person re-enters the queue at
	// `approved` beside fresh approvals, and without this mark the two are
	// indistinguishable there and on every later card. '' / 0 when the row
	// was never restored; overwritten by a later restore.
	RestoredBy string `db:"restored_by" json:"restored_by,omitempty"`
	RestoredAt int64  `db:"restored_at" json:"restored_at,omitempty"`
	// ClaimedAt is when ClaimProposal/ClaimOffboard claimed this row; the
	// anchor RequeueStaleClaims uses to tell a live mint/revoke from one a
	// dead runner abandoned (goat#977). 0 when never claimed, claimed before
	// this column existed, or requeued.
	ClaimedAt int64 `db:"claimed_at" json:"claimed_at,omitempty"`
}

// Proposal lifecycle states.
const (
	ProposalProposed = "proposed"
	// ProposalSponsorApproved is the stage-2 gate: the sponsor has reviewed the
	// candidate's filled submission and vouched for it, so it moves on to the
	// admin queue (stage 3). A proposal is `proposed` until the sponsor approves,
	// `sponsor_approved` until the admin approves. Two human gates precede a mint.
	ProposalSponsorApproved = "sponsor_approved"
	ProposalApproved        = "approved"
	ProposalRejected        = "rejected"
	ProposalOnboarded       = "onboarded"
	ProposalFailed          = "failed"
	// ProposalOffboarded is the terminal state for a proposal in any
	// offboard-eligible state (OffboardableStatuses: onboarded, enrolled,
	// archived) that has been explicitly offboarded via the console or
	// API. The row is retained for audit (sponsor, approver, tier),
	// but future duplicate-email checks treat it like the other
	// terminal-negative states so a fresh proposal can be submitted.
	ProposalOffboarded = "offboarded"
	// ProposalOnboarding is the "claimed by a runner, mint in progress" state.
	// The onboard-runner atomically flips approved → onboarding (ClaimProposal)
	// before it starts minting, so two runners can't race the same proposal.
	// Terminal transition is onboarding → onboarded / failed.
	ProposalOnboarding = "onboarding"
	// ProposalEnrolled is the ONE observed state in this machine: the person
	// actually used the mesh credential we minted for them. Every other
	// transition is a decision somebody made; this one is a fact about the
	// world, discovered by watching the NetBird setup key's usage count
	// (internal/enrollwatch). `onboarded` therefore means "credentials minted
	// and delivered", not "the person is on the mesh" — the gap between those
	// two is exactly what an operator needs to see to know whether a welcome
	// package actually landed. Terminal except for the operator offboard mark (-> offboarded).
	ProposalEnrolled = "enrolled"
	// ProposalArchived is COLD STORAGE for an onboarding that never happened:
	// credentials were minted and emailed, the setup key was never redeemed, and
	// the key has since been revoked by the ADR 1136 purge lane. The row is kept
	// WHOLE — sponsor, both gate approvers, tier, affiliation, manifest.
	//
	// Archiving rather than deleting keeps the two human gates: deleting the
	// proposal record would mean re-enabling one person re-runs the entire
	// three-stage flow and re-collects vouches for somebody already vouched for,
	// which makes the lane too costly to ever run.
	//
	// Archiving costs the same amount of tidiness and keeps restore to a single
	// admin action (RestoreProposal). Terminal except for the operator offboard
	// mark (-> offboarded) and only in the sense that nothing transitions out of
	// it automatically; an admin can bring it back UNTIL it is offboarded —
	// RestoreProposal is guarded on this state, so offboard is a one-way exit.
	ProposalArchived = "archived"

	// ProposalOffboardRequested is a queued REAL cut (goat#372): an admin's
	// step-up-verified click put the row on the offboard queue; the
	// onboard-runner polls this state and executes the reversible native
	// revoke. Distinct from ProposalOffboarded, which records a cut that
	// already happened (the CLI path). NOT in OffboardableStatuses — these
	// transition states are invisible to the CLI's console-offboard leg
	// (crates/goat-cli revoke_console_proposal), deliberately: a row mid-cut
	// must not be double-decided.
	ProposalOffboardRequested = "offboard-requested"
	// ProposalOffboarding is "claimed by a runner, revoke in progress" — the
	// offboard counterpart of ProposalOnboarding. Terminal transition is
	// offboarding → offboarded / offboard-failed.
	ProposalOffboarding = "offboarding"
	// ProposalOffboardFailed records a cut that did NOT land: the person
	// still has access, the note carries why, and the row is re-requestable
	// so an admin can retry. Loud by design — never silently terminal.
	ProposalOffboardFailed = "offboard-failed"
)

// OffboardableStatuses is the set of proposal lifecycle states eligible for
// an offboard mark. Exported so the human + machine handlers and the guarded
// offboard write all share one source of truth.
//
// The goat-cli revoke path (crates/goat-cli/src/cmd/admin.rs, console
// offboard leg) carries its own copy of this set as the Rust constant
// CONSOLE_OFFBOARDABLE_STATUSES — a fourth offboardable state must be added
// there too, or the CLI silently stops offboarding rows the console considers
// eligible. ops/ci/check-offboardable-status-parity.py fails CI when the two
// sets differ (#579).
var OffboardableStatuses = []string{
	ProposalOnboarded,
	ProposalEnrolled,
	ProposalArchived,
}

// OffboardableProse renders OffboardableStatuses for a sentence, in the set's
// order: "onboarded, enrolled, or archived". Every message that names the set
// is built from this so it cannot stale when the set grows (#579).
func OffboardableProse() string {
	n := len(OffboardableStatuses)
	switch n {
	case 0:
		return ""
	case 1:
		return OffboardableStatuses[0]
	case 2:
		return OffboardableStatuses[0] + " or " + OffboardableStatuses[1]
	}
	return strings.Join(OffboardableStatuses[:n-1], ", ") + ", or " + OffboardableStatuses[n-1]
}

// IsOffboardable reports whether a status is in OffboardableStatuses.
func IsOffboardable(status string) bool {
	for _, s := range OffboardableStatuses {
		if status == s {
			return true
		}
	}
	return false
}

// flag writes a Go bool into a 0/1 BIGINT column.
func flag(b bool) int64 {
	if b {
		return 1
	}
	return 0
}

// newestFirst is the one ordering every "newest proposal" query uses.
const newestFirst = `created_at DESC, id DESC`

// newProposalID returns an opaque random id ("prop-<16-hex>").
func newProposalID() (string, error) {
	var b [8]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return "prop-" + hex.EncodeToString(b[:]), nil
}

// Where an intake's affiliation.id came from (Proposal.OrgSource).
const (
	// OrgSourceSponsor: the invite pinned a registered org id at mint, so the
	// organisation is the sponsor's assertion and needs no second decision.
	OrgSourceSponsor = "sponsor"
	// OrgSourceTyped: the id was derived from the organisation the candidate
	// wrote. It may resolve in the registry and still be a claim nobody has
	// checked, which is the case #1086 exists for.
	OrgSourceTyped = "typed"
	// OrgSourceOperator: an admin attached the proposal to a registered org or
	// created the org from the review panel. The id is then a decision on the
	// record rather than a claim, so the panel stops asking.
	OrgSourceOperator = "operator"
)

// CreateProposal inserts a proposal, assigning id + created_at + a default
// 'proposed' status when unset.
func (s *Store) CreateProposal(ctx context.Context, p *Proposal) error {
	if err := prepareProposal(p); err != nil {
		return err
	}
	if err := s.admitRunnerQueue7788(ctx, p, p.Status); err != nil {
		return err
	}
	return createProposal(ctx, s.db, p)
}

// prepareProposal fills the defaults CreateProposal documents.
func prepareProposal(p *Proposal) error {
	if p.ID == "" {
		id, err := newProposalID()
		if err != nil {
			return fmt.Errorf("proposal id: %w", err)
		}
		p.ID = id
	}
	if p.CreatedAt == 0 {
		p.CreatedAt = time.Now().Unix()
	}
	if p.Status == "" {
		p.Status = ProposalProposed
	}
	if p.LintJSON == "" {
		p.LintJSON = "{}"
	}
	if p.Petitions == "" {
		p.Petitions = "[]"
	}
	return nil
}

// createProposal is CreateProposal's INSERT, runnable inside a transaction.
// The 0/1 flag columns are BIGINT, so the Go bools are written as integers.
func createProposal(ctx context.Context, q sqlx.ExecerContext, p *Proposal) error {
	_, err := q.ExecContext(ctx,
		`INSERT INTO onboard_proposals (id, created_at, sponsor, sponsor_name, env, handle, email, status, manifest_json, lint_json, decided_by, decided_at, note, escalated, complete, petitions, tier, org_source)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)`,
		p.ID, p.CreatedAt, p.Sponsor, p.SponsorName, p.Env, p.Handle, p.Email, p.Status, p.ManifestJSON, p.LintJSON, p.DecidedBy, p.DecidedAt, p.Note, flag(p.Escalated), flag(p.Complete), p.Petitions, p.Tier, p.OrgSource)
	if err != nil {
		return fmt.Errorf("insert proposal: %w", err)
	}
	return nil
}

// ListProposals returns proposals, optionally filtered by status (""=all),
// newest first.
// Every "newest proposal" query orders by newestFirst: created_at is whole
// seconds, so two proposals can share it, and without a second key each query
// could break the tie in its own scan order. The id tie-break is the same in
// ListProposals and in the LIMIT 1 lookups, so a reader that walks
// ListProposals and keeps the first row per email picks exactly what
// ActiveProposalForEmail returns.
//
// On SQLite the second key was rowid, which also made the later insert win a
// same-second tie. Postgres has no rowid and no existing column records insert
// order inside one second (id is random), so on a tie the higher id now wins:
// deterministic and identical across every query, but not insert order.
func (s *Store) ListProposals(ctx context.Context, status string) ([]Proposal, error) {
	return s.ListProposalsFiltered(ctx, status, "")
}

// ListProposalsFiltered intersects an optional status with an exact email.
// lower() over a COLLATE "C" value folds ASCII only, matching the fold used by
// the duplicate-email guard (and SQLite's lower()).
// Empty email keeps the existing unfiltered behavior; HTTP callers validate
// explicitly supplied blank filters before reaching this method.
func (s *Store) ListProposalsFiltered(ctx context.Context, status, email string) ([]Proposal, error) {
	var ps []Proposal
	query := `SELECT * FROM onboard_proposals WHERE 1=1`
	var args []any
	if status != "" {
		args = append(args, status)
		query += fmt.Sprintf(` AND status = $%d`, len(args))
	}
	if email != "" {
		args = append(args, email)
		query += fmt.Sprintf(` AND lower(email) = lower($%d COLLATE "C")`, len(args))
	}
	query += ` ORDER BY ` + newestFirst
	err := s.db.SelectContext(ctx, &ps, query, args...)
	if err != nil {
		return nil, err
	}
	return ps, nil
}

// GetProposal returns one proposal by id, or ErrNotFound.
func (s *Store) GetProposal(ctx context.Context, id string) (*Proposal, error) {
	var p Proposal
	err := s.db.GetContext(ctx, &p, `SELECT * FROM onboard_proposals WHERE id = $1`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	return &p, nil
}

// ActiveProposalForEmail returns the newest still-in-flight or already-onboarded
// proposal for an email (case-insensitive) and whether one exists. "Active" here
// excludes the terminal-negative states (rejected/failed/offboarded), so a fresh submission
// can retry after a rejection or a failed mint but is refused while an onboarding
// is in progress or complete. Empty email → never a match (used for the submit-
// time duplicate guard).
func (s *Store) ActiveProposalForEmail(ctx context.Context, email string) (*Proposal, bool, error) {
	email = strings.TrimSpace(email)
	if email == "" {
		return nil, false, nil
	}
	var p Proposal
	err := s.db.GetContext(ctx, &p,
		`SELECT * FROM onboard_proposals
		   WHERE lower(email) = lower($1 COLLATE "C") AND status NOT IN ($2, $3, $4)
		   ORDER BY `+newestFirst+` LIMIT 1`,
		email, ProposalRejected, ProposalFailed, ProposalOffboarded)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, false, nil
	}
	if err != nil {
		return nil, false, err
	}
	return &p, true, nil
}

// ProposalEmailForHandle returns the email of the newest still-in-flight or
// already-onboarded proposal filed under a handle, or "" when there is none.
// Same "active" definition as ActiveProposalForEmail — the terminal-negative
// states are excluded — because this is that lookup run the other way round.
//
// It exists for the two-person rule (#210): an operator key filed through the
// machine door records a handle and no email, so without this the row could
// not be compared against a browser session that knows only an email.
func (s *Store) ProposalEmailForHandle(ctx context.Context, handle string) (string, error) {
	handle = strings.TrimSpace(handle)
	if handle == "" {
		return "", nil
	}
	var email string
	err := s.db.GetContext(ctx, &email,
		`SELECT email FROM onboard_proposals
		   WHERE lower(handle) = lower($1 COLLATE "C") AND status NOT IN ($2, $3, $4)
		   ORDER BY `+newestFirst+` LIMIT 1`,
		handle, ProposalRejected, ProposalFailed, ProposalOffboarded)
	if errors.Is(err, sqlErrNoRows()) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(email), nil
}

// LatestProposalFor returns the newest proposal whose email OR handle matches
// person (case-insensitive), across EVERY status — unlike the Active lookups,
// terminal states are exactly what the caller wants here: the offboard
// verifier (goat#319) reads an `offboarded` row as the receipt and an
// `onboarded` one as "never offboarded", so excluding either would turn a
// finding into a blank.
func (s *Store) LatestProposalFor(ctx context.Context, person string) (*Proposal, bool, error) {
	person = strings.TrimSpace(person)
	if person == "" {
		return nil, false, nil
	}
	var p Proposal
	err := s.db.GetContext(ctx, &p,
		`SELECT * FROM onboard_proposals
		   WHERE lower(email) = lower($1 COLLATE "C") OR lower(handle) = lower($2 COLLATE "C")
		   ORDER BY `+newestFirst+` LIMIT 1`,
		person, person)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, false, nil
	}
	if err != nil {
		return nil, false, err
	}
	return &p, true, nil
}

// RecordSponsorApproval stamps stage-2 provenance: who acted at the sponsor
// gate + when. Called alongside the sponsor-approve DecideProposal (which
// stamps decided_by — later overwritten by the admin approve, which is why
// this column exists). Returns ErrNotFound when id is unknown.
func (s *Store) RecordSponsorApproval(ctx context.Context, id, by string) error {
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET sponsor_approved_by = $1, sponsor_approved_at = $2 WHERE id = $3`,
		by, time.Now().Unix(), id)
	if err != nil {
		return err
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found" logic below.
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// DecideProposal records an operator decision (approve/reject), stamping the
// status + decider + timestamp + note. Returns ErrNotFound when id is unknown.
func (s *Store) DecideProposal(ctx context.Context, id, status, decidedBy, note string) error {
	now := time.Now().Unix()
	res, err := s.execAdmitted7788(ctx, id, status,
		`UPDATE onboard_proposals SET status = $1, decided_by = $2, decided_at = $3, note = $4 WHERE id = $5`,
		status, decidedBy, now, note, id)
	if err != nil {
		return fmt.Errorf("decide proposal: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// MarkOffboardedFrom records an operator offboard decision guarded on the
// current status, so concurrent transitions cannot overwrite newer states.
// The row must currently be in one of the post-provisioning states
// (OffboardableStatuses) to flip — or be a queued cut (offboard-requested)
// that no runner ever claimed, which an admin settles after cutting the
// access by hand. A claimed cut (offboarding) is the runner's to
// finish and is not accepted here.
//
// Returns true when the update matched and false when no row satisfied the
// guard (unknown id, or a status outside the offboardable set). Callers that
// need to distinguish those cases should re-read the row.
func (s *Store) MarkOffboardedFrom(ctx context.Context, id, decidedBy, note string) (bool, error) {
	if err := s.admitAction7788(ctx, id, ActionOffboard7788); err != nil {
		return false, err
	}
	// ArchiveProposal trims its reason; trim here too so a whitespace-only
	// note cannot blank out a recorded archive reason via the CASE below.
	note = strings.TrimSpace(note)
	now := time.Now().Unix()
	eligible := append(append([]string{}, OffboardableStatuses...), ProposalOffboardRequested)
	query := `UPDATE onboard_proposals SET status = $1, decided_by = $2, decided_at = $3, note = CASE WHEN $4 = '' THEN note ELSE $5 END
	   WHERE id = $6 AND status = ANY($7)`
	args := []any{ProposalOffboarded, decidedBy, now, note, note, id, eligible}
	query += ` AND NOT EXISTS (SELECT 1 FROM onboard_actions WHERE proposal_id = onboard_proposals.id AND status IN ('claimed', 'delivering'))`
	return s.offboardTransition(ctx, id, query, args...)
}

// RequestOffboard queues the REAL cut (goat#372): guarded flip to
// offboard-requested, recording who asked. Eligible from the post-provisioning
// states (OffboardableStatuses) and from offboard-failed (retry). Returns
// false when no row satisfied the guard — unknown id, a pre-provisioning
// status, or a cut already queued/running (the two-admins double-click; the
// guard is what keeps the first requester's attribution intact).
func (s *Store) RequestOffboard(ctx context.Context, id, requestedBy string) (bool, error) {
	return s.RequestOffboardBy(ctx, id, requestedBy, "")
}

// RequestOffboardBy is RequestOffboard that also records the requester's
// session identity id (empty when unknown). Both come from the same
// authenticated session and are written in the same guarded UPDATE, so a
// second click can never pair one admin's email with another's identity.
// It also clears offboard_operator_audit: a re-queue after offboard-failed
// starts a new cut, and the previous attempt's record outcome must not be
// read as this one's.
func (s *Store) RequestOffboardBy(ctx context.Context, id, requestedBy, requestedByIdentity string) (bool, error) {
	return s.requestOffboard(ctx, id, requestedBy, requestedByIdentity, Departure{})
}

// Departure is why and when a person left, recorded with the cut (ADR 1272).
type Departure struct {
	Class     string // new-position | left-service | other
	Text      string // one line; required for other
	Effective string // YYYY-MM-DD
}

// The reason classes of a departure. Every class cuts all access; the class
// records how the person may return.
const (
	DepartureNewPosition = "new-position"
	DepartureLeftService = "left-service"
	DepartureOther       = "other"
)

// ErrDepartureReason is returned when a departure names no valid reason
// class, gives "other" without its text, or carries a malformed date.
var ErrDepartureReason = errors.New("an offboarding needs the reason the person left: new-position, left-service, or other with one line of text; and its date as YYYY-MM-DD")

// Validate trims the departure and holds it to the three classes.
func (d *Departure) Validate() error {
	d.Class, d.Text, d.Effective = strings.TrimSpace(d.Class), strings.TrimSpace(d.Text), strings.TrimSpace(d.Effective)
	switch d.Class {
	case DepartureNewPosition, DepartureLeftService:
	case DepartureOther:
		if d.Text == "" {
			return ErrDepartureReason
		}
	default:
		return ErrDepartureReason
	}
	if len(d.Text) > 300 || strings.ContainsAny(d.Text, "\r\n") {
		return ErrDepartureReason
	}
	if _, err := time.Parse("2006-01-02", d.Effective); err != nil {
		return ErrDepartureReason
	}
	return nil
}

// RequestOffboardDeparture is RequestOffboardBy that also records the
// departure: the reason class, its text and the effective day, in the same
// guarded UPDATE as the requester. It refuses an invalid departure before
// touching the row.
func (s *Store) RequestOffboardDeparture(ctx context.Context, id, requestedBy, requestedByIdentity string, d Departure) (bool, error) {
	if err := d.Validate(); err != nil {
		return false, err
	}
	return s.requestOffboard(ctx, id, requestedBy, requestedByIdentity, d)
}

func (s *Store) requestOffboard(ctx context.Context, id, requestedBy, requestedByIdentity string, d Departure) (bool, error) {
	eligible := append(append([]string{}, OffboardableStatuses...), ProposalOffboardFailed)
	query := `UPDATE onboard_proposals
	   SET status = $1, offboard_requested_by = $2, offboard_requested_at = $3, offboard_requested_by_identity = $4,
	       offboard_operator_audit = '',
	       offboard_reason_class = $7, offboard_reason_text = $8, offboard_effective = $9
	 WHERE id = $5 AND status = ANY($6)`
	args := []any{ProposalOffboardRequested, requestedBy, time.Now().Unix(), requestedByIdentity, id, eligible,
		d.Class, d.Text, d.Effective}
	return s.offboardTransition(ctx, id, query, args...)
}

// DepartureStatuses are the proposal states that record a departure: a cut
// queued, running, done or failed.
var DepartureStatuses = []string{ProposalOffboardRequested, ProposalOffboarding, ProposalOffboarded, ProposalOffboardFailed}

// ListDepartures returns every person whose access cut was queued, is
// running, finished or failed, newest request first: the goatnet's register
// of who has left (ADR 1272).
func (s *Store) ListDepartures(ctx context.Context) ([]Proposal, error) {
	var ps []Proposal
	err := s.db.SelectContext(ctx, &ps,
		`SELECT * FROM onboard_proposals WHERE status = ANY($1)
		  ORDER BY GREATEST(offboard_requested_at, COALESCE(decided_at, 0)) DESC, id`,
		DepartureStatuses)
	if err != nil {
		return nil, fmt.Errorf("list departures: %w", err)
	}
	return ps, nil
}

// SetOffboardResult records the offboard leg's terminal result in ONE
// statement: the status (offboarded / offboard-failed), the failure reason as
// the note (only when non-empty, as SetProposalResult does), and the runner's
// operator-action outcome ("recorded", "failed: <why>", or "" when the runner
// reported none). One statement, so a row can never show the new status
// without the mark that qualifies it. Returns ErrNotFound when id is unknown.
func (s *Store) SetOffboardResult(ctx context.Context, id, status, note, operatorAudit string) error {
	if status != ProposalOffboarded && status != ProposalOffboardFailed {
		return fmt.Errorf("set offboard result: %q is not an offboard result", status)
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET status = $1,
		   note = CASE WHEN $2 <> '' THEN $3 ELSE note END,
		   offboard_operator_audit = $4
		 WHERE id = $5`,
		status, note, note, operatorAudit, id)
	if err != nil {
		return fmt.Errorf("set offboard result: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // best-effort; unknown row count falls through as "not found"
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// ClaimOffboard atomically transitions an offboard-requested proposal to
// offboarding (claimed), the exact counterpart of ClaimProposal on the
// offboard queue: two concurrent runners can't both claim, exactly one wins.
// Returns ErrNotFound when the id is unknown, or ErrProposalNotOffboardRequested
// when it exists but no cut is queued — the runner never cuts anyone on its
// own initiative.
func (s *Store) ClaimOffboard(ctx context.Context, id string) error {
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET status = $1, claimed_at = $2 WHERE id = $3 AND status = $4
         AND NOT EXISTS (SELECT 1 FROM onboard_actions WHERE proposal_id = onboard_proposals.id AND status IN ('claimed', 'delivering'))`,
		ProposalOffboarding, time.Now().Unix(), id, ProposalOffboardRequested)
	if err != nil {
		return fmt.Errorf("claim offboard: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("claim offboard: count affected rows: %w", err)
	}
	if n == 1 {
		return nil
	}
	if _, err := s.GetProposal(ctx, id); err != nil {
		return err // ErrNotFound (or a real DB error)
	}
	var running int
	if err := s.db.GetContext(ctx, &running, `SELECT COUNT(*) FROM onboard_actions WHERE proposal_id = $1 AND status IN ('claimed','delivering')`, id); err != nil {
		return err
	}
	if running > 0 {
		return ErrReissueRunning
	}
	return ErrProposalNotOffboardRequested
}

// SetProposalStatus advances the status without recording a new decision
// (e.g. approved → onboarded once the mint succeeds, or → failed). Returns
// ErrNotFound when id is unknown.
func (s *Store) SetProposalStatus(ctx context.Context, id, status string) error {
	// Any other status write is bookkeeping about the mint or the person
	// (onboarded, failed, ...): it carries the result cap (GHSA-7788).
	if status != ProposalApproved {
		if err := s.admitAction7788(ctx, id, ActionResult7788); err != nil {
			return err
		}
	}
	res, err := s.execAdmitted7788(ctx, id, status,
		`UPDATE onboard_proposals SET status = $1 WHERE id = $2`, status, id)
	if err != nil {
		return fmt.Errorf("set proposal status: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// SetProposalResult records a terminal runner result: the status (onboarded /
// failed) plus the minted setup key (stored only on success; blank leaves any
// existing value untouched). Persisting the key is what lets the welcome package
// hand over mesh enrollment after the fact. Returns ErrNotFound when id
// is unknown.
func (s *Store) SetProposalResult(ctx context.Context, id, status, setupKey, handsetKey, bundlePath, note, collabAccount string, collabActivated bool) error {
	// Set status always; update setup_key / bundle_path / note / collab_account
	// only when a non-empty value was sent, so a partial writeback never blanks a
	// stored value. `note` carries the mint failure reason on a `failed` result so
	// the admin queue can show WHY it failed instead of discarding it. collab_activated
	// only ever latches to 1 (once provisioned, stays), never back to 0.
	if err := s.admitRunnerQueueByID7788(ctx, id, status); err != nil {
		return err
	}
	if err := s.admitAction7788(ctx, id, ActionResult7788); err != nil {
		return err
	}
	activatedInt := 0
	if collabActivated {
		activatedInt = 1
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET status = $1,
		   setup_key      = CASE WHEN $2 <> '' THEN $3 ELSE setup_key      END,
		   handset_key    = CASE WHEN $4 <> '' THEN $5 ELSE handset_key    END,
		   bundle_path    = CASE WHEN $6 <> '' THEN $7 ELSE bundle_path    END,
		   note           = CASE WHEN $8 <> '' THEN $9 ELSE note           END,
		   collab_account = CASE WHEN $10 <> '' THEN $11 ELSE collab_account END,
		   collab_activated = CASE WHEN $12 <> 0 THEN 1 ELSE collab_activated END
		 WHERE id = $13`,
		status, setupKey, setupKey, handsetKey, handsetKey, bundlePath, bundlePath, note, note,
		collabAccount, collabAccount, activatedInt, id)
	if err != nil {
		return fmt.Errorf("set proposal result: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// SetProposalRepoGrants persists the JSON array of GitHub repo-collaborator
// grant outcomes recorded at provision (fail-open — this never gates the
// onboard). A blank value is normalized to "[]". Returns ErrNotFound when id is
// unknown.
func (s *Store) SetProposalRepoGrants(ctx context.Context, id, repoGrantsJSON string) error {
	if strings.TrimSpace(repoGrantsJSON) == "" {
		repoGrantsJSON = "[]"
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET repo_grants = $1 WHERE id = $2`, repoGrantsJSON, id)
	if err != nil {
		return fmt.Errorf("set proposal repo grants: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// SetProposalRepoOverrides persists the JSON object of per-onboardee repo-grant
// overrides (the "audible" layer). A blank value is normalized to "{}". Returns
// ErrNotFound when id is unknown.
func (s *Store) SetProposalRepoOverrides(ctx context.Context, id, repoOverridesJSON string) error {
	if strings.TrimSpace(repoOverridesJSON) == "" {
		repoOverridesJSON = "{}"
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET repo_overrides = $1 WHERE id = $2`, repoOverridesJSON, id)
	if err != nil {
		return fmt.Errorf("set proposal repo overrides: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// ClaimProposal atomically transitions an approved proposal to onboarding
// (claimed), guarding on the current status in the UPDATE so two concurrent
// runners can't both claim the same proposal — exactly one wins. Returns
// ErrNotFound when the id is unknown, or ErrProposalNotApproved when it exists
// but isn't approved (already claimed / onboarded / rejected). The onboard-runner
// calls this before it mints; only the winner proceeds.
func (s *Store) ClaimProposal(ctx context.Context, id string) error {
	if err := s.admitAction7788(ctx, id, ActionClaim7788); err != nil {
		return err
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET status = $1, claimed_at = $2 WHERE id = $3 AND status = $4`,
		ProposalOnboarding, time.Now().Unix(), id, ProposalApproved)
	if err != nil {
		return fmt.Errorf("claim proposal: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through to the GetProposal check.
	if n == 1 {
		return nil
	}
	// No row flipped: either the id is unknown or its status wasn't approved.
	// Load once to disambiguate so the caller can return the right HTTP code.
	if _, err := s.GetProposal(ctx, id); err != nil {
		return err // ErrNotFound (or a real DB error)
	}
	return ErrProposalNotApproved
}

// UpdateProposalManifest replaces the proposal's manifest_json — the org
// match-or-create review step (ADR 1112 §6) rewrites affiliation fields to
// the canonical org before mint. Refused once the proposal has left the
// review stages (approved/onboarding/onboarded/…): the manifest an admin
// approved is what mints, never a post-decision edit.
func (s *Store) UpdateProposalManifest(ctx context.Context, id, manifestJSON string) error {
	if strings.TrimSpace(manifestJSON) == "" {
		return fmt.Errorf("update proposal manifest: empty manifest")
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET manifest_json = $1
		 WHERE id = $2 AND status IN ($3, $4)`,
		manifestJSON, id, ProposalProposed, ProposalSponsorApproved)
	if err != nil {
		return fmt.Errorf("update proposal manifest: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// SetProposalOrgSource records how affiliation.id came to be what it is. The
// review panel calls it with OrgSourceOperator once an admin has attached the
// proposal to a registered org or created that org, which is what turns a
// candidate's claim into a decision and clears the panel.
//
// Guarded on the review stages for the same reason UpdateProposalManifest is:
// the manifest an admin approved is what mints, and its provenance must not be
// rewritten after the fact. Idempotent — a second call writes the same value.
func (s *Store) SetProposalOrgSource(ctx context.Context, id, source string) error {
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET org_source = $1
		 WHERE id = $2 AND status IN ($3, $4)`,
		source, id, ProposalProposed, ProposalSponsorApproved)
	if err != nil {
		return fmt.Errorf("update proposal org source: %w", err)
	}
	n, raErr := res.RowsAffected()
	_ = raErr // The driver's RowsAffected error is best-effort; unknown row count falls through as "not found".
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// ArchiveProposal moves an `onboarded` proposal to cold storage: the person was
// sent credentials, never redeemed them, and the ADR 1136 purge lane has revoked
// the unredeemed key.
//
// Guarded on the CURRENT status being exactly `onboarded`, which is what makes
// this safe to run from a sweep. An `enrolled` row is somebody who DID join and
// must never be archived by a dormancy pass; a `rejected`/`failed` row has its
// own terminal meaning that archiving would erase. Both are excluded by the
// WHERE clause rather than by the caller remembering to check.
//
// Idempotent: a second call matches no row and reports false.
func (s *Store) ArchiveProposal(ctx context.Context, id, reason string) (bool, error) {
	if err := s.admitAction7788(ctx, id, ActionArchive7788); err != nil {
		return false, err
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET status = $1, note = $2 WHERE id = $3 AND status = $4`,
		ProposalArchived, strings.TrimSpace(reason), id, ProposalOnboarded)
	if err != nil {
		return false, fmt.Errorf("archive proposal: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("archive proposal: count affected rows: %w", err)
	}
	return n == 1, nil
}

// RestoreProposal brings an archived row back to `approved`, which is the state
// the onboard-runner claims from — so a restore re-mints a fresh setup key and
// the console re-sends the welcome package, with no human gate repeated.
//
// THAT is the whole point of archiving rather than deleting. The row still
// carries `sponsor`, `sponsor_approved_by` and `decided_by` from the original
// onboarding, so restoring re-uses vouches that were already given rather than
// asking two people to give them again for somebody they already approved.
// Returning to `approved` (not `proposed`) is what skips the gates; going back
// further would make restore indistinguishable from starting over.
//
// actor is stamped into restored_by/restored_at so the row says who brought
// it back; it is the one fact that separates a restored person from a fresh
// approval in the queue. Both the operator API and the console button call
// this — one guarded write, two callers.
//
// Guarded on `archived` so it cannot resurrect a rejected row or re-approve a
// live one. Idempotent: a second call matches no row and reports false.
//
// The manifest goes back through dropUnjustifiedTTL on the way: a row archived
// previously still asks for the old 90-day key, which the mint now
// refuses, so without it every such restore lands straight in `failed`.
func (s *Store) RestoreProposal(ctx context.Context, id, actor string) (bool, error) {
	return s.reenterApproved(ctx, id, ProposalArchived,
		`UPDATE onboard_proposals SET status = $1, manifest_json = $2, setup_key = '', bundle_path = '',
		        restored_by = $3, restored_at = $4
		   WHERE id = $5 AND status = $6`,
		func(manifest string) []any {
			return []any{ProposalApproved, manifest, strings.TrimSpace(actor), time.Now().Unix(), id, ProposalArchived}
		})
}

// reenterApproved is the one write behind restore and retry: read the row's
// manifest under the status guard, drop an unjustified key lifetime, and flip
// the row to `approved` in the same transaction. args receives the manifest to
// store and returns the UPDATE's arguments. Reports whether a row moved.
func (s *Store) reenterApproved(ctx context.Context, id, from, update string, args func(manifest string) []any) (bool, error) {
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return false, fmt.Errorf("re-enter approved: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// The whole row is read under the lock: the runner-queue check
	// (GHSA-7788) ranks its tier and manifest before it re-enters `approved`.
	var row Proposal
	err = tx.GetContext(ctx, &row,
		`SELECT * FROM onboard_proposals WHERE id = $1 AND status = $2 FOR UPDATE`, id, from)
	if errors.Is(err, sql.ErrNoRows) {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("re-enter approved: %w", err)
	}
	if err := s.admitRunnerQueue7788(ctx, &row, ProposalApproved); err != nil {
		return false, err
	}
	manifest := row.ManifestJSON
	if fixed, changed := dropUnjustifiedTTL(manifest); changed {
		manifest = fixed
	}
	if err := ValidateProvisioningManifest(manifest); err != nil {
		return false, err
	}
	res, err := tx.ExecContext(ctx, update, args(manifest)...)
	if err != nil {
		return false, fmt.Errorf("re-enter approved: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("re-enter approved: count affected rows: %w", err)
	}
	if n != 1 {
		return false, nil
	}
	if err := tx.Commit(); err != nil {
		return false, fmt.Errorf("re-enter approved: %w", err)
	}
	return true, nil
}

// roleTTLDays is the setup-key lifetime the mint allows without a stated
// reason. It mirrors role_ttl_days in crates/goat-cli/src/cmd/admin.rs, which
// is the copy that enforces; this one only decides what restore and retry drop.
func roleTTLDays(role string) int {
	switch role {
	case "bootstrap":
		return 1
	case "operator":
		return 30
	default: // admin, peer and unknown
		return 7
	}
}

// minTTLReasonLen matches check_ttl_override in the CLI: a shorter reason is
// refused there, so it does not justify keeping the longer window here.
const minTTLReasonLen = 12

// dropUnjustifiedTTL removes netbird.ttl_days when it is longer than the role
// default and netbird.ttl_reason does not say why. The mint refuses exactly
// that combination (check_ttl_override), and every row approved before the peer
// default was cut from 90 days to 7 carries it.
//
// Dropping it only ever shortens the credential to the role default, which the
// mint accepts without justification, so it narrows what the admin approved and
// never widens it. A justified override, a shorter one, or a manifest this
// cannot parse is returned unchanged.
func dropUnjustifiedTTL(manifest string) (string, bool) {
	dec := json.NewDecoder(strings.NewReader(manifest))
	dec.UseNumber()
	var m map[string]any
	if err := dec.Decode(&m); err != nil {
		return manifest, false
	}
	nb, ok := m["netbird"].(map[string]any)
	if !ok {
		return manifest, false
	}
	num, ok := nb["ttl_days"].(json.Number)
	if !ok {
		return manifest, false
	}
	days, err := num.Int64()
	if err != nil {
		return manifest, false
	}
	role, _ := nb["role"].(string)
	if days <= int64(roleTTLDays(role)) {
		return manifest, false
	}
	if reason, _ := nb["ttl_reason"].(string); len(strings.TrimSpace(reason)) >= minTTLReasonLen {
		return manifest, false
	}
	delete(nb, "ttl_days")
	out, err := json.Marshal(m)
	if err != nil {
		return manifest, false
	}
	return string(out), true
}

// MarkEnrolled flips an `onboarded` proposal to `enrolled` — the observed
// state: the person consumed the mesh setup key we minted for them.
//
// Guarded on the CURRENT status being exactly `onboarded` so the watcher can
// never resurrect a rejected/failed row, and never overwrite a later state.
// Idempotent by construction: the second call matches no row and reports
// false, so a watcher tick that re-observes the same key is a no-op rather
// than a repeated audit event.
//
// Returns true when this call performed the transition.
func (s *Store) MarkEnrolled(ctx context.Context, id string) (bool, error) {
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_proposals SET status = $1 WHERE id = $2 AND status = $3`,
		ProposalEnrolled, id, ProposalOnboarded)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n > 0, nil
}

// ErrProposalNotFailed is returned by RetryProposal when the proposal exists
// but is not in the `failed` state — retry is only ever a recovery from a
// mint that broke, never a way to re-run a decision.
var ErrProposalNotFailed = errors.New("proposal is not in the failed state")

// RetryProposal flips a `failed` proposal back to `approved` so the
// onboard-runner picks it up again on its next poll.
//
// Why this exists: a mint that fails for an environmental reason — the runner
// missing a token, an unreachable portal, a half-provisioned remnant — must not
// leave the proposal terminal at `failed`. The alternative is a hand-written
// UPDATE against this SQLite file, which is exactly the out-of-band surgery
// docs/operations/onboarding-rails.md exists to forbid. Both human gates have
// already been passed at this point, so returning the row to `approved` re-runs
// only the mint — it re-decides nothing.
//
// Guarded on the current status being exactly `failed`: this can never
// resurrect a rejected proposal, restart a live mint, or re-run a completed
// one. Idempotent — a second call matches no row and reports ErrProposalNotFailed.
//
// Like restore, it drops an unjustified key lifetime (dropUnjustifiedTTL): a
// row that failed on the mint's ttl_reason refusal would otherwise fail again.
func (s *Store) RetryProposal(ctx context.Context, id string) error {
	moved, err := s.reenterApproved(ctx, id, ProposalFailed,
		`UPDATE onboard_proposals SET status = $1, manifest_json = $2 WHERE id = $3 AND status = $4`,
		func(manifest string) []any { return []any{ProposalApproved, manifest, id, ProposalFailed} })
	if err != nil {
		return fmt.Errorf("retry proposal: %w", err)
	}
	if moved {
		return nil
	}
	// Nothing flipped: unknown id, or a status other than failed. Load once so
	// the caller can answer 404 vs 409 honestly.
	if _, err := s.GetProposal(ctx, id); err != nil {
		return err
	}
	return ErrProposalNotFailed
}

// Generous against a mint/revoke that normally takes seconds, so the sweep
// only fires on a runner that actually died mid-claim (goat#977).
const DefaultClaimRequeueAfter = 10 * time.Minute

// RequeueStaleClaims recovers rows a dead runner stranded (goat#977): a claim
// (onboarding/offboarding) that has sat unreported for at least `after` is put
// back on the queue the runner polls — onboarding -> approved, offboarding ->
// offboard-requested — so the next tick picks it up. Nothing else ever revisits
// a claimed row: the runner polls only approved/offboard-requested and the
// console/CLI skip mid-cut rows, so without this the only recovery is editing
// SQLite by hand.
//
// A slow-but-alive runner can still report back after a requeue, double-running
// the mint/revoke; both legs are idempotent/reversible, so that race is an
// accepted risk, not a bug.
//
// Status-guarded UPDATE per row (mirrors ArchiveStaleInvites): a row that
// reported back between read and write is left alone, and the sweep is
// idempotent. after <= 0 uses DefaultClaimRequeueAfter.
func (s *Store) RequeueStaleClaims(ctx context.Context, after time.Duration) ([]Proposal, error) {
	if after <= 0 {
		after = DefaultClaimRequeueAfter
	}
	var ps []Proposal
	if err := s.db.SelectContext(ctx, &ps,
		`SELECT * FROM onboard_proposals WHERE status IN ($1, $2)`,
		ProposalOnboarding, ProposalOffboarding); err != nil {
		return nil, fmt.Errorf("requeue stale claims: list: %w", err)
	}
	now := time.Now()
	cutoff := now.Add(-after).Unix()
	var moved []Proposal
	for _, p := range ps {
		// A row claimed before this column existed has claimed_at 0; fall back
		// to created_at so it still recovers rather than staying stranded. It's
		// later than the true claim time, so this never fires early.
		anchor := p.ClaimedAt
		if anchor == 0 {
			anchor = p.CreatedAt
		}
		if anchor > cutoff {
			continue
		}
		var target string
		if p.Status == ProposalOnboarding {
			target = ProposalApproved
		} else {
			target = ProposalOffboardRequested
		}
		note := fmt.Sprintf("re-queued: runner did not report back within %s", after)
		res, err := s.db.ExecContext(ctx,
			`UPDATE onboard_proposals SET status = $1, claimed_at = 0, note = $2 WHERE id = $3 AND status = $4`,
			target, note, p.ID, p.Status)
		if err != nil {
			return moved, fmt.Errorf("requeue stale claims: %s: %w", p.ID, err)
		}
		n, err := res.RowsAffected()
		if err != nil {
			return moved, fmt.Errorf("requeue stale claims: %s: count affected rows: %w", p.ID, err)
		}
		if n == 1 {
			p.Status = target
			p.ClaimedAt = 0
			p.Note = note
			moved = append(moved, p)
		}
	}
	return moved, nil
}
