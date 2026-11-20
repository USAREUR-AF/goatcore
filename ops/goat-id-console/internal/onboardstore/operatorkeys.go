package onboardstore

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// Operator-key lifecycle. Self-service REQUEST, admin-gated APPROVE, live in
// the portal within one reload interval.
//
// An operator key (`op_…`) is what makes a Portal admin action attributable to
// a named person instead of to one shared credential. Until now, issuing one
// meant editing a gitignored envrc on somebody's laptop and redeploying the
// portal — which meant the path of least resistance was to share a key that
// already worked, defeating the attribution the key exists to provide. The
// friction was not a control; the control is a human approving.
//
// Same two-gate shape as SSH keys, and for the same reason: the console is
// reachable with an ordinary web session, so requesting must never authorize.
//
//	requested -> active -> revoked
//	requested -> rejected
const (
	OperatorKeyRequested = "requested"
	OperatorKeyActive    = "active"
	OperatorKeyRejected  = "rejected"
	OperatorKeyRevoked   = "revoked"
)

// ErrOperatorKeyWrongState signals a transition that does not apply to the
// row's current state — a conflict, not a not-found.
var ErrOperatorKeyWrongState = errors.New("operator key is not in the required state")

// ErrOperatorKeyAlreadyDelivered is returned when the secret has already been
// shown once and is therefore gone. It is a normal outcome, not a fault.
var ErrOperatorKeyAlreadyDelivered = errors.New("operator key was already delivered; revoke and request a new one")

// ErrOperatorKeySelfDecision refuses an approve/reject whose decider is the
// row's own requester (#210). Approval IS the grant — the file lands and the
// portal hot-reloads — so a self-approval is one person minting themselves a
// live credential with no second human. The rule lives here, in the store,
// so the browser door and the API door cannot drift into two different rules.
// Self-REVOKE is deliberately not covered: removing your own live key is
// de-escalation, and slowing that down only hurts credential hygiene.
var ErrOperatorKeySelfDecision = errors.New("operator key requests need a different admin: requester and decider are the same person")

// ErrOperatorKeyUnverifiedDecider refuses a decision that CANNOT BE SHOWN to
// involve two people (#210 follow-up). Absence of a match only proves
// difference when the two identities were comparable to begin with: a decider
// known only by email, judging a row filed only under a handle, produces a
// non-match that means nothing. Refusing there is the same fail-closed stance
// the self-decision rule takes — an unverifiable second person is not a
// second person. The message names the door that always resolves an identity,
// so a blocked admin is redirected rather than dead-ended.
var ErrOperatorKeyUnverifiedDecider = errors.New("operator key decision refused: the decider could not be shown to be a different person")

// ErrOperatorKeyReservedHandle refuses a key for a service label. The portal
// lets a key under "operator", "id-console" or "onboard-api" name another
// person in X-Goat-Operator-Actor, because those labels belong to services
// that authenticate the human themselves (GHSA-q7qx). A person's key filed
// under one would let its holder record actions as anyone. The portal also
// refuses these labels from its key directory; this keeps the console from
// issuing a key the portal will not honour, and from being the way one gets
// there if the portal rule ever regresses.
var ErrOperatorKeyReservedHandle = errors.New("that handle is a reserved service label and cannot hold a person's operator key")

// reservedOperatorKeyHandles are the service labels the portal (actorOf) and
// this console (isServiceSponsorLabel) trust to delegate attribution.
var reservedOperatorKeyHandles = []string{"operator", "id-console", "onboard-api"}

// IsReservedOperatorKeyHandle reports whether handle is a service label.
// Case-insensitive: key collection matches handles with EqualFold, and the
// key file name is the handle.
func IsReservedOperatorKeyHandle(handle string) bool {
	h := strings.TrimSpace(handle)
	for _, r := range reservedOperatorKeyHandles {
		if strings.EqualFold(h, r) {
			return true
		}
	}
	return false
}

