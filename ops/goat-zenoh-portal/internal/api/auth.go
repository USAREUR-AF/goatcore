// Package api wires the HTTP layer for goat-zenoh-portal.
//
// Bearer-token authentication per docs/design/zenoh-trial-fabric.md
// §"Open design decisions" #6. Two principal classes:
//
//	operator  — super-user; the token equals the configured operator API key
//	vendor    — bcrypt-checked against the api_key_hash row in vendors
//
// The Zenoh-wire identity is the per-vendor X.509 cert + URI-SAN; the
// portal API key only authenticates to the portal. They are distinct
// credentials by design.
package api

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"errors"
	"log/slog"
	"net/http"
	"regexp"
	"strings"
	"sync/atomic"

	"golang.org/x/crypto/bcrypt"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// preHashForBcrypt collapses a long API key down to 64 hex chars (32 bytes
// of SHA-256, 64 hex bytes — under bcrypt's 72-byte plaintext ceiling) so
// bcrypt's slow-hash property still applies regardless of token length.
//
// This is the standard "use bcrypt on a fast hash of the password" pattern
// for fixed-length high-entropy tokens; security loss is zero because
// our tokens already have 256 bits of entropy and bcrypt's cost factor
// covers offline cracking. Without it bcrypt silently truncates at byte
// 72, which is the actual security gap.
func preHashForBcrypt(token string) []byte {
	sum := sha256.Sum256([]byte(token))
	enc := hex.EncodeToString(sum[:])
	return []byte(enc)
}

// defaultOperatorHandle is the actor a legacy single OperatorAPIKey match
// resolves to, so deployments that haven't moved to per-person keys keep
// logging a stable (if shared) actor instead of an empty one.
const defaultOperatorHandle = "operator"

// Principal identifies the authenticated caller. Vendor is non-empty for
// vendor-scoped principals; Operator is true for the super-user.
type Principal struct {
	Operator bool
	Vendor   string // vendor prefix when not operator
	// Actor is the resolved person handle for an operator principal.
	// Empty for vendors (vendor identity is in Vendor); set to
	// the per-person handle, or the legacy "operator" default, for
	// operators.
	Actor string
}

type ctxKey int

const principalKey ctxKey = 1

// PrincipalFromContext returns the Principal attached by AuthMiddleware,
// or nil when unauthenticated.
func PrincipalFromContext(ctx context.Context) *Principal {
	if v, ok := ctx.Value(principalKey).(*Principal); ok {
		return v
	}
	return nil
}

// actorOf returns the person handle to attribute an audited action to.
// For an operator principal that's the resolved Actor; it falls
// back to the shared "operator" label only if the principal is somehow
// unset, so an audit row never carries an empty actor.
func actorOf(r *http.Request) string {
	p := PrincipalFromContext(r.Context())
	// Delegated attribution: a trusted operator-key holder that authenticates the
	// human itself but calls us with ONE shared key (goat-id-console — oauth2-proxy
	// ADMINS gate, then the shared portal operator bearer) can name that human via
	// X-Goat-Operator-Actor, so the sponsor/decider recorded is the real person and
	// not the generic "operator". Honored only for an operator principal — holding a
	// valid operator key IS the trust boundary.
	if p != nil && p.Operator {
		if a := strings.TrimSpace(r.Header.Get("X-Goat-Operator-Actor")); a != "" {
			return a
		}
	}
	if p != nil && p.Actor != "" {
		return p.Actor
	}
	return defaultOperatorHandle
}

// extractToken pulls the Bearer token from the Authorization header. Empty
// when missing or malformed.
func extractToken(r *http.Request) string {
	h := r.Header.Get("Authorization")
	if !strings.HasPrefix(h, "Bearer ") {
		return ""
	}
	return strings.TrimSpace(strings.TrimPrefix(h, "Bearer "))
}

// AuthMiddleware authenticates the request. Routes that need auth wrap
// their handler with this; the principal lands in the request context.
// When required is false, unauthenticated requests pass through with no
// principal attached (used for /api/health and the org/vendor bundle
// download routes, which authenticate via a path token instead).
func (s *Server) AuthMiddleware(required bool, next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		token := extractToken(r)
		if token == "" && r.Header.Get("Authorization") == "" {
			token = s.browserSessions.credential(r)
		}
		if token == "" {
			if required {
				writeError(w, http.StatusUnauthorized, "auth", "missing bearer token")
				return
			}
			next(w, r)
			return
		}
		p, err := s.resolvePrincipal(r.Context(), token)
		if err != nil || p == nil {
			writeError(w, http.StatusUnauthorized, "auth", "invalid token")
			return
		}
		// Stamp the access-log principal on the shared statusWriter so the
		// outer middleware can log it (the augmented context only flows
		// downward). F-207.
		setPrincipalLabel(w, p)
		next(w, r.WithContext(context.WithValue(r.Context(), principalKey, p)))
	}
}

