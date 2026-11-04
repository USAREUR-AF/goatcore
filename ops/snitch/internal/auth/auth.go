// Package auth provides API key authentication and RBAC for SNITCH.
//
// Roles (privilege hierarchy):
//   - admin:    full access — dashboard, analytics, key management, probe trigger
//   - operator: dashboard, analytics, probe trigger, install script
//   - viewer:   dashboard, analytics (read-only)
//   - probe:    POST /api/report only
//
// Keys are SHA-256 hashed in PostgreSQL. Plaintext keys are shown once at creation.
package auth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
)

// Role is an RBAC role.
type Role string

const (
	RoleAdmin    Role = "admin"
	RoleOperator Role = "operator"
	RoleViewer   Role = "viewer"
	RoleProbe    Role = "probe"
)

type contextKey string

const roleCtxKey contextKey = "snitch_role"

// KeyInfo is a non-secret view of an API key.
type KeyInfo struct {
	ID        int64  `json:"id"`
	Role      Role   `json:"role"`
	Label     string `json:"label"`
	CreatedAt int64  `json:"created_at"`
	LastUsed  *int64 `json:"last_used,omitempty"`
}

// Auth manages API key authentication against PostgreSQL.
type Auth struct {
	browserSessions browserSessions
	pool            *pgxpool.Pool
}

// New creates an Auth manager. The api_keys table must already exist (created by store).
func New(pool *pgxpool.Pool) *Auth {
	return &Auth{pool: pool}
}

// CreateKey generates a new API key with the given role and label.
// Returns the plaintext key (show once to the operator, then discard).
func (a *Auth) CreateKey(ctx context.Context, role Role, label string) (string, error) {
	raw := make([]byte, 32)
	if _, err := io.ReadFull(rand.Reader, raw); err != nil {
		return "", fmt.Errorf("generate key: %w", err)
	}
	plain := "snitch_" + hex.EncodeToString(raw)
	hash := hashKey(plain)
	now := time.Now().Unix()

	_, err := a.pool.Exec(ctx,
		`INSERT INTO api_keys (key_hash, role, label, created_at) VALUES ($1, $2, $3, $4)`,
		hash, string(role), label, now)
	if err != nil {
		return "", fmt.Errorf("insert key: %w", err)
	}
	return plain, nil
}

// CreateKeyWithPlaintext registers a caller-supplied plaintext API key
// for the given role + label. Used by Block 61 bundle-create where the
// per-device probe key is generated inside the bundle issuance ceremony
// (offline-CA-rooted material flow) and the collector is told about the
// key via this admin path.
//
// Idempotent on label: re-issuing for the same device replaces the prior
// key under that label. Implemented as DELETE + INSERT inside one
// transaction so a partial failure doesn't strand the device with no
// admitted key. Re-issuance is the expected operator workflow when a
// bundle is rotated.
func (a *Auth) CreateKeyWithPlaintext(ctx context.Context, role Role, label, plain string) error {
	if plain == "" {
		return fmt.Errorf("plaintext key empty")
	}
	if label == "" {
		return fmt.Errorf("label empty")
	}
	hash := hashKey(plain)
	now := time.Now().Unix()

	tx, err := a.pool.Begin(ctx)
	if err != nil {
		return fmt.Errorf("begin tx: %w", err)
	}
	defer func() { _ = tx.Rollback(ctx) }() //nolint:errcheck // cleanup after commit or the returned transaction error

	if _, err := tx.Exec(ctx, `DELETE FROM api_keys WHERE label = $1`, label); err != nil {
		return fmt.Errorf("delete prior keys for label %q: %w", label, err)
	}
	if _, err := tx.Exec(ctx,
		`INSERT INTO api_keys (key_hash, role, label, created_at) VALUES ($1, $2, $3, $4)`,
		hash, string(role), label, now); err != nil {
		return fmt.Errorf("insert key: %w", err)
	}
	if err := tx.Commit(ctx); err != nil {
		return fmt.Errorf("commit: %w", err)
	}
	return nil
}

// ValidateKey checks a plaintext key and returns its role.
func (a *Auth) ValidateKey(ctx context.Context, plain string) (Role, error) {
	hash := hashKey(plain)
	var role string
	err := a.pool.QueryRow(ctx, `SELECT role FROM api_keys WHERE key_hash = $1`, hash).Scan(&role)
	if err != nil {
		return "", fmt.Errorf("invalid key")
	}
	// Best-effort last_used update — not on the auth-decision path; log
	// failures (Block 50E.3) but do not fail the request.
	if _, uerr := a.pool.Exec(ctx, `UPDATE api_keys SET last_used = $1 WHERE key_hash = $2`, time.Now().Unix(), hash); uerr != nil {
		log.Printf("[auth] last_used update failed: %v", uerr)
	}
	return Role(role), nil
}