// OperatorKey is one person's request for an attributable portal key.
//
// Secret handling. The key VALUE lives in this row only between approval and
// its single delivery to the requester, after which the column is emptied and
// only the fingerprint remains. The portal never reads this table — it reads
// the key file the console writes — so a row with no Secret is fully
// functional. That the value is short-lived here is deliberate: a console
// database that permanently held live bearer tokens would be a strictly worse
// thing to lose than one that holds fingerprints.
type OperatorKey struct {
	ID          string `db:"id"           json:"id"`
	CreatedAt   int64  `db:"created_at"   json:"created_at"`
	Handle      string `db:"handle"       json:"handle"`
	Email       string `db:"email"        json:"email,omitempty"`
	Env         string `db:"env"          json:"env,omitempty"`
	Reason      string `db:"reason"       json:"reason,omitempty"`
	Status      string `db:"status"       json:"status"`
	Fingerprint string `db:"fingerprint"  json:"fingerprint,omitempty"`
	// Secret is the `op_…` value. Present only between approval and delivery.
	Secret      string `db:"secret"       json:"-"`
	DecidedBy   string `db:"decided_by"   json:"decided_by,omitempty"`
	DecidedAt   int64  `db:"decided_at"   json:"decided_at,omitempty"`
	DeliveredAt int64  `db:"delivered_at" json:"delivered_at,omitempty"`
	RevokedBy   string `db:"revoked_by"   json:"revoked_by,omitempty"`
	RevokedAt   int64  `db:"revoked_at"   json:"revoked_at,omitempty"`
	Note        string `db:"note"         json:"note,omitempty"`
	// RequestedBy is the principal that filed the request (the machine
	// principal's handle, or the browser session's email), kept in
	// operator_key_requesters. The two-person rule refuses it as the decider,
	// so one principal cannot request and approve the same key (GHSA-2w7v).
	// Empty when no filer was recorded (a request filed before this existed,
	// or imported from SQLite): such a pending request cannot be decided.
	RequestedBy string `db:"requested_by" json:"requested_by,omitempty"`
}

type operatorKeyRequesterKey struct{}

// WithOperatorKeyRequester records on ctx the principal filing an operator-key
// request; RequestOperatorKey stores it on the new row as RequestedBy.
func WithOperatorKeyRequester(ctx context.Context, principal string) context.Context {
	return context.WithValue(ctx, operatorKeyRequesterKey{}, strings.TrimSpace(principal))
}

func operatorKeyRequesterFrom(ctx context.Context) string {
	v, _ := ctx.Value(operatorKeyRequesterKey{}).(string)
	return v
}

// Deliverable reports whether the requester can still collect their key.
func (k OperatorKey) Deliverable() bool {
	return k.Status == OperatorKeyActive && k.Secret != "" && k.DeliveredAt == 0
}

// Issued reports whether the console ever issued a key for this row: the
// fingerprint is written at approval and kept through revocation. Only an
// issued row's revoke writes a tombstone (GHSA-2w7v).
func (k OperatorKey) Issued() bool { return k.Fingerprint != "" }

// ErrOperatorKeyBadHandle refuses a request under a handle a revocation
// tombstone cannot name (whitespace, '#' or ':'), GHSA-2w7v.
var ErrOperatorKeyBadHandle = errors.New("operator key handle is not valid")

// ErrOperatorKeyLegacyRequest refuses a decision on a pending request filed
// with no recorded filer (filed before the console recorded it, or imported
// from a SQLite checkpoint): the
// two-person rule cannot show the decider differs from the filer, so the row
// must be withdrawn and requested again (GHSA-2w7v review r3). It wraps
// ErrOperatorKeyUnverifiedDecider, so every door refuses it as unverifiable.
// operatorKeyRequesterDirect is recorded when trusted code files a request
// through the store with no principal on ctx (the doors always set one), so
// such a row is decided under the ordinary rule rather than refused.
const operatorKeyRequesterDirect = "(filed directly through the store)"

var ErrOperatorKeyLegacyRequest = errors.New("this request predates recording who filed it, so the two-person rule cannot be checked: withdraw it (revoke) and request again after the upgrade")

// identityIsEmail says which DIMENSION an identity string lives in. Handles
// are the registry's short names and never contain "@"; emails always do.
// That is the whole of the test — nothing here validates an address, it only
// needs to know which set a value could ever have matched.
func identityIsEmail(v string) bool { return strings.Contains(v, "@") }

// trimNonEmpty drops blank and whitespace-only identity forms.
func trimNonEmpty(ids []string) []string {
	out := make([]string, 0, len(ids))
	for _, id := range ids {
		if v := strings.TrimSpace(id); v != "" {
			out = append(out, v)
		}
	}
	return out
}

