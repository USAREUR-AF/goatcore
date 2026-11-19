package onboardstore

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"
)

// The holder of an operator key is the Kratos identity it was issued to,
// recorded on the row when the key is collected (GHSA-mxf4-8rv7-774w). The
// console's live machine-realm gate then reads that identity by id. Without
// it the gate had to join the key's handle to a person through the roster,
// and handles are not unique, so the join could be made ambiguous or pointed
// at somebody else.

// OperatorKeyHolder is the part of an operator-key row the holder binding and
// the owner gate need.
type OperatorKeyHolder struct {
	ID               string `db:"id"`
	Handle           string `db:"handle"`
	Email            string `db:"email"`
	Status           string `db:"status"`
	Fingerprint      string `db:"fingerprint"`
	HolderIdentityID string `db:"holder_identity_id"`
}

// SetOperatorKeyHolder records the Kratos identity id a key was issued to.
func (s *Store) SetOperatorKeyHolder(ctx context.Context, id, holderIdentityID string) error {
	res, err := s.db.ExecContext(ctx, `UPDATE operator_keys SET holder_identity_id = $1 WHERE id = $2`,
		strings.TrimSpace(holderIdentityID), id)
	if err != nil {
		return fmt.Errorf("record operator key holder: %w", err)
	}
	if n, _ := res.RowsAffected(); n == 0 { //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		return ErrNotFound
	}
	return nil
}

// OperatorKeyHolderOf returns one row's holder fields.
func (s *Store) OperatorKeyHolderOf(ctx context.Context, id string) (*OperatorKeyHolder, error) {
	var h OperatorKeyHolder
	err := s.db.GetContext(ctx, &h, `SELECT id, handle, email, status, fingerprint, holder_identity_id
	                                    FROM operator_keys WHERE id = $1`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("read operator key holder: %w", err)
	}
	return &h, nil
}

// OperatorKeyHoldersForHandle returns every operator-key row filed under a
// handle (case-insensitive), any status, newest first.
func (s *Store) OperatorKeyHoldersForHandle(ctx context.Context, handle string) ([]OperatorKeyHolder, error) {
	var out []OperatorKeyHolder
	err := s.db.SelectContext(ctx, &out, `SELECT id, handle, email, status, fingerprint, holder_identity_id
	                                        FROM operator_keys
	                                       WHERE lower(handle) = lower($1 COLLATE "C")
	                                       ORDER BY created_at DESC, id DESC`, strings.TrimSpace(handle))
	if err != nil {
		return nil, fmt.Errorf("list operator key holders: %w", err)
	}
	return out, nil
}

// ProposalsUnderHandle returns every onboarding record filed under a handle
// (case-insensitive), any status, newest first.
func (s *Store) ProposalsUnderHandle(ctx context.Context, handle string) ([]Proposal, error) {
	var out []Proposal
	err := s.db.SelectContext(ctx, &out, `SELECT * FROM onboard_proposals
	                                       WHERE lower(handle) = lower($1 COLLATE "C")
	                                       ORDER BY `+newestFirst, strings.TrimSpace(handle))
	if err != nil {
		return nil, fmt.Errorf("list proposals under handle: %w", err)
	}
	return out, nil
}

// ProposalPending reports whether a record has not yet passed the admin gate
// (proposed, sponsor-approved). A pending record can be filed by anyone
// holding an invite, so it never names a key's owner.
func ProposalPending(status string) bool {
	return status == ProposalProposed || status == ProposalSponsorApproved
}

// ProposalNonTerminal reports whether a record still holds its handle: every
// status but rejected, failed and offboarded, the same set the active
// lookups use.
func ProposalNonTerminal(status string) bool {
	switch status {
	case ProposalRejected, ProposalFailed, ProposalOffboarded:
		return false
	}
	return true
}

// ProposalsForEmail returns every onboarding record filed for an address
// (case-insensitive), any status, newest first.
func (s *Store) ProposalsForEmail(ctx context.Context, email string) ([]Proposal, error) {
	var out []Proposal
	err := s.db.SelectContext(ctx, &out, `SELECT * FROM onboard_proposals
	                                       WHERE lower(email) = lower($1 COLLATE "C")
	                                       ORDER BY `+newestFirst, strings.TrimSpace(email))
	if err != nil {
		return nil, fmt.Errorf("list proposals for email: %w", err)
	}
	return out, nil
}

// ErrOperatorKeyHolderMismatch refuses to hand a key to an identity other
// than the one it is already bound to.
var ErrOperatorKeyHolderMismatch = errors.New("this key is bound to a different identity")

// CollectOperatorKeyAs is CollectOperatorKey that, in the same transaction and
// only once the row is shown to be the collector's, records the identity the
// key is issued to. The row must carry the collector's handle, be active (an
// approved, issued key: never a request) and be undelivered; a row already
// bound to a different identity is refused. Every refusal writes nothing, so
// collecting somebody else's row id can neither bind it nor lock it. An empty
// holderIdentityID binds nothing (the console's live identity check is off).
func (s *Store) CollectOperatorKeyAs(ctx context.Context, id, handle, holderIdentityID string) (string, error) {
	holderIdentityID = strings.TrimSpace(holderIdentityID)
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return "", fmt.Errorf("collect operator key: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var row struct {
		Handle      string `db:"handle"`
		Status      string `db:"status"`
		Secret      string `db:"secret"`
		DeliveredAt int64  `db:"delivered_at"`
		Holder      string `db:"holder_identity_id"`
	}
	err = tx.GetContext(ctx, &row, `SELECT handle, status, secret, delivered_at, holder_identity_id
	        FROM operator_keys WHERE id = $1 FOR UPDATE`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return "", ErrNotFound
	}
	if err != nil {
		return "", fmt.Errorf("collect operator key: %w", err)
	}
	if !strings.EqualFold(row.Handle, strings.TrimSpace(handle)) {
		return "", ErrNotFound
	}
	if row.Status != OperatorKeyActive {
		return "", fmt.Errorf("%w: %s", ErrOperatorKeyWrongState, row.Status)
	}
	if row.Secret == "" || row.DeliveredAt != 0 {
		return "", ErrOperatorKeyAlreadyDelivered
	}
	bound := strings.TrimSpace(row.Holder)
	if holderIdentityID != "" && bound != "" && !strings.EqualFold(bound, holderIdentityID) {
		return "", ErrOperatorKeyHolderMismatch
	}
	if bound == "" {
		bound = holderIdentityID
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE operator_keys SET secret = '', delivered_at = $1, holder_identity_id = $2 WHERE id = $3`,
		time.Now().Unix(), bound, id); err != nil {
		return "", fmt.Errorf("mark operator key delivered: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return "", fmt.Errorf("mark operator key delivered: commit: %w", err)
	}
	return row.Secret, nil
}
