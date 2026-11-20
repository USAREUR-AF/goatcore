package onboardstore

import (
	"context"
	"errors"
	"fmt"
	"strings"
)

// RegisterMintedProposal records an onboarding that happened through the CLI
// mint path (`goat admin onboard`) rather than the console's review queue.
//
// WHY THIS EXISTS. There are two ways somebody gets onboarded and both must
// record the same things: the console path writes the operator manifest and a
// proposal row, so the CLI path writes both too. Every lifecycle lane — the
// ADR 1136 dormancy sweep, the `archived` cold-storage pass,
// `ops/identity/identity-usage.py` — enumerates THIS store, so anyone onboarded
// by CLI without a row here would be invisible to all of them.
//
// THE ROW LANDS AT `onboarded`, NOT `proposed`. The gates exist to decide
// whether to mint; this is called AFTER a mint that already happened, performed
// by an operator who holds mint authority. Writing it as `proposed` would
// invent a decision nobody is waiting to make and would put a live credential
// in a review queue. `Retroactive` is deliberately NOT set — that flag means
// "provenance predates the three-stage system", and this row has provenance:
// the operator who ran the mint.
//
// IDEMPOTENT BY EMAIL. Re-running a mint, or re-running the backfill, must not
// produce a second row for one person — that is exactly the collision the
// email-keyed pre-mint check exists to prevent (ADR 1048). An existing
// non-terminal row wins and is returned unchanged; the caller is told it
// already existed rather than being handed a silent no-op.
func (s *Store) RegisterMintedProposal(ctx context.Context, p *Proposal) (created bool, existing *Proposal, err error) {
	p.Handle = strings.TrimSpace(p.Handle)
	p.Email = strings.TrimSpace(p.Email)
	if p.Handle == "" {
		return false, nil, fmt.Errorf("register minted proposal: empty handle")
	}
	if p.Env == "" {
		return false, nil, fmt.Errorf("register minted proposal: empty env")
	}

	// NO EMAIL MEANS NO EMAIL-KEYED GUARD, so fall back to handle+env. A
	// roster-sourced backfill row carries no address (the committed roster stores
	// email_hash, deliberately, so a readable file cannot leak a mailing list),
	// and the check below is skipped entirely when Email is empty, which would
	// create a second row for the same person. A handle can appear more than once in
	// the roster. handle+env is the right key for that case: it is what every
	// lifecycle lane joins on.
	if p.Email == "" {
		var found Proposal
		err := s.db.GetContext(ctx, &found,
			`SELECT * FROM onboard_proposals
			   WHERE lower(handle) = lower($1 COLLATE "C") AND lower(env) = lower($2 COLLATE "C")
			   ORDER BY created_at DESC LIMIT 1`, p.Handle, p.Env)
		if err == nil {
			return false, &found, nil
		}
		if !errors.Is(err, sqlErrNoRows()) {
			return false, nil, fmt.Errorf("register minted proposal: %w", err)
		}
	}

	// An existing row for this email — in ANY state, including terminal ones —
	// means this person is already recorded. Registering beside a `rejected` or
	// `archived` row would resurrect them by the side door, bypassing the
	// restore path that exists precisely so a shelved person comes back
	// deliberately.
	if p.Email != "" {
		var found Proposal
		err := s.db.GetContext(ctx, &found,
			`SELECT * FROM onboard_proposals WHERE lower(email) = lower($1 COLLATE "C")
			   ORDER BY created_at DESC LIMIT 1`, p.Email)
		if err == nil {
			return false, &found, nil
		}
		if !errors.Is(err, sqlErrNoRows()) {
			return false, nil, fmt.Errorf("register minted proposal: %w", err)
		}
	}

	if p.Status == "" {
		p.Status = ProposalOnboarded
	}
	if p.ManifestJSON == "" {
		p.ManifestJSON = "{}"
	}
	if p.Note == "" {
		p.Note = "registered by the CLI mint path (goat admin onboard); no review gate applies"
	}
	// GHSA-7788: the same admission check as the runner's mint queue.
	if err := s.admitRegister7788(ctx, p); err != nil {
		return false, nil, err
	}
	if err := s.CreateProposal(ctx, p); err != nil {
		return false, nil, fmt.Errorf("register minted proposal: %w", err)
	}
	// CreateProposal defaults an unset status to `proposed`; force the real one.
	if p.Status != ProposalProposed {
		if _, err := s.db.ExecContext(ctx,
			`UPDATE onboard_proposals SET status = $1 WHERE id = $2`, p.Status, p.ID); err != nil {
			return false, nil, fmt.Errorf("register minted proposal status: %w", err)
		}
	}
	return true, nil, nil
}