// comparableIdentities reports whether two identity sets overlap in at least
// one dimension — that is, whether a non-match between them carries any
// information at all. An email-only set against a handle-only set could not
// have matched however identical the people behind them are, so treating its
// non-match as "two people" is the vacuous check this exists to catch.
func comparableIdentities(a, b []string) bool {
	aEmail, aHandle := false, false
	for _, v := range a {
		if identityIsEmail(v) {
			aEmail = true
		} else {
			aHandle = true
		}
	}
	for _, v := range b {
		if identityIsEmail(v) {
			if aEmail {
				return true
			}
			continue
		}
		if aHandle {
			return true
		}
	}
	return false
}

// identityDimension names a set's dimension for a refusal message somebody
// has to act on: "email", "handle", or both when the set spans them.
func identityDimension(ids []string) string {
	email, handle := false, false
	for _, v := range ids {
		if identityIsEmail(v) {
			email = true
		} else {
			handle = true
		}
	}
	switch {
	case email && handle:
		return "email and handle"
	case email:
		return "email"
	default:
		return "handle"
	}
}

// ownerIdentities is every form of the ROW OWNER's identity the console can
// establish. A row filed through the browser carries handle AND email, but
// `goat admin operator-key request --handle x` records a handle and nothing
// else — and against a decider known only by email such a row produced a
// vacuous non-match, which is precisely how a self-approval could still slip
// through. So when the row records no email, resolve one from the owner's
// onboarding record before comparing. Bounded to email-less rows on purpose:
// a row that already carries an email needs no extra query.
func (s *Store) ownerIdentities(ctx context.Context, row *OperatorKey) []string {
	// The principal that filed the request counts as an owner too: approval
	// needs a principal other than the requester, not only other than the
	// handle the key is for (GHSA-2w7v). trimNonEmpty drops it when unset.
	ids := []string{row.Handle, row.RequestedBy}
	if e := strings.TrimSpace(row.Email); e != "" {
		return trimNonEmpty(append(ids, e))
	}
	if e, err := s.ProposalEmailForHandle(ctx, row.Handle); err == nil && e != "" {
		ids = append(ids, e)
	}
	return trimNonEmpty(ids)
}

// OperatorKeyDecisionCheck is the two-person rule (#210), and the ONE place
// that answers "may this decider decide this row". Approve and reject call it
// before mutating; the queue page calls it to render each row — so what the
// page offers and what the store permits are one answer, not two
// implementations of one intention.
//
// deciderIDs is every form the door knows this session by: the browser's
// oauth2-proxy email plus the handle it resolves to, or the API principal's
// per-handle attribution. There is deliberately no exported "does any
// form match" helper any more — that predicate reads as an answer while
// being only half of one, and reaching for it is exactly how the first cut
// let a self-approval through.
//
// Three outcomes, fail-closed by construction:
//
//	nil                             — a different person, provably
//	ErrOperatorKeySelfDecision      — the decider IS the owner
//	ErrOperatorKeyUnverifiedDecider — they cannot be told apart, so refuse
func (s *Store) OperatorKeyDecisionCheck(ctx context.Context, row *OperatorKey, deciderIDs []string) error {
	if row.Status == OperatorKeyRequested && strings.TrimSpace(row.RequestedBy) == "" {
		return fmt.Errorf("%w: %w", ErrOperatorKeyUnverifiedDecider, ErrOperatorKeyLegacyRequest)
	}
	decider := trimNonEmpty(deciderIDs)
	if len(decider) == 0 {
		return fmt.Errorf("%w: no decider identity was presented", ErrOperatorKeyUnverifiedDecider)
	}
	owner := s.ownerIdentities(ctx, row)
	if len(owner) == 0 {
		return fmt.Errorf("%w: this request records no owner identity to compare against", ErrOperatorKeyUnverifiedDecider)
	}
	// Same person under a second spelling counts: case, a plus-address tag
	// and lookalike characters are folded (selfcheck), not only compared
	// with EqualFold.
	for _, d := range decider {
		for _, o := range owner {
			if strings.EqualFold(d, o) || selfcheck.Same(d, o) {
				return ErrOperatorKeySelfDecision
			}
		}
	}
	if !comparableIdentities(decider, owner) {
		return fmt.Errorf("%w: this session is known only by %s and the request only by %s, so the two cannot be compared",
			ErrOperatorKeyUnverifiedDecider, identityDimension(decider), identityDimension(owner))
	}
	return nil
}

