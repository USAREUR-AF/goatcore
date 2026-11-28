package registry

// Carriage rules (ADR 1251, docs/design/pod-carriage-and-provenance.md §2, §4,
// §7). This file stores what the rules are; it does not decide which sources a
// pod carries. Org membership comes from the data roster, which the API layer
// holds, so the API layer joins the two (internal/api/carriage.go) and renders
// the signed generation with internal/carriage.
//
// Lifecycle: a rule is proposed (granted by the source's custodian, not yet
// accepted by the pod's), active, or archived. Archived rules are kept: expiry
// and revoke archive, nothing deletes. Reactivation makes a NEW active rule
// that names the archived one in reactivated_from.

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/jmoiron/sqlx"
)

// carriageSQL is registry migration 3.
const carriageSQL = `
CREATE TABLE IF NOT EXISTS carriage_pods (
    pod_slot        TEXT COLLATE "C" PRIMARY KEY,  -- the pod's own slot (a machine)
    org             TEXT COLLATE "C" NOT NULL,     -- the org that runs the pod
    local_authority TEXT NOT NULL DEFAULT '[]',    -- JSON [{identity, cert_cn, slot}], named in advance (§4)
    generation      BIGINT NOT NULL DEFAULT 0,     -- last issued goat.carriage.generation/v1 number
    content_hash    TEXT COLLATE "C" NOT NULL DEFAULT '', -- carriage.ContentHash of that generation
    issued_at       BIGINT,                        -- when that generation was issued
    created_by      TEXT COLLATE "C" NOT NULL,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS carriage_rules (
    id               TEXT COLLATE "C" PRIMARY KEY, -- cr_<hex>
    kind             TEXT COLLATE "C" NOT NULL,    -- org | named | temporary
    source_slot      TEXT COLLATE "C",             -- exactly one of source_slot / source_org
    source_org       TEXT COLLATE "C",
    pod_slot         TEXT COLLATE "C",             -- exactly one of pod_slot / pod_org
    pod_org          TEXT COLLATE "C",
    until            BIGINT,                       -- Unix seconds; NULL only for an org rule
    reason           TEXT NOT NULL DEFAULT '',
    granted_by       TEXT COLLATE "C" NOT NULL,    -- the source's custodian who granted it
    accepted_by      TEXT COLLATE "C",             -- the pod's custodian who accepted it; NULL while proposed
    state            TEXT COLLATE "C" NOT NULL,    -- proposed | active | archived
    archived_at      BIGINT,
    archive_reason   TEXT COLLATE "C",             -- expired | revoked
    archived_by      TEXT COLLATE "C",
    reactivated_from TEXT COLLATE "C",             -- the archived rule this one brought back
    origin           TEXT COLLATE "C" NOT NULL DEFAULT 'central', -- central | ratified (from a pod's ledger)
    created_at       BIGINT NOT NULL,
    updated_at       BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS carriage_rules_state_idx  ON carriage_rules(state, until);
CREATE INDEX IF NOT EXISTS carriage_rules_source_idx ON carriage_rules(source_slot);
CREATE INDEX IF NOT EXISTS carriage_rules_pod_idx    ON carriage_rules(pod_slot);
CREATE UNIQUE INDEX IF NOT EXISTS carriage_rules_one_org_rule ON carriage_rules(source_org) WHERE kind = 'org' AND state <> 'archived';

CREATE TABLE IF NOT EXISTS carriage_opt_outs (
    source_slot  TEXT COLLATE "C" PRIMARY KEY,
    opted_out_by TEXT COLLATE "C" NOT NULL,
    opted_out_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS carriage_keyless (
    slot       TEXT COLLATE "C" PRIMARY KEY,       -- the keyless source's own slot; no leaf is minted for it
    adapters   TEXT COLLATE "C" NOT NULL,          -- comma-joined adapter slots that may publish for it
    interface  TEXT COLLATE "C" NOT NULL,          -- how it is received (§7.3)
    port       BIGINT NOT NULL,
    sender     TEXT COLLATE "C" NOT NULL,          -- the only sender the adapter accepts
    created_by TEXT COLLATE "C" NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS carriage_local_rules (
    id                 TEXT COLLATE "C" PRIMARY KEY, -- lr_<hex>, the portal's id
    pod_slot           TEXT COLLATE "C" NOT NULL,
    seq                BIGINT NOT NULL,              -- the ledger line number
    entry              TEXT NOT NULL,                -- the verified ledger entry, canonical JSON
    action             TEXT COLLATE "C" NOT NULL,    -- allow | extend | revoke | reactivate
    pod_rule_id        TEXT COLLATE "C" NOT NULL DEFAULT '', -- the rule id the pod used
    source_slot        TEXT COLLATE "C" NOT NULL,
    until              BIGINT NOT NULL,
    reason             TEXT NOT NULL,
    authority_identity TEXT COLLATE "C" NOT NULL,
    authority_cn       TEXT COLLATE "C" NOT NULL DEFAULT '',
    entry_ts           BIGINT NOT NULL,
    state              TEXT COLLATE "C" NOT NULL,    -- pending | ratified | lapsed
    state_note         TEXT NOT NULL DEFAULT '',
    received_at        BIGINT NOT NULL,
    decided_at         BIGINT,
    decided_by         TEXT COLLATE "C",
    ratified_rule_id   TEXT COLLATE "C",
    UNIQUE (pod_slot, seq)
);
CREATE INDEX IF NOT EXISTS carriage_local_rules_state_idx ON carriage_local_rules(state, pod_slot);
`

