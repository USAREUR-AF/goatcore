package onboardstore

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"
)

// Action is one queued follow-up for an already-onboarded proposal (ADR 1175):
// the console asks, the runner claims it exactly once, runs the matching
// `goat admin onboard` verb, and records the outcome. The proposals queue only
// carries a person TO onboarded; this table carries what happens after.
type Action struct {
	ID          string `db:"id"           json:"id"`
	ProposalID  string `db:"proposal_id"  json:"proposal_id"`
	Kind        string `db:"kind"         json:"kind"`
	Status      string `db:"status"       json:"status"`
	RequestedBy string `db:"requested_by" json:"requested_by,omitempty"`
	CreatedAt   int64  `db:"created_at"   json:"created_at"`
	ClaimedAt   *int64 `db:"claimed_at"   json:"claimed_at,omitempty"`
	ClaimedBy   string `db:"claimed_by"   json:"claimed_by,omitempty"`
	ClaimToken  string `db:"claim_token"  json:"claim_token,omitempty"`
	FinishedAt  *int64 `db:"finished_at"  json:"finished_at,omitempty"`
	ResultJSON  string `db:"result_json"  json:"result_json,omitempty"`
	Error       string `db:"error"        json:"error,omitempty"`
	// Params is the JSON object the console attached at enqueue — for
	// reissue_device, {"uses": N}. Never a credential.
	Params string `db:"params" json:"params,omitempty"`
	// Handle and Env are the proposal's, joined in for the runner so it can
	// build the goat command without a second lookup.
	Handle string `db:"handle" json:"handle"`
	Env    string `db:"env"    json:"env"`
}

// Action kinds and lifecycle states.
const (
	// ActionReissueHandset revokes the person's live handset keys and mints a
	// fresh one (`goat admin onboard <handle> --native --reissue-handset-key`),
	// for a lost or replaced phone. The new key lands on the proposal's
	// handset_key and the welcome package is re-sent.
	ActionReissueHandset = "reissue_handset"
	// ActionReissueDevice mints a fresh device setup key for a person whose key
	// expired or was used up, and revokes any of their device keys still live
	// (`goat admin onboard <handle> --native --reissue-device-key --uses N`).
	// Params carry the reuse count. The new key lands on the proposal's
	// setup_key and the welcome package is re-sent.
	ActionReissueDevice = "reissue_device"
	// ActionReleaseHold asks the site runner to resume the lifecycle release
	// for one immutable (site, peer_id, entered_at) hold identity.
	ActionReleaseHold = "release_hold"

	ActionQueued     = "queued"
	ActionClaimed    = "claimed"
	ActionDelivering = "delivering"
	ActionDone       = "done"
	ActionFailed     = "failed"

	// ReleaseHoldClaimRequeueAfter bounds how long a dead runner may strand a
	// claimed hold release. The lifecycle release is resumable and idempotent,
	// and its entered_at parameter binds every retry to the same hold identity.
	ReleaseHoldClaimRequeueAfter = 10 * time.Minute
)

var (
	// ErrActionNotOnboarded: a follow-up only makes sense for a person who has
	// been minted (onboarded, or enrolled once they used the key); anything
	// earlier is the proposals queue's business, and anything later (archived,
	// offboarded) has had its access cut on purpose.
	ErrActionNotOnboarded = errors.New("proposal is not onboarded")
	// ErrActionPending: the same kind is already queued or claimed for this
	// proposal, so a second click does not mint a second key.
	ErrActionPending = errors.New("an action of this kind is already pending for this proposal")
	// ErrActionNotQueued: a claim or result arrived for an action that is not
	// in the state the transition requires.
	ErrActionNotQueued = errors.New("action is not queued")
	// ErrActionNotClaimed: a result arrived for an action nobody claimed.
	ErrActionNotClaimed = errors.New("action is not claimed")
)

func newActionID() (string, error) {
	var b [8]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return "act-" + hex.EncodeToString(b[:]), nil
}

func newClaimToken() (string, error) {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(b[:]), nil
}

const actionSelect = `SELECT a.id, a.proposal_id, a.kind, a.status, a.requested_by, a.created_at,
	a.claimed_at, a.claimed_by, a.claim_token, a.finished_at, a.result_json, a.error, a.params,
	COALESCE(NULLIF(a.target_handle, ''), p.handle, '') AS handle,
	COALESCE(NULLIF(a.target_env, ''), p.env, '') AS env
	FROM onboard_actions a LEFT JOIN onboard_proposals p ON p.id = a.proposal_id`