// mintOperatorSecret returns a new `op_` key. 32 hex characters of crypto/rand
// — the same shape the hand-issued keys used (`openssl rand -hex 16`), so
// nothing downstream has to learn a new format for keys that are now minted
// rather than typed.
func mintOperatorSecret() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("mint operator key: %w", err)
	}
	return "op_" + hex.EncodeToString(b), nil
}

// OperatorKeyFingerprint is what surfaces and audit records refer to. It is a
// hash prefix rather than the key: a fingerprint identifies a credential in a
// log without being one.
func OperatorKeyFingerprint(secret string) string {
	sum := sha256.Sum256([]byte(secret))
	return "sha256:" + hex.EncodeToString(sum[:])[:16]
}

// newOperatorKeyID mirrors newSSHKeyID: a short random id with a prefix that
// names the thing, so an id in a log or a URL is self-describing.
func newOperatorKeyID() (string, error) {
	var b [8]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return "opk-" + hex.EncodeToString(b[:]), nil
}

// RequestOperatorKey records a request. It mints nothing: a request that
// already carried a secret would make "requested" indistinguishable from
// "granted" for anyone who got hold of the row.
//
// Re-requesting while one is already pending or active returns the existing row
// rather than stacking duplicates — two live keys for one person is two things
// to revoke and one of them will be missed.
func (s *Store) RequestOperatorKey(ctx context.Context, k *OperatorKey) (*OperatorKey, error) {
	handle := strings.TrimSpace(k.Handle)
	if handle == "" {
		return nil, fmt.Errorf("operator key request needs a handle")
	}
	if IsReservedOperatorKeyHandle(handle) {
		return nil, fmt.Errorf("%w: %q", ErrOperatorKeyReservedHandle, handle)
	}
	if existing, err := s.LiveOperatorKeyFor(ctx, handle, k.Env); err != nil {
		return nil, err
	} else if existing != nil {
		return existing, nil
	}
	// GHSA-2w7v: the handle must be one a revocation tombstone can name, or the
	// key could be issued and never fully revoked.
	if strings.HasPrefix(handle, ".") || strings.ContainsAny(handle, " \t\r\n#:/\\") || strings.Contains(handle, "..") { // same rule as opkeys.CheckHandle
		return nil, fmt.Errorf("%w: %q starts with '.' or has whitespace, '#', ':', '/', '\\' or '..'", ErrOperatorKeyBadHandle, handle)
	}
	id, err := newOperatorKeyID()
	if err != nil {
		return nil, err
	}
	now := time.Now().Unix()
	row := &OperatorKey{
		ID:        id,
		CreatedAt: now,
		Handle:    handle,
		Email:     strings.TrimSpace(k.Email),
		Env:       strings.TrimSpace(k.Env),
		Reason:    strings.TrimSpace(k.Reason),
		Status:    OperatorKeyRequested,
	}
	row.RequestedBy = operatorKeyRequesterFrom(ctx)
	if row.RequestedBy == "" {
		row.RequestedBy = operatorKeyRequesterDirect
	}
	// The filer is written first: a key row without one is refused, never
	// decided under a weaker rule.
	if _, err := s.db.ExecContext(ctx, `INSERT INTO operator_key_requesters (id, requested_by) VALUES ($1, $2)`,
		row.ID, row.RequestedBy); err != nil {
		return nil, fmt.Errorf("record operator key requester: %w", err)
	}
	_, err = s.db.ExecContext(ctx, `
		INSERT INTO operator_keys (id, created_at, handle, email, env, reason, status)
		VALUES ($1, $2, $3, $4, $5, $6, $7)`,
		row.ID, row.CreatedAt, row.Handle, row.Email, row.Env, row.Reason, row.Status)
	if err != nil {
		return nil, fmt.Errorf("insert operator key: %w", err)
	}
	return row, nil
}