// carriageHealthSQL is registry migration 4: what the portal last saw of each
// pod (ADR 1251 phase 4, carriage health).
const carriageHealthSQL = `
ALTER TABLE carriage_pods ADD COLUMN last_pulled_at BIGINT;
ALTER TABLE carriage_pods ADD COLUMN last_pulled_generation BIGINT;
ALTER TABLE carriage_pods ADD COLUMN last_ledger_at BIGINT;
`

// Rule kinds (§2).
const (
	CarriageKindOrg       = "org"
	CarriageKindNamed     = "named"
	CarriageKindTemporary = "temporary"
	// CarriageKindExclusion takes one source off one pod's org rule. It is
	// recorded when an org lead ratifies a pod's local revoke, has no end
	// date, and is lifted by revoking it (the source's custodian).
	CarriageKindExclusion = "exclusion"
)

// ErrCarriageExclusion refuses extending or reactivating an exclusion.
var ErrCarriageExclusion = errors.New("an exclusion has no end date; revoke it to lift it")

// Rule states.
const (
	CarriageProposed = "proposed"
	CarriageActive   = "active"
	CarriageArchived = "archived"
)

// Archive reasons.
const (
	CarriageExpired = "expired"
	CarriageRevoked = "revoked"
)

// Local rule states (§4, §7.5).
const (
	LocalPending  = "pending"
	LocalRatified = "ratified"
	LocalLapsed   = "lapsed"
)

// Carriage refusals. The API maps each to its own code.
var (
	ErrCarriageNoUntil          = errors.New("a named or temporary rule needs an end date")
	ErrCarriageTooLong          = errors.New("the end date is beyond the longest rule length")
	ErrCarriagePast             = errors.New("the end date is in the past")
	ErrCarriageNotArchived      = errors.New("only an archived rule can be reactivated")
	ErrCarriageBeyondWindow     = errors.New("the rule was archived longer ago than the reactivation window")
	ErrCarriageNotActive        = errors.New("the rule is not active")
	ErrCarriageNotProposed      = errors.New("the rule is not waiting for acceptance")
	ErrCarriageOrgRuleNoEnd     = errors.New("an org rule has no end date to extend")
	ErrCarriagePodExists        = errors.New("the pod is already registered for another org")
	ErrCarriageLedgerConflict   = errors.New("a different ledger entry is already recorded at this sequence number")
	ErrCarriageLocalNotPending  = errors.New("the local rule is not pending")
	ErrCarriageReactivatedTwice = errors.New("this archived rule has already been reactivated")
)

// CarriagePod is one registered pod.
type CarriagePod struct {
	PodSlot        string `db:"pod_slot"        json:"pod_slot"`
	Org            string `db:"org"             json:"org"`
	LocalAuthority string `db:"local_authority" json:"-"`
	Generation     int64  `db:"generation"      json:"generation"`
	ContentHash    string `db:"content_hash"    json:"-"`
	IssuedAt       *int64 `db:"issued_at"       json:"issued_at,omitempty"`
	CreatedBy      string `db:"created_by"      json:"created_by"`
	CreatedAt      int64  `db:"created_at"      json:"created_at"`
	UpdatedAt      int64  `db:"updated_at"      json:"updated_at"`
	// What the portal last saw of the pod (migration 4): the generation it
	// last served the pod and when, and when the pod last posted its ledger.
	LastPulledAt         *int64 `db:"last_pulled_at"         json:"last_pulled_at,omitempty"`
	LastPulledGeneration *int64 `db:"last_pulled_generation" json:"last_pulled_generation,omitempty"`
	LastLedgerAt         *int64 `db:"last_ledger_at"         json:"last_ledger_at,omitempty"`
}

// PodAuthority is one local-authority entry as stored: an identity and either
// the certificate CN it signs in with, or the slot whose current leaf CN that is.
type PodAuthority struct {
	Identity string `json:"identity"`
	CertCN   string `json:"cert_cn,omitempty"`
	Slot     string `json:"slot,omitempty"`
}

// Authorities decodes LocalAuthority.
func (p *CarriagePod) Authorities() []PodAuthority {
	var out []PodAuthority
	if err := json.Unmarshal([]byte(p.LocalAuthority), &out); err != nil {
		return nil
	}
	return out
}

// CarriageRule is one rule row.
type CarriageRule struct {
	ID              string  `db:"id"               json:"id"`
	Kind            string  `db:"kind"             json:"kind"`
	SourceSlot      *string `db:"source_slot"      json:"source_slot,omitempty"`
	SourceOrg       *string `db:"source_org"       json:"source_org,omitempty"`
	PodSlot         *string `db:"pod_slot"         json:"pod_slot,omitempty"`
	PodOrg          *string `db:"pod_org"          json:"pod_org,omitempty"`
	Until           *int64  `db:"until"            json:"until,omitempty"`
	Reason          string  `db:"reason"           json:"reason"`
	GrantedBy       string  `db:"granted_by"       json:"granted_by"`
	AcceptedBy      *string `db:"accepted_by"      json:"accepted_by,omitempty"`
	State           string  `db:"state"            json:"state"`
	ArchivedAt      *int64  `db:"archived_at"      json:"archived_at,omitempty"`
	ArchiveReason   *string `db:"archive_reason"   json:"archive_reason,omitempty"`
	ArchivedBy      *string `db:"archived_by"      json:"archived_by,omitempty"`
	ReactivatedFrom *string `db:"reactivated_from" json:"reactivated_from,omitempty"`
	Origin          string  `db:"origin"           json:"origin"`
	CreatedAt       int64   `db:"created_at"       json:"created_at"`
	UpdatedAt       int64   `db:"updated_at"       json:"updated_at"`
}