// ListKeys returns all keys (without actual key values).
func (a *Auth) ListKeys(ctx context.Context) ([]KeyInfo, error) {
	rows, err := a.pool.Query(ctx, `SELECT id, role, label, created_at, last_used FROM api_keys ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var keys []KeyInfo
	for rows.Next() {
		var k KeyInfo
		var role string
		if err := rows.Scan(&k.ID, &role, &k.Label, &k.CreatedAt, &k.LastUsed); err != nil {
			return nil, err
		}
		k.Role = Role(role)
		keys = append(keys, k)
	}
	return keys, rows.Err()
}

// RevokeKey deletes a key by ID.
func (a *Auth) RevokeKey(ctx context.Context, id int64) error {
	tag, err := a.pool.Exec(ctx, `DELETE FROM api_keys WHERE id = $1`, id)
	if err != nil {
		return err
	}
	if tag.RowsAffected() == 0 {
		return fmt.Errorf("key %d not found", id)
	}
	return nil
}

// KeyCount returns the number of keys in the database.
//
// Block 50E.3 — surface the error: the previous signature swallowed the
// Scan error and returned 0, which made EnsureDefaults race-create
// default keys on transient DB failures.
func (a *Auth) KeyCount(ctx context.Context) (int, error) {
	var n int
	if err := a.pool.QueryRow(ctx, `SELECT COUNT(*) FROM api_keys`).Scan(&n); err != nil {
		return 0, fmt.Errorf("count api_keys: %w", err)
	}
	return n, nil
}

// EnsureDefaults creates admin and probe keys if none exist.
// Returns plaintext keys (empty if keys already existed).
func (a *Auth) EnsureDefaults(ctx context.Context) (adminKey, probeKey string, err error) {
	n, err := a.KeyCount(ctx)
	if err != nil {
		return "", "", fmt.Errorf("ensure-defaults: %w", err)
	}
	if n > 0 {
		return "", "", nil
	}
	adminKey, err = a.CreateKey(ctx, RoleAdmin, "default-admin")
	if err != nil {
		return "", "", fmt.Errorf("create admin key: %w", err)
	}
	probeKey, err = a.CreateKey(ctx, RoleProbe, "default-probe")
	if err != nil {
		return "", "", fmt.Errorf("create probe key: %w", err)
	}
	return adminKey, probeKey, nil
}

// Require returns middleware that authenticates and checks the caller has
// at least the given role before invoking next.
func (a *Auth) Require(minRole Role, next http.HandlerFunc) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		plain := extractKey(r)
		if plain == "" && r.Header.Get("Authorization") == "" {
			plain = a.browserSessions.credential(r)
		}
		if plain == "" {
			w.Header().Set("Content-Type", "application/json")
			http.Error(w, `{"error":"unauthorized","hint":"provide Authorization: Bearer <key>"}`, http.StatusUnauthorized)
			return
		}
		role, err := a.ValidateKey(r.Context(), plain)
		if err != nil {
			w.Header().Set("Content-Type", "application/json")
			http.Error(w, `{"error":"unauthorized","hint":"invalid API key"}`, http.StatusUnauthorized)
			return
		}
		if !canAccess(role, minRole) {
			w.Header().Set("Content-Type", "application/json")
			http.Error(w, `{"error":"forbidden","hint":"insufficient role"}`, http.StatusForbidden)
			return
		}
		ctx := context.WithValue(r.Context(), roleCtxKey, role)
		next(w, r.WithContext(ctx))
	})
}

// RoleFromContext extracts the authenticated role from request context.
func RoleFromContext(ctx context.Context) Role {
	role, _ := ctx.Value(roleCtxKey).(Role)
	return role
}

// LogAccess logs an authenticated API access.
//
// gosec G706 (log injection) false-positive: r.RemoteAddr is set by the
// http.Server from the accept-side socket; role is an internal Role enum
// (RoleAdmin / RoleOperator / RoleViewer / RoleProbe); r.Method is one of
// the canonical HTTP-method strings; action is a literal at every call
// site (e.g. "/bin/snitch", "/install", "/api/run"). No untrusted input
// reaches this format string.
func LogAccess(r *http.Request, action string) {
	role := RoleFromContext(r.Context())
	log.Printf("[auth] %s role=%s %s %s", r.RemoteAddr, role, r.Method, action) //nolint:gosec // see doc comment above
}

// canAccess checks if 'have' role can access resources requiring 'need' role.
func canAccess(have, need Role) bool {
	if have == need || have == RoleAdmin {
		return true
	}
	if have == RoleOperator && (need == RoleViewer || need == RoleProbe) {
		return true
	}
	return false
}

// extractKey reads the API key from the Authorization header only.
//
// Block 50E.2 — the previous implementation also accepted ?key=<plain>
// as a URL query parameter for the install-script-pipe-bash flow.
// That path was removed because URL queries land in proxy access logs,
// CDN caches, and shell history, exposing API keys at every hop. The
// install-pipe flow now uses curl -H "Authorization: Bearer ...".
func extractKey(r *http.Request) string {
	if auth := r.Header.Get("Authorization"); strings.HasPrefix(auth, "Bearer ") {
		return strings.TrimPrefix(auth, "Bearer ")
	}
	return ""
}

func hashKey(plain string) string {
	h := sha256.Sum256([]byte(plain))
	return hex.EncodeToString(h[:])
}