// ApproveOperatorKey mints the secret and marks the row active. The caller is
// responsible for writing the key file the portal reads — approval that did not
// reach the portal would leave a person holding a credential the fabric refuses,
// which reads to them as "the system is broken".
//
// deciderIDs are every identity form the door knows for the decider, on top
// of decidedBy itself; OperatorKeyDecisionCheck refuses both a decider who IS
// the owner and one who cannot be told apart from them (#210).
//
// State is checked FIRST. A row that is already decided cannot be decided
// again by anybody, so answering "that is your own request" to an owner
// re-clicking a stale Approve button would name the wrong reason — the two-
// person rule is about who may decide, not about a decision already made.
func (s *Store) ApproveOperatorKey(ctx context.Context, id, decidedBy string, deciderIDs []string) (*OperatorKey, error) {
	row, err := s.OperatorKey(ctx, id)
	if err != nil {
		return nil, err
	}
	if row.Status != OperatorKeyRequested {
		return nil, fmt.Errorf("%w: %s", ErrOperatorKeyWrongState, row.Status)
	}
	// A row filed before RequestOperatorKey refused service labels must not
	// be approvable either.
	if IsReservedOperatorKeyHandle(row.Handle) {
		return nil, fmt.Errorf("%w: %q", ErrOperatorKeyReservedHandle, row.Handle)
	}
	if err := s.OperatorKeyDecisionCheck(ctx, row, append([]string{decidedBy}, deciderIDs...)); err != nil {
		return nil, err
	}
	secret, err := mintOperatorSecret()
	if err != nil {
		return nil, err
	}
	now := time.Now().Unix()
	if _, err := s.db.ExecContext(ctx, `
		UPDATE operator_keys
		   SET status = $1, secret = $2, fingerprint = $3, decided_by = $4, decided_at = $5
		 WHERE id = $6`,
		OperatorKeyActive, secret, OperatorKeyFingerprint(secret), decidedBy, now, id); err != nil {
		return nil, fmt.Errorf("approve operator key: %w", err)
	}
	return s.OperatorKey(ctx, id)
}

// RejectOperatorKey closes a request without minting anything. Same
// requester ≠ decider rule as approve: a self-reject is harmless today, but
// one rule for "decide" is one rule to test and one message to learn.
func (s *Store) RejectOperatorKey(ctx context.Context, id, decidedBy, note string, deciderIDs []string) (*OperatorKey, error) {
	row, err := s.OperatorKey(ctx, id)
	if err != nil {
		return nil, err
	}
	if row.Status != OperatorKeyRequested {
		return nil, fmt.Errorf("%w: %s", ErrOperatorKeyWrongState, row.Status)
	}
	if err := s.OperatorKeyDecisionCheck(ctx, row, append([]string{decidedBy}, deciderIDs...)); err != nil {
		return nil, err
	}
	if _, err := s.db.ExecContext(ctx, `
		UPDATE operator_keys SET status = $1, decided_by = $2, decided_at = $3, note = $4 WHERE id = $5`,
		OperatorKeyRejected, decidedBy, time.Now().Unix(), strings.TrimSpace(note), id); err != nil {
		return nil, fmt.Errorf("reject operator key: %w", err)
	}
	return s.OperatorKey(ctx, id)
}

