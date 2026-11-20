package onboardstore

import (
	"context"
	"fmt"
	"time"
)

// offboardTransition closes eligibility and cancels queued actions together.
// Claimed work remains visible until the runner acknowledges termination;
// ClaimOffboard will not perform the final cut while a mint can still run.
func (s *Store) offboardTransition(ctx context.Context, id, query string, args ...any) (bool, error) {
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return false, fmt.Errorf("begin offboard: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// Lock the proposal row first. The guarded UPDATE below then reads the
	// actions table with a snapshot taken after the lock, and ClaimAction takes
	// the same row lock (FOR SHARE) before it claims, so a reissue cannot be
	// claimed between this guard and the cancel of queued actions.
	if _, err := tx.ExecContext(ctx, `SELECT 1 FROM onboard_proposals WHERE id = $1 FOR UPDATE`, id); err != nil {
		return false, fmt.Errorf("lock proposal for offboard: %w", err)
	}
	res, err := tx.ExecContext(ctx, query, args...)
	if err != nil {
		return false, fmt.Errorf("transition offboard: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	if n == 0 {
		return false, nil
	}
	_, err = tx.ExecContext(ctx, `UPDATE onboard_actions SET status = $1, error = $2, finished_at = $3
        WHERE proposal_id = $4 AND status = $5`, ActionFailed, "canceled by offboarding", time.Now().Unix(), id, ActionQueued)
	if err != nil {
		return false, fmt.Errorf("cancel queued reissues: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return false, fmt.Errorf("commit offboard: %w", err)
	}
	return true, nil
}

// FinishReissue commits the action and replacement credential in one transaction.
// A late result acknowledges that minting has stopped but can never publish a
// credential. The offboard runner can then make its final revocation cut.
// Credentials remain on the proposal, never in action metadata.
func (s *Store) FinishReissue(ctx context.Context, id, status, resultJSON, errMsg, setupKey, handsetKey string) (*Proposal, error) {
	return s.finishReissue(ctx, id, status, resultJSON, errMsg, setupKey, handsetKey, "", false)
}

// BeginReissueDelivery reserves one result and keeps the final offboard cut
// blocked until all credential-producing notification work has stopped.
func (s *Store) BeginReissueDelivery(ctx context.Context, id, runnerID, status, resultJSON, errMsg, setupKey, handsetKey string) (*Proposal, error) {
	if runnerID == "" {
		return nil, ErrActionNotClaimed
	}
	return s.finishReissue(ctx, id, status, resultJSON, errMsg, setupKey, handsetKey, runnerID, true)
}

func (s *Store) finishReissue(ctx context.Context, id, status, resultJSON, errMsg, setupKey, handsetKey, runnerID string, deliver bool) (*Proposal, error) {
	if status != ActionDone && status != ActionFailed {
		return nil, fmt.Errorf("invalid action result status %q", status)
	}
	if err := s.admitFollowUp7788(ctx, id, ActionResult7788); err != nil {
		return nil, err
	}
	if resultJSON == "" {
		resultJSON = "{}"
	}
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("begin reissue result: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// Write first: the guarded UPDATE locks the action row, then the parent
	// proposal is read FOR UPDATE so its status cannot change before commit.
	reservedStatus := status
	if deliver && status == ActionDone {
		reservedStatus = ActionDelivering
	}
	res, err := tx.ExecContext(ctx, `UPDATE onboard_actions SET status = $1, result_json = $2, error = $3, finished_at = CASE WHEN $4 = 'delivering' THEN NULL ELSE $5::bigint END
        WHERE id = $6 AND status = $7 AND ($8 = '' OR claimed_by = $9)`, reservedStatus, resultJSON, errMsg, reservedStatus, time.Now().Unix(), id, ActionClaimed, runnerID, runnerID)
	if err != nil {
		return nil, fmt.Errorf("finish action: %w", err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return nil, err
	}
	if n == 0 {
		return nil, ErrActionNotClaimed
	}
	var p Proposal
	if err := tx.GetContext(ctx, &p, `SELECT p.* FROM onboard_proposals p JOIN onboard_actions a ON a.proposal_id = p.id WHERE a.id = $1 FOR UPDATE OF p`, id); err != nil {
		return nil, fmt.Errorf("load reissue parent: %w", err)
	}
	refused := status == ActionDone && !actionEligible(p.Status)
	if refused {
		_, err = tx.ExecContext(ctx, `UPDATE onboard_actions SET status = $1, result_json = '{}', error = $2, finished_at = $3 WHERE id = $4`, ActionFailed, "reissue result refused: proposal is no longer onboarded", time.Now().Unix(), id)
	} else if status == ActionDone && (setupKey != "" || handsetKey != "") {
		_, err = tx.ExecContext(ctx, `UPDATE onboard_proposals SET
            setup_key = CASE WHEN $1 <> '' THEN $2 ELSE setup_key END,
            handset_key = CASE WHEN $3 <> '' THEN $4 ELSE handset_key END WHERE id = $5`, setupKey, setupKey, handsetKey, handsetKey, p.ID)
		if setupKey != "" {
			p.SetupKey = setupKey
		}
		if handsetKey != "" {
			p.HandsetKey = handsetKey
		}
	}
	if err != nil {
		return nil, fmt.Errorf("store reissue result: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("commit reissue result: %w", err)
	}
	if refused {
		return nil, ErrActionNotOnboarded
	}
	return &p, nil
}

// RecordActionDelivery records notification metadata only after a committed
// eligible result; it cannot mutate credentials or reopen a terminal action.
func (s *Store) RecordActionDelivery(ctx context.Context, id, resultJSON string) error {
	res, err := s.db.ExecContext(ctx, `UPDATE onboard_actions SET result_json = $1, status = $2, finished_at = $3 WHERE id = $4 AND status = $5`, resultJSON, ActionDone, time.Now().Unix(), id, ActionDelivering)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n != 1 {
		return ErrActionNotClaimed
	}
	return nil
}