// KeylessSource is a source enrolled without a key (§7.3).
type KeylessSource struct {
	Slot      string `db:"slot"       json:"slot"`
	Adapters  string `db:"adapters"   json:"-"`
	Interface string `db:"interface"  json:"interface"`
	Port      int64  `db:"port"       json:"port"`
	Sender    string `db:"sender"     json:"sender"`
	CreatedBy string `db:"created_by" json:"created_by"`
	CreatedAt int64  `db:"created_at" json:"created_at"`
}

// AdapterSlots splits Adapters.
func (k KeylessSource) AdapterSlots() []string {
	var out []string
	for _, a := range strings.Split(k.Adapters, ",") {
		if a = strings.TrimSpace(a); a != "" {
			out = append(out, a)
		}
	}
	return out
}

// LocalRule is one ledger entry reported by a pod.
type LocalRule struct {
	ID                string  `db:"id"                 json:"id"`
	PodSlot           string  `db:"pod_slot"           json:"pod_slot"`
	Seq               int64   `db:"seq"                json:"seq"`
	Entry             string  `db:"entry"              json:"-"`
	Action            string  `db:"action"             json:"action"`
	PodRuleID         string  `db:"pod_rule_id"        json:"pod_rule_id,omitempty"`
	SourceSlot        string  `db:"source_slot"        json:"source_slot"`
	Until             int64   `db:"until"              json:"until"`
	Reason            string  `db:"reason"             json:"reason"`
	AuthorityIdentity string  `db:"authority_identity" json:"authority_identity"`
	AuthorityCN       string  `db:"authority_cn"       json:"authority_cn,omitempty"`
	EntryTS           int64   `db:"entry_ts"           json:"entry_ts"`
	State             string  `db:"state"              json:"state"`
	StateNote         string  `db:"state_note"         json:"state_note,omitempty"`
	ReceivedAt        int64   `db:"received_at"        json:"received_at"`
	DecidedAt         *int64  `db:"decided_at"         json:"decided_at,omitempty"`
	DecidedBy         *string `db:"decided_by"         json:"decided_by,omitempty"`
	RatifiedRuleID    *string `db:"ratified_rule_id"   json:"ratified_rule_id,omitempty"`
}

// CarriageClock is the time rules expire against. A variable so tests can
// move it.
var CarriageClock = time.Now

func newCarriageID(prefix string) (string, error) {
	var b [8]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return prefix + hex.EncodeToString(b[:]), nil
}

func strPtr(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}

// CarriageLimits are the goatnet settings rules are checked against.
type CarriageLimits struct {
	MaxRule            time.Duration // a named or temporary rule runs at most this long
	ReactivationWindow time.Duration // an archived rule can be reactivated this long after archiving
}

// checkUntil validates an end date against now and the limits.
func (l CarriageLimits) checkUntil(until, now int64) error {
	if until <= now {
		return ErrCarriagePast
	}
	if l.MaxRule > 0 && until > now+int64(l.MaxRule/time.Second) {
		return fmt.Errorf("%w (%d days)", ErrCarriageTooLong, int64(l.MaxRule/(24*time.Hour)))
	}
	return nil
}

// RegisterCarriagePod records a pod for an org and, the first time a pod of
// that org is registered, the org's default rule (pods run by the org may
// carry the org's sources). Registering the same pod for the same org again
// updates its local authority. It returns the pod and the org rule (nil when
// the org's rule was revoked and is archived: registration does not undo a
// revoke).
func (r *Registry) RegisterCarriagePod(ctx context.Context, podSlot, org string, authority []PodAuthority, actor string) (*CarriagePod, *CarriageRule, error) {
	podSlot, org = strings.TrimSpace(podSlot), strings.TrimSpace(org)
	if podSlot == "" || org == "" {
		return nil, nil, invalid(errors.New("pod_slot and org are required"))
	}
	if authority == nil {
		authority = []PodAuthority{}
	}
	la, err := json.Marshal(authority)
	if err != nil {
		return nil, nil, err
	}
	now := CarriageClock().Unix()
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return nil, nil, fmt.Errorf("register pod: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()

	var existing string
	err = tx.GetContext(ctx, &existing, `SELECT org FROM carriage_pods WHERE pod_slot = $1`, podSlot)
	switch {
	case errors.Is(err, sqlErrNoRows()):
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO carriage_pods (pod_slot, org, local_authority, created_by, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5)`,
			podSlot, org, string(la), actor, now); err != nil {
			return nil, nil, fmt.Errorf("register pod: %w", err)
		}
	case err != nil:
		return nil, nil, fmt.Errorf("register pod: %w", err)
	case existing != org:
		return nil, nil, fmt.Errorf("%w (%s)", ErrCarriagePodExists, existing)
	default:
		if _, err := tx.ExecContext(ctx,
			`UPDATE carriage_pods SET local_authority = $1, updated_at = $2 WHERE pod_slot = $3`, string(la), now, podSlot); err != nil {
			return nil, nil, fmt.Errorf("register pod: %w", err)
		}
	}

	var rule *CarriageRule
	var rules []CarriageRule
	if err := tx.SelectContext(ctx, &rules,
		`SELECT * FROM carriage_rules WHERE kind = 'org' AND source_org = $1 ORDER BY created_at DESC, id`, org); err != nil {
		return nil, nil, fmt.Errorf("register pod: org rule: %w", err)
	}
	for i := range rules {
		if rules[i].State != CarriageArchived {
			rule = &rules[i]
			break
		}
	}
	if rule == nil && len(rules) == 0 {
		id, err := newCarriageID("cr_")
		if err != nil {
			return nil, nil, err
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO carriage_rules (id, kind, source_org, pod_org, reason, granted_by, accepted_by, state, created_at, updated_at)
			 VALUES ($1, 'org', $2, $2, $3, $2, $2, 'active', $4, $4)`,
			id, org, "org rule: pods run by "+org+" carry "+org+" sources", now); err != nil {
			return nil, nil, fmt.Errorf("register pod: create org rule: %w", err)
		}
		rule = &CarriageRule{}
		if err := tx.GetContext(ctx, rule, `SELECT * FROM carriage_rules WHERE id = $1`, id); err != nil {
			return nil, nil, err
		}
	}
	if err := tx.Commit(); err != nil {
		return nil, nil, fmt.Errorf("register pod: commit: %w", err)
	}
	pod, err := r.CarriagePod(ctx, podSlot)
	return pod, rule, err
}