// EnqueueHoldAction queues one exact lifecycle release without requiring an
// onboarding proposal. Machines and members enrolled before the console are
// therefore first-class subjects. The exact hold key is checked in the same
// transaction as the insert, making a double click harmless.
func (s *Store) EnqueueHoldAction(ctx context.Context, env, peerID string, enteredAt time.Time, handle, requestedBy, params string) (*Action, error) {
	if strings.TrimSpace(env) == "" || strings.TrimSpace(peerID) == "" || enteredAt.IsZero() {
		return nil, fmt.Errorf("hold action requires env, peer id, and entered_at")
	}
	if _, err := s.requeueStaleReleaseClaimsAt(ctx, ReleaseHoldClaimRequeueAfter, time.Now()); err != nil {
		return nil, err
	}
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// The pending check and the insert must not interleave with another
	// enqueue for the same hold. There is no row to lock (the hold has no
	// proposal), so a transaction-scoped advisory lock on the hold key does it;
	// on SQLite the single connection serialized the two statements.
	holdKey := env + "\n" + peerID + "\n" + enteredAt.UTC().Format(time.RFC3339)
	if _, err := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1, 0))`, "onboard_actions/release_hold\n"+holdKey); err != nil {
		return nil, fmt.Errorf("lock hold action: %w", err)
	}
	var pending int
	if err := tx.GetContext(ctx, &pending, `SELECT COUNT(*) FROM onboard_actions
		WHERE kind = $1 AND status IN ($2, $3)
		AND target_env = $4
		AND params::jsonb->>'peer_id' = $5
		AND params::jsonb->>'hold_entered_at' = $6`,
		ActionReleaseHold, ActionQueued, ActionClaimed, env, peerID, enteredAt.UTC().Format(time.RFC3339)); err != nil {
		return nil, fmt.Errorf("pending hold-action check: %w", err)
	}
	if pending > 0 {
		return nil, ErrActionPending
	}
	id, err := newActionID()
	if err != nil {
		return nil, fmt.Errorf("action id: %w", err)
	}
	if strings.TrimSpace(params) == "" {
		params = "{}"
	}
	if _, err := tx.ExecContext(ctx, `INSERT INTO onboard_actions
		(id, proposal_id, kind, status, requested_by, created_at, params, target_env, target_handle)
		VALUES ($1, '', $2, $3, $4, $5, $6, $7, $8)`, id, ActionReleaseHold, ActionQueued,
		requestedBy, time.Now().Unix(), params, env, handle); err != nil {
		return nil, fmt.Errorf("insert hold action: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("commit: %w", err)
	}
	return s.GetAction(ctx, id)
}

// EnqueueAction queues a follow-up of the given kind for an onboarded proposal.
// Refuses with ErrNotFound (unknown proposal), ErrActionNotOnboarded (proposal
// in any other state) or ErrActionPending (same kind already queued/claimed).
// The refusals are checked inside one transaction so two concurrent clicks
// cannot both pass the pending check.
func (s *Store) EnqueueAction(ctx context.Context, proposalID, kind, requestedBy string) (*Action, error) {
	return s.EnqueueActionWithParams(ctx, proposalID, kind, requestedBy, "")
}

// actionEligible is the set of proposal states a follow-up may be queued for:
// minted and not since cut. `enrolled` is included because an expired or
// used-up key is most often met AFTER the person enrolled a first device.
func actionEligible(status string) bool {
	return status == ProposalOnboarded || status == ProposalEnrolled
}

// EnqueueActionWithParams is EnqueueAction with a JSON params object stored on
// the row for the runner ("" means {}).
func (s *Store) EnqueueActionWithParams(ctx context.Context, proposalID, kind, requestedBy, params string) (*Action, error) {
	if params == "" {
		params = "{}"
	}
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()

	// FOR UPDATE holds the proposal row until commit, so a second enqueue for
	// it waits here and then sees the first one's row in the pending check, and
	// no status change can land between the eligibility read and the insert.
	var status string
	if err := tx.GetContext(ctx, &status, `SELECT status FROM onboard_proposals WHERE id = $1 FOR UPDATE`, proposalID); err != nil {
		if errors.Is(err, sqlErrNoRows()) {
			return nil, ErrNotFound
		}
		return nil, fmt.Errorf("load proposal: %w", err)
	}
	if !actionEligible(status) {
		return nil, ErrActionNotOnboarded
	}
	var pending int
	if err := tx.GetContext(ctx, &pending,
		`SELECT COUNT(*) FROM onboard_actions WHERE proposal_id = $1 AND kind = $2 AND status IN ($3, $4, $5)`,
		proposalID, kind, ActionQueued, ActionClaimed, ActionDelivering); err != nil {
		return nil, fmt.Errorf("pending check: %w", err)
	}
	if pending > 0 {
		return nil, ErrActionPending
	}
	id, err := newActionID()
	if err != nil {
		return nil, fmt.Errorf("action id: %w", err)
	}
	now := time.Now().Unix()
	if _, err := tx.ExecContext(ctx,
		`INSERT INTO onboard_actions (id, proposal_id, kind, status, requested_by, created_at, params)
		 VALUES ($1, $2, $3, $4, $5, $6, $7)`, id, proposalID, kind, ActionQueued, requestedBy, now, params); err != nil {
		return nil, fmt.Errorf("insert action: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("commit: %w", err)
	}
	return s.GetAction(ctx, id)
}

// GetAction loads one action with its proposal's handle and env.
func (s *Store) GetAction(ctx context.Context, id string) (*Action, error) {
	var a Action
	if err := s.db.GetContext(ctx, &a, actionSelect+` WHERE a.id = $1`, id); err != nil {
		if errors.Is(err, sqlErrNoRows()) {
			return nil, ErrNotFound
		}
		return nil, fmt.Errorf("get action: %w", err)
	}
	return &a, nil
}

// ListActions returns actions in a status ("" = every status), oldest first.
// Same-second ties break on id (SQLite used rowid, i.e. insert order; no
// existing column records insert order, so the tie-break is deterministic but
// no longer insert order).
func (s *Store) ListActions(ctx context.Context, status string) ([]Action, error) {
	if status == "" || status == ActionQueued {
		if _, err := s.requeueStaleReleaseClaimsAt(ctx, ReleaseHoldClaimRequeueAfter, time.Now()); err != nil {
			return nil, err
		}
	}
	var out []Action
	q, args := actionSelect+` ORDER BY a.created_at ASC, a.id ASC`, []any{}
	if status != "" {
		q, args = actionSelect+` WHERE a.status = $1 ORDER BY a.created_at ASC, a.id ASC`, []any{status}
	}
	if err := s.db.SelectContext(ctx, &out, q, args...); err != nil {
		return nil, fmt.Errorf("list actions: %w", err)
	}
	return out, nil
}

// requeueStaleReleaseClaimsAt recovers only release_hold work whose runner
// stopped after claiming it. Other action kinds retain their existing
// exactly-once behavior. A status-guarded update makes concurrent result
// writeback win safely; the release engine itself tolerates a retry after the
// timeout because it records progress before mutation and checks entered_at.
func (s *Store) requeueStaleReleaseClaimsAt(ctx context.Context, after time.Duration, now time.Time) (int64, error) {
	if after <= 0 {
		after = ReleaseHoldClaimRequeueAfter
	}
	cutoff := now.Add(-after).Unix()
	res, err := s.db.ExecContext(ctx, `UPDATE onboard_actions
		SET status = $1, claimed_at = NULL, claimed_by = '', claim_token = ''
		WHERE kind = $2 AND status = $3 AND claimed_at IS NOT NULL AND claimed_at <= $4`,
		ActionQueued, ActionReleaseHold, ActionClaimed, cutoff)
	if err != nil {
		return 0, fmt.Errorf("requeue stale release claims: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, fmt.Errorf("requeue stale release claims: count affected rows: %w", err)
	}
	return n, nil
}

// ListActionsForProposal returns every action ever queued for one proposal,
// newest first — what the console shows under the onboarded panel.
func (s *Store) ListActionsForProposal(ctx context.Context, proposalID string) ([]Action, error) {
	var out []Action
	if err := s.db.SelectContext(ctx, &out,
		actionSelect+` WHERE a.proposal_id = $1 ORDER BY a.created_at DESC, a.id DESC`, proposalID); err != nil {
		return nil, fmt.Errorf("list actions for proposal: %w", err)
	}
	return out, nil
}

// ClaimAction atomically flips queued → claimed for one runner. Exactly one
// claimer wins; the rest get ErrActionNotQueued (or ErrNotFound).
func (s *Store) ClaimAction(ctx context.Context, id, runnerID string) error {
	if err := s.admitFollowUp7788(ctx, id, ActionClaim7788); err != nil {
		return err
	}
	claimToken, err := newClaimToken()
	if err != nil {
		return fmt.Errorf("claim token: %w", err)
	}
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return fmt.Errorf("claim action: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// Share-lock the parent proposal (when there is one) so an offboard, which
	// locks it FOR UPDATE, cannot pass its "no claimed action" guard while this
	// claim is in flight, and this claim sees an offboard that committed first.
	if _, err := tx.ExecContext(ctx, `SELECT 1 FROM onboard_proposals p JOIN onboard_actions a ON a.proposal_id = p.id
		WHERE a.id = $1 FOR SHARE OF p`, id); err != nil {
		return fmt.Errorf("claim action: lock proposal: %w", err)
	}
	res, err := tx.ExecContext(ctx,
		`UPDATE onboard_actions SET status = $1, claimed_by = $2, claimed_at = $3, claim_token = $4
		 WHERE id = $5 AND status = $6
		 AND (kind = $7 OR EXISTS (SELECT 1 FROM onboard_proposals p WHERE p.id = onboard_actions.proposal_id AND p.status IN ($8, $9)))`,
		ActionClaimed, runnerID, time.Now().Unix(), claimToken, id, ActionQueued, ActionReleaseHold, ProposalOnboarded, ProposalEnrolled)
	if err != nil {
		return fmt.Errorf("claim action: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("claim action: count affected rows: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("claim action: commit: %w", err)
	}
	if n == 1 {
		return nil
	}
	if _, err := s.GetAction(ctx, id); err != nil {
		return err
	}
	return ErrActionNotQueued
}

// SetActionResult records the terminal outcome (done | failed) of a claimed
// action. resultJSON is stored verbatim ("{}" when blank); errMsg is the
// runner's reason on a failed result.
func (s *Store) SetActionResult(ctx context.Context, id, claimToken, status, resultJSON, errMsg string) error {
	if status != ActionDone && status != ActionFailed {
		return fmt.Errorf("set action result: status must be done or failed, got %q", status)
	}
	if err := s.admitFollowUp7788(ctx, id, ActionResult7788); err != nil {
		return err
	}
	if resultJSON == "" {
		resultJSON = "{}"
	}
	res, err := s.db.ExecContext(ctx,
		`UPDATE onboard_actions SET status = $1, result_json = $2, error = $3, finished_at = $4, claim_token = ''
		 WHERE id = $5 AND status = $6 AND ($7 = '' OR claim_token = $8)`,
		status, resultJSON, errMsg, time.Now().Unix(), id, ActionClaimed, claimToken, claimToken)
	if err != nil {
		return fmt.Errorf("set action result: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("set action result: count affected rows: %w", err)
	}
	if n == 1 {
		return nil
	}
	if _, err := s.GetAction(ctx, id); err != nil {
		return err
	}
	return ErrActionNotClaimed
}

// SetReleaseActionResult records the runner result and, for a verified
// released/already-released outcome, closes the exact petition in the same
// transaction. A crash can therefore never show a completed action beside a
// still-open request for that hold.
func (s *Store) SetReleaseActionResult(ctx context.Context, id, claimToken, status, resultJSON, errMsg, peerID string, enteredAt time.Time, actor string, closeRequest bool) error {
	if status != ActionDone && status != ActionFailed {
		return fmt.Errorf("set release result: invalid status %q", status)
	}
	if err := s.admitFollowUp7788(ctx, id, ActionResult7788); err != nil {
		return err
	}
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return fmt.Errorf("begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	res, err := tx.ExecContext(ctx, `UPDATE onboard_actions SET status=$1, result_json=$2, error=$3, finished_at=$4, claim_token='' WHERE id=$5 AND status=$6 AND claim_token=$7`,
		status, resultJSON, errMsg, time.Now().Unix(), id, ActionClaimed, claimToken)
	if err != nil {
		return fmt.Errorf("set release action result: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("set release action result count: %w", err)
	}
	if n != 1 {
		return ErrActionNotClaimed
	}
	if closeRequest {
		if _, err := tx.ExecContext(ctx, `UPDATE probation_reactivation_requests SET status=$1, decided_by=$2, decided_at=$3
			WHERE peer_id=$4 AND hold_entered_at=$5 AND status=$6`, ReactivationClosed, actor, time.Now().UTC().Unix(), peerID, enteredAt.UTC().Unix(), ReactivationOpen); err != nil {
			return fmt.Errorf("close release request: %w", err)
		}
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("commit release result: %w", err)
	}
	return nil
}
