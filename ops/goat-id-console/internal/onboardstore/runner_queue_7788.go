package onboardstore

// Runner-queue admission (GHSA-7788). The onboard-runner mints a login for
// every proposal in status `approved` (it lists and claims them), writing the
// manifest's ory_site.groups to the new identity. Every write that moves a
// proposal into `approved` therefore passes one check, whatever route asked
// for it: approve, restore, retry, a status write or a create. The console
// installs the check (SetRunnerQueueCheck7788) with the request's principal
// rules; the store only guarantees it runs at the state write.

import (
	"context"
	"database/sql"
	"errors"
	"sync"
)

// RunnerQueueCheck decides whether p may enter the runner's mint queue for
// the request carried by ctx. A non-nil error refuses the write.
type RunnerQueueCheck func(ctx context.Context, p *Proposal) error

// RunnerQueueRefusal is the error a refused admission returns; callers map it
// to 403.
type RunnerQueueRefusal struct{ Msg string }

func (e *RunnerQueueRefusal) Error() string { return e.Msg }

// runnerQueueChecks holds each store's check. It is keyed by store so that
// two stores in one process (tests) never share a check.
var runnerQueueChecks sync.Map // *Store -> RunnerQueueCheck

// SetRunnerQueueCheck7788 installs the admission check for s.
func (s *Store) SetRunnerQueueCheck7788(f RunnerQueueCheck) {
	if f == nil {
		runnerQueueChecks.Delete(s)
		return
	}
	runnerQueueChecks.Store(s, f)
}

// admitRunnerQueue7788 runs s's check on p when status is `approved`.
func (s *Store) admitRunnerQueue7788(ctx context.Context, p *Proposal, status string) error {
	if status != ProposalApproved || p == nil {
		return nil
	}
	f, ok := runnerQueueChecks.Load(s)
	if !ok {
		return nil
	}
	return f.(RunnerQueueCheck)(ctx, p)
}

// RunnerActionCheck decides whether the request in ctx may run a runner-side
// lifecycle action (claim, result, archive) on p. A non-nil error refuses.
type RunnerActionCheck func(ctx context.Context, p *Proposal, action string) error

var runnerActionChecks sync.Map // *Store -> RunnerActionCheck

// SetRunnerActionCheck7788 installs the lifecycle-action check for s.
func (s *Store) SetRunnerActionCheck7788(f RunnerActionCheck) {
	if f == nil {
		runnerActionChecks.Delete(s)
		return
	}
	runnerActionChecks.Store(s, f)
}

// Lifecycle actions passed to the RunnerActionCheck.
const (
	ActionClaim7788    = "claim"
	ActionResult7788   = "result"
	ActionArchive7788  = "archive"
	ActionOffboard7788 = "offboard"
)

// admitAction7788 loads row id and runs s's lifecycle-action check on it.
// An unknown id is left to the write; any other lookup error refuses.
func (s *Store) admitAction7788(ctx context.Context, id, action string) error {
	f, ok := runnerActionChecks.Load(s)
	if !ok {
		return nil
	}
	p, err := s.GetProposal(ctx, id)
	if errors.Is(err, ErrNotFound) {
		return nil
	}
	if err != nil {
		return err
	}
	return f.(RunnerActionCheck)(ctx, p, action)
}

// admitFollowUp7788 runs the lifecycle-action check for a follow-up action
// (a mesh-key reissue, a hold release) on the proposal it belongs to:
// claiming one, or posting its result, writes that person's mesh keys or
// re-sends their package. An action with no proposal (a peer hold) has no
// row to rank and is left alone; an unknown action is left to the write.
func (s *Store) admitFollowUp7788(ctx context.Context, actionID, action string) error {
	if _, ok := runnerActionChecks.Load(s); !ok {
		return nil
	}
	a, err := s.GetAction(ctx, actionID)
	if errors.Is(err, ErrNotFound) {
		return nil
	}
	if err != nil {
		return err
	}
	if a.ProposalID == "" {
		return nil
	}
	return s.admitAction7788(ctx, a.ProposalID, action)
}

// admitRegister7788 runs s's check on a row RegisterMintedProposal is about
// to record. The register records a login as already minted and reviewed by
// nobody, so it is held to the same rule as entering the mint queue: the
// row must not record an admin-bearing login the credential could not have
// had approved.
func (s *Store) admitRegister7788(ctx context.Context, p *Proposal) error {
	f, ok := runnerQueueChecks.Load(s)
	if !ok || p == nil {
		return nil
	}
	return f.(RunnerQueueCheck)(context.WithValue(ctx, registerCtxKey7788{}, true), p)
}

type registerCtxKey7788 struct{}

// IsRegisterContext7788 reports whether the check is running for a register
// (bookkeeping of a login already minted) rather than a write into the mint
// queue.
func IsRegisterContext7788(ctx context.Context) bool {
	v, _ := ctx.Value(registerCtxKey7788{}).(bool)
	return v
}

// execAdmitted7788 runs update (whose WHERE names row id) as a write to
// status. When status is `approved` and a check is installed, the row is
// read FOR UPDATE, checked and written in one transaction, so the check and
// the write see the same row (as reenterApproved does for restore and
// retry). An unknown id runs the UPDATE, which matches nothing, so the
// caller reports not-found as before.
func (s *Store) execAdmitted7788(ctx context.Context, id, status, update string, args ...any) (sql.Result, error) {
	f, ok := runnerQueueChecks.Load(s)
	if status != ProposalApproved || !ok {
		return s.db.ExecContext(ctx, update, args...)
	}
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return nil, err
	}
	defer func() { _ = tx.Rollback() }()
	var row Proposal
	err = tx.GetContext(ctx, &row, `SELECT * FROM onboard_proposals WHERE id = $1 FOR UPDATE`, id)
	switch {
	case errors.Is(err, sql.ErrNoRows):
	case err != nil:
		return nil, err
	default:
		if err := f.(RunnerQueueCheck)(ctx, &row); err != nil {
			return nil, err
		}
	}
	res, err := tx.ExecContext(ctx, update, args...)
	if err != nil {
		return nil, err
	}
	if err := tx.Commit(); err != nil {
		return nil, err
	}
	return res, nil
}

// admitRunnerQueueByID7788 loads the row and runs the check when status is
// `approved`. An unknown id is left to the write, which reports it.
func (s *Store) admitRunnerQueueByID7788(ctx context.Context, id, status string) error {
	if status != ProposalApproved {
		return nil
	}
	if _, ok := runnerQueueChecks.Load(s); !ok {
		return nil
	}
	p, err := s.GetProposal(ctx, id)
	if errors.Is(err, ErrNotFound) {
		return nil // the write itself answers not-found
	}
	if err != nil {
		return err // fail closed: no write into the mint queue unchecked
	}
	return s.admitRunnerQueue7788(ctx, p, status)
}