// CarriagePod returns one pod, or ErrNotFound.
func (r *Registry) CarriagePod(ctx context.Context, podSlot string) (*CarriagePod, error) {
	var p CarriagePod
	err := r.reader().GetContext(ctx, &p, `SELECT * FROM carriage_pods WHERE pod_slot = $1`, podSlot)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("carriage pod: %w", err)
	}
	return &p, nil
}

// CarriagePods lists every registered pod.
func (r *Registry) CarriagePods(ctx context.Context) ([]CarriagePod, error) {
	out := []CarriagePod{}
	if err := r.reader().SelectContext(ctx, &out, `SELECT * FROM carriage_pods ORDER BY org, pod_slot`); err != nil {
		return nil, fmt.Errorf("carriage pods: %w", err)
	}
	return out, nil
}

// NewCarriageRule is a request for a named or temporary rule.
type NewCarriageRule struct {
	Kind       string
	SourceSlot string
	SourceOrg  string
	PodSlot    string
	PodOrg     string
	Until      int64
	Reason     string
	GrantedBy  string
	AcceptedBy string // "" leaves the rule proposed until the pod's custodian accepts
	Origin     string // "" = central
}

// CreateCarriageRule stores a named or temporary rule. Org rules are created
// only by RegisterCarriagePod.
func (r *Registry) CreateCarriageRule(ctx context.Context, n NewCarriageRule, lim CarriageLimits) (*CarriageRule, error) {
	if n.Kind != CarriageKindNamed && n.Kind != CarriageKindTemporary {
		return nil, invalid(fmt.Errorf("kind %q: only named or temporary rules are created here (the org rule is created with the pod)", n.Kind))
	}
	if (n.SourceSlot == "") == (n.SourceOrg == "") {
		return nil, invalid(errors.New("name exactly one of source_slot or source_org"))
	}
	if (n.PodSlot == "") == (n.PodOrg == "") {
		return nil, invalid(errors.New("name exactly one of pod_slot or pod_org"))
	}
	if strings.TrimSpace(n.Reason) == "" {
		return nil, invalid(errors.New("a reason is required"))
	}
	if n.Until == 0 {
		return nil, ErrCarriageNoUntil
	}
	now := CarriageClock().Unix()
	if err := lim.checkUntil(n.Until, now); err != nil {
		return nil, err
	}
	id, err := newCarriageID("cr_")
	if err != nil {
		return nil, err
	}
	state := CarriageProposed
	if n.AcceptedBy != "" {
		state = CarriageActive
	}
	origin := n.Origin
	if origin == "" {
		origin = "central"
	}
	if _, err := r.db.ExecContext(ctx,
		`INSERT INTO carriage_rules (id, kind, source_slot, source_org, pod_slot, pod_org, until, reason, granted_by, accepted_by, state, origin, created_at, updated_at)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)`,
		id, n.Kind, strPtr(n.SourceSlot), strPtr(n.SourceOrg), strPtr(n.PodSlot), strPtr(n.PodOrg), n.Until,
		strings.TrimSpace(n.Reason), n.GrantedBy, strPtr(n.AcceptedBy), state, origin, now); err != nil {
		return nil, fmt.Errorf("create carriage rule: %w", err)
	}
	return r.CarriageRule(ctx, id)
}

// CarriageRule returns one rule, or ErrNotFound.
func (r *Registry) CarriageRule(ctx context.Context, id string) (*CarriageRule, error) {
	var c CarriageRule
	err := r.reader().GetContext(ctx, &c, `SELECT * FROM carriage_rules WHERE id = $1`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("carriage rule: %w", err)
	}
	return &c, nil
}

// CarriageRuleFilter narrows ListCarriageRules. Empty fields match everything.
type CarriageRuleFilter struct {
	State      string
	PodSlot    string // rules naming this pod by slot
	PodOrg     string // or naming this org's pods
	SourceSlot string
}