// CollectOperatorKey returns the secret exactly once and clears it.
//
// One-time delivery is what keeps a live bearer token out of the database for
// longer than it takes to hand it over. A second attempt gets
// ErrOperatorKeyAlreadyDelivered rather than silence, so a person who lost the
// key learns they need a new one instead of concluding the page is broken.
func (s *Store) CollectOperatorKey(ctx context.Context, id, handle string) (string, error) {
	// One transaction, row locked from the read to the write: two concurrent
	// collections cannot both see the secret (single delivery). On SQLite the
	// single connection gave this.
	tx, err := s.db.BeginTxx(ctx, nil)
	if err != nil {
		return "", fmt.Errorf("collect operator key: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	row := &OperatorKey{}
	err = tx.GetContext(ctx, row, `SELECT id, created_at, handle, email, env, reason, status, fingerprint,
	             secret, decided_by, decided_at, delivered_at, revoked_by, revoked_at, note
	        FROM operator_keys WHERE id = $1 FOR UPDATE`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return "", ErrNotFound
	}
	if err != nil {
		return "", fmt.Errorf("collect operator key: %w", err)
	}
	// Collection is bound to the requester. Without this, any signed-in person
	// who could name an id would be handed somebody else's credential.
	if !strings.EqualFold(row.Handle, handle) {
		return "", ErrNotFound
	}
	if row.Status != OperatorKeyActive {
		return "", fmt.Errorf("%w: %s", ErrOperatorKeyWrongState, row.Status)
	}
	if row.Secret == "" || row.DeliveredAt != 0 {
		return "", ErrOperatorKeyAlreadyDelivered
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE operator_keys SET secret = '', delivered_at = $1 WHERE id = $2`,
		time.Now().Unix(), id); err != nil {
		return "", fmt.Errorf("mark operator key delivered: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return "", fmt.Errorf("mark operator key delivered: commit: %w", err)
	}
	return row.Secret, nil
}

// RevokeOperatorKey ends a key's life. The caller removes the key file; the
// portal stops accepting it within one reload interval.
func (s *Store) RevokeOperatorKey(ctx context.Context, id, revokedBy, note string) (*OperatorKey, error) {
	row, err := s.OperatorKey(ctx, id)
	if err != nil {
		return nil, err
	}
	if row.Status == OperatorKeyRevoked {
		return row, nil // idempotent: revoking twice is not an error worth raising
	}
	if _, err := s.db.ExecContext(ctx, `
		UPDATE operator_keys
		   SET status = $1, secret = '', revoked_by = $2, revoked_at = $3, note = $4
		 WHERE id = $5`,
		OperatorKeyRevoked, revokedBy, time.Now().Unix(), strings.TrimSpace(note), id); err != nil {
		return nil, fmt.Errorf("revoke operator key: %w", err)
	}
	return s.OperatorKey(ctx, id)
}

// OperatorKey fetches one row by id.
func (s *Store) OperatorKey(ctx context.Context, id string) (*OperatorKey, error) {
	rows, err := s.queryOperatorKeys(ctx, `WHERE id = $1`, id)
	if err != nil {
		return nil, err
	}
	if len(rows) == 0 {
		return nil, ErrNotFound
	}
	return rows[0], nil
}

// LiveOperatorKeyFor returns the requested-or-active row for a handle in an
// env, or nil. "Live" excludes rejected and revoked, which are history.
func (s *Store) LiveOperatorKeyFor(ctx context.Context, handle, env string) (*OperatorKey, error) {
	rows, err := s.queryOperatorKeys(ctx,
		`WHERE handle = $1 AND env = $2 AND status IN ($3, $4) ORDER BY created_at DESC`,
		handle, strings.TrimSpace(env), OperatorKeyRequested, OperatorKeyActive)
	if err != nil {
		return nil, err
	}
	if len(rows) == 0 {
		return nil, nil
	}
	return rows[0], nil
}

// ListOperatorKeys returns rows filtered by status ("" = all), newest first.
func (s *Store) ListOperatorKeys(ctx context.Context, status string) ([]*OperatorKey, error) {
	if strings.TrimSpace(status) == "" {
		return s.queryOperatorKeys(ctx, `ORDER BY created_at DESC`)
	}
	return s.queryOperatorKeys(ctx, `WHERE status = $1 ORDER BY created_at DESC`, status)
}

// ActiveOperatorKeys returns the rows whose keys the portal should be
// accepting. Used to reconcile the on-host key directory against the record —
// the directory is what the portal reads, so a row and a file disagreeing means
// somebody has access the record does not describe, or the reverse.
func (s *Store) ActiveOperatorKeys(ctx context.Context, env string) ([]*OperatorKey, error) {
	if strings.TrimSpace(env) == "" {
		return s.queryOperatorKeys(ctx, `WHERE status = $1 ORDER BY handle`, OperatorKeyActive)
	}
	return s.queryOperatorKeys(ctx, `WHERE status = $1 AND env = $2 ORDER BY handle`,
		OperatorKeyActive, strings.TrimSpace(env))
}

func (s *Store) queryOperatorKeys(ctx context.Context, where string, args ...any) ([]*OperatorKey, error) {
	q := `SELECT id, created_at, handle, email, env, reason, status, fingerprint,
	             secret, decided_by, decided_at, delivered_at, revoked_by, revoked_at, note,
	             COALESCE((SELECT r.requested_by FROM operator_key_requesters r WHERE r.id = operator_keys.id), '')
	        FROM operator_keys ` + where
	rows, err := s.db.QueryContext(ctx, q, args...)
	if err != nil {
		return nil, fmt.Errorf("query operator keys: %w", err)
	}
	defer rows.Close()
	var out []*OperatorKey
	for rows.Next() {
		k := &OperatorKey{}
		if err := rows.Scan(&k.ID, &k.CreatedAt, &k.Handle, &k.Email, &k.Env, &k.Reason,
			&k.Status, &k.Fingerprint, &k.Secret, &k.DecidedBy, &k.DecidedAt,
			&k.DeliveredAt, &k.RevokedBy, &k.RevokedAt, &k.Note, &k.RequestedBy); err != nil {
			return nil, fmt.Errorf("scan operator key: %w", err)
		}
		out = append(out, k)
	}
	return out, rows.Err()
}

// ActiveHandles satisfies opkeys.ActiveLister: the handles whose keys the
// portal should be accepting for a goatnet.
func (s *Store) ActiveHandles(ctx context.Context, env string) ([]string, error) {
	rows, err := s.ActiveOperatorKeys(ctx, env)
	if err != nil {
		return nil, err
	}
	out := make([]string, 0, len(rows))
	for _, r := range rows {
		out = append(out, r.Handle)
	}
	return out, nil
}

// RevokedHandles lists the handles of REVOKED rows the console actually issued
// a key for (a fingerprint is set only at approval), for the reconcile that
// writes missing revocation tombstones (GHSA-2w7v). A withdrawn, never-approved
// request is excluded: tombstoning it would disable an env-tier key nobody
// decided to revoke.
func (s *Store) RevokedHandles(ctx context.Context, env string) ([]string, error) {
	var rows []*OperatorKey
	var err error
	if strings.TrimSpace(env) == "" {
		rows, err = s.queryOperatorKeys(ctx, `WHERE status = $1 AND fingerprint <> '' ORDER BY handle`, OperatorKeyRevoked)
	} else {
		rows, err = s.queryOperatorKeys(ctx, `WHERE status = $1 AND fingerprint <> '' AND env = $2 ORDER BY handle`,
			OperatorKeyRevoked, strings.TrimSpace(env))
	}
	if err != nil {
		return nil, err
	}
	out := make([]string, 0, len(rows))
	for _, r := range rows {
		out = append(out, r.Handle)
	}
	return out, nil
}

// UncollectedOperatorKeys lists ACTIVE keys nobody has ever collected, oldest
// approval first.
//
// This state is invisible everywhere else. The key is live —
// the file is on disk and the portal accepts it — but the person it belongs to
// has never seen the secret, so from their side they have no credential and no
// signal that anything is waiting. Before the approval email existed the only
// way into this state was silence; now it is somebody who did not read the
// mail, which is rarer but not rare.
//
// `olderThan` filters to rows approved before that unix time, so a caller can
// ignore the minutes between an approval and the person collecting.
func (s *Store) UncollectedOperatorKeys(ctx context.Context, olderThan int64) ([]*OperatorKey, error) {
	var out []*OperatorKey
	err := s.db.SelectContext(ctx, &out,
		`SELECT id, created_at, handle, email, env, reason, status, fingerprint,
		        secret, decided_by, decided_at, delivered_at, revoked_by, revoked_at, note
		   FROM operator_keys
		   WHERE status = $1 AND delivered_at = 0 AND secret <> '' AND decided_at < $2
		   ORDER BY decided_at ASC`,
		OperatorKeyActive, olderThan)
	if err != nil {
		return nil, err
	}
	return out, nil
}

// KnownHandles is every handle this console has an operator-key row for, in any
// state. It is what tells a reconcile which grants are ITS to enforce: a file
// for a handle with no row was written by somebody else (a service's own
// ansible role, or an operator before this registry existed), and removing it
// would be the console asserting ownership of a credential it never created.
func (s *Store) KnownHandles(ctx context.Context, env string) ([]string, error) {
	var rows []*OperatorKey
	var err error
	if strings.TrimSpace(env) == "" {
		rows, err = s.queryOperatorKeys(ctx, `ORDER BY handle`)
	} else {
		rows, err = s.queryOperatorKeys(ctx, `WHERE env = $1 ORDER BY handle`, strings.TrimSpace(env))
	}
	if err != nil {
		return nil, err
	}
	seen := map[string]bool{}
	out := make([]string, 0, len(rows))
	for _, r := range rows {
		if !seen[r.Handle] {
			seen[r.Handle] = true
			out = append(out, r.Handle)
		}
	}
	return out, nil
}