// resolvePrincipal tries operator first (constant-time compare), then resolves
// a vendor token by indexed digest lookup.
//
// WHY NO WALK. Walking every vendor row and running bcrypt against each
// api_key_hash makes an unauthenticated request expensive: a bogus bearer token
// walks all rows, turning one cheap HTTP request into CPU-seconds, with no
// credential at all. An edge rate limit is no defence when a single request is
// that expensive.
//
// Three changes, in the order they execute — each one cheaper than the last
// thing it protects:
//
//  1. SHAPE CHECK. Every token this portal has ever minted is
//     `vp_<prefix>_<64 hex>` (generateAPIKey, unchanged since the portal's
//     first commit). A token that doesn't parse cannot match any row, so it
//     is rejected before touching the database, let alone a hash function.
//     This is what closes the unauthenticated amplifier.
//  2. INDEXED LOOKUP. hex(sha256(token)) against vendors.api_key_sha256 —
//     one index probe, no key derivation, cost independent of vendor count.
//  3. ONE bcrypt, AT MOST. A row minted before the digest column carries an
//     empty api_key_sha256. The token names its own vendor, and `prefix` is
//     the immutable primary key with no rename path anywhere in the
//     registry, so the hint is authoritative: fetch THAT row and bcrypt
//     exactly it. On a match the digest is backfilled and the row never
//     costs a bcrypt again. There is no walk left, at any vendor count.
//
// Worst case is now one bcrypt (~70 ms) for a token naming a real, not-yet-
// backfilled vendor; steady state is a single indexed SELECT. Convergence is
// observable via CountVendorsAwaitingAPIKeyDigest on /api/health; when that
// reads 0 fleet-wide, step 3 can be deleted.
func (s *Server) resolvePrincipal(ctx context.Context, token string) (*Principal, error) {
	// Per-person operator keys and the legacy single key, all via
	// constant-time compare. Compare against every configured key before
	// returning so timing doesn't leak which handle (if any) matched.
	tokenB := []byte(token)
	matchedActor := ""
	if s.cfg.OperatorKeys != nil {
		matchedActor = s.cfg.OperatorKeys.Match(tokenB)
	} else {
		// Tests and callers that build a Config literal without going through
		// Load still get the env map honoured rather than silently no-auth.
		for handle, key := range s.cfg.OperatorAPIKeys {
			if subtle.ConstantTimeCompare(tokenB, []byte(key)) == 1 {
				matchedActor = handle
			}
		}
	}
	if s.cfg.OperatorAPIKey != "" && subtle.ConstantTimeCompare(tokenB, []byte(s.cfg.OperatorAPIKey)) == 1 {
		if matchedActor == "" {
			matchedActor = defaultOperatorHandle
		}
	}
	if matchedActor != "" {
		return &Principal{Operator: true, Actor: matchedActor}, nil
	}
	// (1) Shape check — free rejection of anything that was never a token.
	hintPrefix, ok := parseVendorToken(token)
	if !ok {
		return nil, nil
	}

	// (2) Indexed digest lookup — the steady-state path.
	digest := apiKeyDigest(token)
	v, err := s.reg.VendorByAPIKeySHA256(ctx, digest)
	if err == nil && v != nil {
		return &Principal{Vendor: v.Prefix}, nil
	}
	if err != nil && !errors.Is(err, registry.ErrNotFound) {
		// ErrAmbiguous (two rows, one digest) lands here and fails closed.
		return nil, err
	}

	// (3) Legacy row, not yet backfilled: at most one bcrypt, on the row the
	// token names. A miss here is a 401, not a reason to go looking elsewhere.
	v, err = s.reg.GetVendor(ctx, hintPrefix)
	if err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			return nil, nil
		}
		return nil, err
	}
	if v.APIKeyHash == "" || v.APIKeySHA256 != "" {
		// No credential on the row, or it already has a digest and step (2)
		// authoritatively said this token is not it.
		return nil, nil
	}
	apiKeyBcryptCompares.Add(1)
	if bcrypt.CompareHashAndPassword([]byte(v.APIKeyHash), preHashForBcrypt(token)) != nil {
		return nil, nil
	}
	// Backfill so this key never pays for bcrypt again. A failure here is not
	// an auth failure — the caller proved the credential; we just stay slow for
	// them until the next successful attempt.
	if err := s.reg.SetVendorAPIKeySHA256(ctx, v.Prefix, digest); err != nil {
		slog.Warn("api-key digest backfill failed; this key keeps paying bcrypt per request",
			"vendor", v.Prefix, "err", err)
	}
	return &Principal{Vendor: v.Prefix}, nil
}