// ListCarriageRules returns rules, newest first. It archives expired rules
// first, so an expired rule never reads as active.
func (r *Registry) ListCarriageRules(ctx context.Context, f CarriageRuleFilter) ([]CarriageRule, error) {
	if _, err := r.ArchiveExpiredCarriageRules(ctx); err != nil {
		return nil, err
	}
	q := `SELECT * FROM carriage_rules`
	var where []string
	var args []any
	if f.State != "" {
		where = append(where, "state = ?")
		args = append(args, f.State)
	}
	switch {
	case f.PodSlot != "" && f.PodOrg != "":
		where = append(where, "(pod_slot = ? OR pod_org = ?)")
		args = append(args, f.PodSlot, f.PodOrg)
	case f.PodSlot != "":
		where = append(where, "pod_slot = ?")
		args = append(args, f.PodSlot)
	case f.PodOrg != "":
		where = append(where, "pod_org = ?")
		args = append(args, f.PodOrg)
	}
	if f.SourceSlot != "" {
		where = append(where, "source_slot = ?")
		args = append(args, f.SourceSlot)
	}
	if len(where) > 0 {
		q += " WHERE " + strings.Join(where, " AND ")
	}
	q += " ORDER BY created_at DESC, id"
	out := []CarriageRule{}
	if err := r.reader().SelectContext(ctx, &out, sqlx.Rebind(sqlx.DOLLAR, q), args...); err != nil {
		return nil, fmt.Errorf("list carriage rules: %w", err)
	}
	return out, nil
}

// ArchiveExpiredCarriageRules archives every active or proposed rule whose end
// date has passed. Never deletes. Returns how many it archived.
func (r *Registry) ArchiveExpiredCarriageRules(ctx context.Context) (int64, error) {
	now := CarriageClock().Unix()
	res, err := r.db.ExecContext(ctx,
		`UPDATE carriage_rules SET state = 'archived', archived_at = until, archive_reason = 'expired', archived_by = 'expiry', updated_at = $1
		  WHERE state <> 'archived' AND until IS NOT NULL AND until <= $1`, now)
	if err != nil {
		return 0, fmt.Errorf("archive expired carriage rules: %w", err)
	}
	n, _ := res.RowsAffected()
	return n, nil
}

// AcceptCarriageRule records the pod custodian's acceptance of a proposed rule.
func (r *Registry) AcceptCarriageRule(ctx context.Context, id, by string) (*CarriageRule, error) {
	if _, err := r.ArchiveExpiredCarriageRules(ctx); err != nil {
		return nil, err
	}
	res, err := r.db.ExecContext(ctx,
		`UPDATE carriage_rules SET state = 'active', accepted_by = $1, updated_at = $2 WHERE id = $3 AND state = 'proposed'`,
		by, CarriageClock().Unix(), id)
	if err != nil {
		return nil, fmt.Errorf("accept carriage rule: %w", err)
	}
	if n, _ := res.RowsAffected(); n == 0 {
		if _, err := r.CarriageRule(ctx, id); err != nil {
			return nil, err
		}
		return nil, ErrCarriageNotProposed
	}
	return r.CarriageRule(ctx, id)
}

// ExtendCarriageRule moves an active or proposed rule's end date.
func (r *Registry) ExtendCarriageRule(ctx context.Context, id string, until int64, lim CarriageLimits) (*CarriageRule, error) {
	if _, err := r.ArchiveExpiredCarriageRules(ctx); err != nil {
		return nil, err
	}
	c, err := r.CarriageRule(ctx, id)
	if err != nil {
		return nil, err
	}
	if c.Kind == CarriageKindOrg {
		return nil, ErrCarriageOrgRuleNoEnd
	}
	if c.Kind == CarriageKindExclusion {
		return nil, ErrCarriageExclusion
	}
	if c.State == CarriageArchived {
		return nil, ErrCarriageNotActive
	}
	now := CarriageClock().Unix()
	if err := lim.checkUntil(until, now); err != nil {
		return nil, err
	}
	if _, err := r.db.ExecContext(ctx,
		`UPDATE carriage_rules SET until = $1, updated_at = $2 WHERE id = $3 AND state <> 'archived'`, until, now, id); err != nil {
		return nil, fmt.Errorf("extend carriage rule: %w", err)
	}
	return r.CarriageRule(ctx, id)
}

// RevokeCarriageRule archives a rule at once, as revoked.
func (r *Registry) RevokeCarriageRule(ctx context.Context, id, by string) (*CarriageRule, error) {
	if _, err := r.ArchiveExpiredCarriageRules(ctx); err != nil {
		return nil, err
	}
	now := CarriageClock().Unix()
	res, err := r.db.ExecContext(ctx,
		`UPDATE carriage_rules SET state = 'archived', archived_at = $1, archive_reason = 'revoked', archived_by = $2, updated_at = $1
		  WHERE id = $3 AND state <> 'archived'`, now, by, id)
	if err != nil {
		return nil, fmt.Errorf("revoke carriage rule: %w", err)
	}
	if n, _ := res.RowsAffected(); n == 0 {
		if _, err := r.CarriageRule(ctx, id); err != nil {
			return nil, err
		}
		return nil, ErrCarriageNotActive
	}
	return r.CarriageRule(ctx, id)
}

// ReactivateCarriageRule brings back an archived rule as a new active rule,
// inside the reactivation window. A named or temporary rule runs again for as
// long as it ran before, capped at the longest rule length, unless until is
// given. Outside the window the error says so: the rule is then approved like
// a new one.
func (r *Registry) ReactivateCarriageRule(ctx context.Context, id string, until int64, by string, lim CarriageLimits) (*CarriageRule, error) {
	if _, err := r.ArchiveExpiredCarriageRules(ctx); err != nil {
		return nil, err
	}
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return nil, fmt.Errorf("reactivate: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var c CarriageRule
	err = tx.GetContext(ctx, &c, `SELECT * FROM carriage_rules WHERE id = $1`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("reactivate: %w", err)
	}
	if c.Kind == CarriageKindExclusion {
		return nil, ErrCarriageExclusion
	}
	if c.State != CarriageArchived || c.ArchivedAt == nil {
		return nil, ErrCarriageNotArchived
	}
	var already int
	if err := tx.GetContext(ctx, &already, `SELECT COUNT(*) FROM carriage_rules WHERE reactivated_from = $1`, id); err != nil {
		return nil, err
	}
	if already > 0 {
		return nil, ErrCarriageReactivatedTwice
	}
	now := CarriageClock().Unix()
	if lim.ReactivationWindow > 0 && now > *c.ArchivedAt+int64(lim.ReactivationWindow/time.Second) {
		return nil, fmt.Errorf("%w: archived %s, window %d days; request it again as a new rule",
			ErrCarriageBeyondWindow, time.Unix(*c.ArchivedAt, 0).UTC().Format(time.RFC3339), int64(lim.ReactivationWindow/(24*time.Hour)))
	}
	var newUntil *int64
	if c.Kind != CarriageKindOrg {
		u := until
		if u == 0 {
			ran := int64(0)
			if c.Until != nil {
				ran = *c.Until - c.CreatedAt
			}
			if ran <= 0 || (lim.MaxRule > 0 && ran > int64(lim.MaxRule/time.Second)) {
				ran = int64(lim.MaxRule / time.Second)
			}
			u = now + ran
		}
		if err := lim.checkUntil(u, now); err != nil {
			return nil, err
		}
		newUntil = &u
	} else {
		var live int
		if err := tx.GetContext(ctx, &live,
			`SELECT COUNT(*) FROM carriage_rules WHERE kind = 'org' AND source_org = $1 AND state <> 'archived'`, c.SourceOrg); err != nil {
			return nil, err
		}
		if live > 0 {
			return nil, ErrCarriageReactivatedTwice
		}
	}
	nid, err := newCarriageID("cr_")
	if err != nil {
		return nil, err
	}
	if _, err := tx.ExecContext(ctx,
		`INSERT INTO carriage_rules (id, kind, source_slot, source_org, pod_slot, pod_org, until, reason, granted_by, accepted_by, state, reactivated_from, origin, created_at, updated_at)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'active', $11, $12, $13, $13)`,
		nid, c.Kind, c.SourceSlot, c.SourceOrg, c.PodSlot, c.PodOrg, newUntil, c.Reason, c.GrantedBy, c.AcceptedBy, id, c.Origin, now); err != nil {
		return nil, fmt.Errorf("reactivate: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("reactivate: commit: %w", err)
	}
	_ = by // the actor is recorded in the audit log by the caller
	return r.CarriageRule(ctx, nid)
}

// SetCarriageOptOut records or removes a source's opt-out from the org rule.
func (r *Registry) SetCarriageOptOut(ctx context.Context, sourceSlot string, optOut bool, by string) error {
	if optOut {
		_, err := r.db.ExecContext(ctx,
			`INSERT INTO carriage_opt_outs (source_slot, opted_out_by, opted_out_at) VALUES ($1, $2, $3)
			 ON CONFLICT (source_slot) DO NOTHING`, sourceSlot, by, CarriageClock().Unix())
		return err
	}
	_, err := r.db.ExecContext(ctx, `DELETE FROM carriage_opt_outs WHERE source_slot = $1`, sourceSlot)
	return err
}

// CarriageOptOuts returns the set of sources that opted out of the org rule.
func (r *Registry) CarriageOptOuts(ctx context.Context) (map[string]bool, error) {
	var slots []string
	if err := r.reader().SelectContext(ctx, &slots, `SELECT source_slot FROM carriage_opt_outs`); err != nil {
		return nil, fmt.Errorf("carriage opt-outs: %w", err)
	}
	out := make(map[string]bool, len(slots))
	for _, s := range slots {
		out[s] = true
	}
	return out, nil
}

// RecordKeylessSource stores how a keyless source is received and which
// adapters may publish for it.
func (r *Registry) RecordKeylessSource(ctx context.Context, k KeylessSource) error {
	if k.CreatedAt == 0 {
		k.CreatedAt = CarriageClock().Unix()
	}
	_, err := r.db.ExecContext(ctx,
		`INSERT INTO carriage_keyless (slot, adapters, interface, port, sender, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)`,
		k.Slot, k.Adapters, k.Interface, k.Port, k.Sender, k.CreatedBy, k.CreatedAt)
	if err != nil {
		return fmt.Errorf("record keyless source: %w", err)
	}
	return nil
}

// KeylessSources returns every keyless source, keyed by slot.
func (r *Registry) KeylessSources(ctx context.Context) (map[string]KeylessSource, error) {
	var rows []KeylessSource
	if err := r.reader().SelectContext(ctx, &rows, `SELECT * FROM carriage_keyless`); err != nil {
		return nil, fmt.Errorf("keyless sources: %w", err)
	}
	out := make(map[string]KeylessSource, len(rows))
	for _, k := range rows {
		out[k.Slot] = k
	}
	return out, nil
}

// CarriageSlot is what rendering needs to know about one live slot.
type CarriageSlot struct {
	SlotID     string  `db:"slot_id"`
	Generation int64   `db:"generation"`
	EnrolledBy *string `db:"enrolled_by"`
	HasLeaf    bool    `db:"has_leaf"` // a leaf was issued at the current generation
}

// LiveCarriageSlots returns every assigned, unrevoked slot.
func (r *Registry) LiveCarriageSlots(ctx context.Context) ([]CarriageSlot, error) {
	var out []CarriageSlot
	if err := r.reader().SelectContext(ctx, &out,
		`SELECT s.slot_id, s.generation, s.enrolled_by,
		        EXISTS (SELECT 1 FROM slot_leaves l WHERE l.slot_id = s.slot_id AND l.generation = s.generation AND l.superseded_at IS NULL) AS has_leaf
		   FROM slots s
		  WHERE s.assigned_to IS NOT NULL AND s.revoked_at IS NULL
		  ORDER BY s.slot_id`); err != nil {
		return nil, fmt.Errorf("live carriage slots: %w", err)
	}
	return out, nil
}

// IssueCarriageGeneration records the generation number for a pod's rendered
// content. When contentHash equals the stored hash nothing changes and the
// stored number and issue time come back; otherwise the number goes up by one
// and the issue time is now. Monotonic per pod.
func (r *Registry) IssueCarriageGeneration(ctx context.Context, podSlot, contentHash string) (gen int64, issuedAt int64, err error) {
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return 0, 0, fmt.Errorf("issue generation: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var p CarriagePod
	err = tx.GetContext(ctx, &p, `SELECT * FROM carriage_pods WHERE pod_slot = $1`, podSlot)
	if errors.Is(err, sqlErrNoRows()) {
		return 0, 0, ErrNotFound
	}
	if err != nil {
		return 0, 0, fmt.Errorf("issue generation: %w", err)
	}
	if p.ContentHash == contentHash && p.Generation > 0 && p.IssuedAt != nil {
		return p.Generation, *p.IssuedAt, nil
	}
	now := CarriageClock().Unix()
	if p.IssuedAt != nil && now <= *p.IssuedAt {
		now = *p.IssuedAt + 1 // issue times move forward with the number
	}
	gen = p.Generation + 1
	if _, err := tx.ExecContext(ctx,
		`UPDATE carriage_pods SET generation = $1, content_hash = $2, issued_at = $3, updated_at = $3 WHERE pod_slot = $4`,
		gen, contentHash, now, podSlot); err != nil {
		return 0, 0, fmt.Errorf("issue generation: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return 0, 0, fmt.Errorf("issue generation: commit: %w", err)
	}
	return gen, now, nil
}

// RecordCarriagePull notes that pod was served generation gen now.
func (r *Registry) RecordCarriagePull(ctx context.Context, podSlot string, gen int64) error {
	_, err := r.db.ExecContext(ctx,
		`UPDATE carriage_pods SET last_pulled_at = $1, last_pulled_generation = $2 WHERE pod_slot = $3`,
		CarriageClock().Unix(), gen, podSlot)
	if err != nil {
		return fmt.Errorf("record carriage pull: %w", err)
	}
	return nil
}

// RecordCarriageLedger notes that pod posted its ledger now.
func (r *Registry) RecordCarriageLedger(ctx context.Context, podSlot string) error {
	_, err := r.db.ExecContext(ctx, `UPDATE carriage_pods SET last_ledger_at = $1 WHERE pod_slot = $2`, CarriageClock().Unix(), podSlot)
	if err != nil {
		return fmt.Errorf("record carriage ledger: %w", err)
	}
	return nil
}

// PendingLocalRuleCounts is the number of pending local rules per pod.
func (r *Registry) PendingLocalRuleCounts(ctx context.Context) (map[string]int64, error) {
	var rows []struct {
		Pod string `db:"pod_slot"`
		N   int64  `db:"n"`
	}
	if err := r.reader().SelectContext(ctx, &rows,
		`SELECT pod_slot, COUNT(*) AS n FROM carriage_local_rules WHERE state = 'pending' GROUP BY pod_slot`); err != nil {
		return nil, fmt.Errorf("pending local rules: %w", err)
	}
	out := make(map[string]int64, len(rows))
	for _, x := range rows {
		out[x.Pod] = x.N
	}
	return out, nil
}

// NewLocalRule is one verified ledger entry to record.
type NewLocalRule struct {
	PodSlot           string
	Seq               int64
	Entry             string // canonical JSON of the verified entry
	Action            string
	PodRuleID         string
	SourceSlot        string
	Until             int64
	Reason            string
	AuthorityIdentity string
	AuthorityCN       string
	EntryTS           int64
	State             string // pending or lapsed
	StateNote         string
}

// RecordLocalRule stores a ledger entry. The same entry at the same seq is a
// duplicate (returned with dup=true, nothing written); a different entry at a
// seq already on file is ErrCarriageLedgerConflict.
func (r *Registry) RecordLocalRule(ctx context.Context, n NewLocalRule) (lr *LocalRule, dup bool, err error) {
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return nil, false, fmt.Errorf("record local rule: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var have LocalRule
	err = tx.GetContext(ctx, &have, `SELECT * FROM carriage_local_rules WHERE pod_slot = $1 AND seq = $2`, n.PodSlot, n.Seq)
	if err == nil {
		if have.Entry == n.Entry {
			return &have, true, nil
		}
		return nil, false, fmt.Errorf("%w (seq %d)", ErrCarriageLedgerConflict, n.Seq)
	}
	if !errors.Is(err, sqlErrNoRows()) {
		return nil, false, fmt.Errorf("record local rule: %w", err)
	}
	id, err := newCarriageID("lr_")
	if err != nil {
		return nil, false, err
	}
	now := CarriageClock().Unix()
	var decidedAt any
	var decidedBy any
	if n.State == LocalLapsed {
		decidedAt, decidedBy = now, "portal"
	}
	if _, err := tx.ExecContext(ctx,
		`INSERT INTO carriage_local_rules (id, pod_slot, seq, entry, action, pod_rule_id, source_slot, until, reason, authority_identity, authority_cn, entry_ts, state, state_note, received_at, decided_at, decided_by)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)`,
		id, n.PodSlot, n.Seq, n.Entry, n.Action, n.PodRuleID, n.SourceSlot, n.Until, n.Reason, n.AuthorityIdentity, n.AuthorityCN,
		n.EntryTS, n.State, n.StateNote, now, decidedAt, decidedBy); err != nil {
		return nil, false, fmt.Errorf("record local rule: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, false, fmt.Errorf("record local rule: commit: %w", err)
	}
	got, err := r.LocalRule(ctx, id)
	return got, false, err
}

// LocalRule returns one local rule, or ErrNotFound.
func (r *Registry) LocalRule(ctx context.Context, id string) (*LocalRule, error) {
	var l LocalRule
	err := r.reader().GetContext(ctx, &l, `SELECT * FROM carriage_local_rules WHERE id = $1`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("local rule: %w", err)
	}
	return &l, nil
}

// ListLocalRules returns local rules in a state ("" = all), optionally for
// one pod, oldest first.
func (r *Registry) ListLocalRules(ctx context.Context, state, podSlot string) ([]LocalRule, error) {
	q := `SELECT * FROM carriage_local_rules`
	var where []string
	var args []any
	if state != "" {
		where = append(where, "state = ?")
		args = append(args, state)
	}
	if podSlot != "" {
		where = append(where, "pod_slot = ?")
		args = append(args, podSlot)
	}
	if len(where) > 0 {
		q += " WHERE " + strings.Join(where, " AND ")
	}
	q += " ORDER BY pod_slot, seq"
	out := []LocalRule{}
	if err := r.reader().SelectContext(ctx, &out, sqlx.Rebind(sqlx.DOLLAR, q), args...); err != nil {
		return nil, fmt.Errorf("list local rules: %w", err)
	}
	return out, nil
}

// LastLocalSeq is the highest ledger seq stored for a pod (0 = none): the
// pod's acknowledgment of what the portal holds.
func (r *Registry) LastLocalSeq(ctx context.Context, podSlot string) (int64, error) {
	var n int64
	if err := r.reader().GetContext(ctx, &n,
		`SELECT COALESCE(MAX(seq), 0) FROM carriage_local_rules WHERE pod_slot = $1`, podSlot); err != nil {
		return 0, fmt.Errorf("last local seq: %w", err)
	}
	return n, nil
}

// CreateCarriageExclusion records that the org rule no longer covers source
// on pod. Idempotent: an active exclusion for the pair is returned as is.
func (r *Registry) CreateCarriageExclusion(ctx context.Context, sourceSlot, podSlot, reason, by string) (*CarriageRule, error) {
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return nil, fmt.Errorf("exclusion: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var id string
	err = tx.GetContext(ctx, &id, `SELECT id FROM carriage_rules WHERE kind = 'exclusion' AND state = 'active' AND source_slot = $1 AND pod_slot = $2`, sourceSlot, podSlot)
	if errors.Is(err, sqlErrNoRows()) {
		if id, err = newCarriageID("cr_"); err != nil {
			return nil, err
		}
		now := CarriageClock().Unix()
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO carriage_rules (id, kind, source_slot, pod_slot, reason, granted_by, accepted_by, state, origin, created_at, updated_at)
			 VALUES ($1, 'exclusion', $2, $3, $4, $5, $5, 'active', 'ratified', $6, $6)`,
			id, sourceSlot, podSlot, reason, by, now); err != nil {
			return nil, fmt.Errorf("exclusion: %w", err)
		}
	} else if err != nil {
		return nil, fmt.Errorf("exclusion: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("exclusion: commit: %w", err)
	}
	return r.CarriageRule(ctx, id)
}

// DecideLocalRule moves a pending local rule to ratified or lapsed.
func (r *Registry) DecideLocalRule(ctx context.Context, id, state, note, by, ratifiedRuleID string) (*LocalRule, error) {
	if state != LocalRatified && state != LocalLapsed {
		return nil, invalid(fmt.Errorf("state %q is not ratified or lapsed", state))
	}
	res, err := r.db.ExecContext(ctx,
		`UPDATE carriage_local_rules SET state = $1, state_note = $2, decided_at = $3, decided_by = $4, ratified_rule_id = $5
		  WHERE id = $6 AND state = 'pending'`,
		state, note, CarriageClock().Unix(), by, strPtr(ratifiedRuleID), id)
	if err != nil {
		return nil, fmt.Errorf("decide local rule: %w", err)
	}
	if n, _ := res.RowsAffected(); n == 0 {
		if _, err := r.LocalRule(ctx, id); err != nil {
			return nil, err
		}
		return nil, ErrCarriageLocalNotPending
	}
	return r.LocalRule(ctx, id)
}