// apiKeyBcryptCompares counts how many times authentication has fallen back to
// the legacy per-row bcrypt compare since this process started. Surfaced on
// /api/health next to the awaiting-digest count.
//
// It is the number that tells you whether F-371's slow path is still load-
// bearing: on a converged goatnet it stops climbing, and once it is flat at a
// zero awaiting-digest count the fallback leg can be deleted. It also gives
// the tests a deterministic assertion — "this request cost exactly one bcrypt,
// not one per vendor" — instead of a flaky timing bound.
var apiKeyBcryptCompares atomic.Int64

// APIKeyBcryptCompares reports that counter for the health surface.
func APIKeyBcryptCompares() int64 { return apiKeyBcryptCompares.Load() }

// vendorTokenRe matches the ONLY token shape this portal mints:
// "vp_" + <vendor prefix> + "_" + 64 lowercase hex chars (32 random bytes).
// The prefix charset mirrors registry.ValidVendorPrefix (lowercase ASCII,
// digits, '-'), and anchors in the per-uuid identity model are 32 hex chars,
// which that charset already covers.
//
// Anchored at both ends on purpose: this is a gate, not a search.
var vendorTokenRe = regexp.MustCompile(`^vp_([a-z0-9-]{1,128})_([0-9a-f]{64})$`)

// parseVendorToken returns the vendor prefix a token names, and whether the
// token is even shaped like one this portal issued. A false here costs the
// caller nothing beyond a regex match — that is the entire point.
func parseVendorToken(token string) (string, bool) {
	m := vendorTokenRe.FindStringSubmatch(token)
	if m == nil {
		return "", false
	}
	return m[1], true
}

// apiKeyDigest is the indexed lookup key: hex(sha256(plaintext token)).
//
// Deliberately NOT a slow hash. These tokens are 32 bytes of crypto/rand, so
// there is no candidate space to search and a work factor protects nothing;
// what it did instead was let any caller spend the portal's CPU.
func apiKeyDigest(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

// requireOperator wraps next so only an Operator principal can reach it.
func (s *Server) requireOperator(next http.HandlerFunc) http.HandlerFunc {
	return s.AuthMiddleware(true, func(w http.ResponseWriter, r *http.Request) {
		p := PrincipalFromContext(r.Context())
		if p == nil || !p.Operator {
			writeError(w, http.StatusForbidden, "auth", "operator-only endpoint")
			return
		}
		next(w, r)
	})
}

// requireVendorScope wraps next so either an Operator OR the matching
// vendor can reach it. The expected vendor comes from the URL path (set
// by the router before calling).
func (s *Server) requireVendorScope(vendor string, next http.HandlerFunc) http.HandlerFunc {
	return s.AuthMiddleware(true, func(w http.ResponseWriter, r *http.Request) {
		p := PrincipalFromContext(r.Context())
		if p == nil {
			writeError(w, http.StatusUnauthorized, "auth", "missing principal")
			return
		}
		if p.Operator {
			next(w, r)
			return
		}
		if p.Vendor != vendor {
			writeError(w, http.StatusForbidden, "auth", "vendor scope mismatch")
			return
		}
		next(w, r)
	})
}

// hashAPIKey derives BOTH stored credential columns for a freshly minted
// token: the bcrypt hash (kept so a rollback to the pre-F-371 image still
// authenticates every key minted in the meantime) and the sha256 digest that
// authentication actually looks up.
//
// It returns both rather than two separate helpers so a mint site cannot
// write one and forget the other — a row with a bcrypt hash and no digest
// silently costs a bcrypt on every request, and a row with a digest and no
// bcrypt hash would stop working on rollback. The compiler enforces it.
//
// Called once per vendor at registration/rotation. bcrypt's input is
// pre-hashed to keep us under its 72-byte plaintext ceiling.
func hashAPIKey(plaintext string) (bcryptHash, sha256Digest string, err error) {
	b, err := bcrypt.GenerateFromPassword(preHashForBcrypt(plaintext), bcrypt.DefaultCost)
	if err != nil {
		return "", "", err
	}
	return string(b), apiKeyDigest(plaintext), nil
}

// _ keeps the registry import live in case future maintainers move
// resolution logic. ListVendors above already references it; this is a
// no-op assertion.
var _ = (*registry.Registry)(nil)
