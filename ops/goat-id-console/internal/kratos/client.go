// Package kratos is a small HTTP client for the Ory Kratos admin API.
// We hand-roll instead of pulling the official Ory Go SDK because the
// surface we need is small (list + get + patch traits + create + delete
// + sessions + recovery link) and the SDK pulls in heavy transitive deps.
//
// Authoritative API ref: https://www.ory.sh/docs/kratos/reference/api
package kratos

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

type Client struct {
	AdminURL string
	HTTP     *http.Client
}

func NewClient(adminURL string) *Client {
	return &Client{
		AdminURL: adminURL,
		HTTP:     &http.Client{Timeout: 10 * time.Second},
	}
}

// Identity is the subset of Kratos identity shape we render. Kratos returns
// a richer object; we only deserialize what we render or mutate.
type Identity struct {
	ID        string                 `json:"id"`
	SchemaID  string                 `json:"schema_id"`
	State     string                 `json:"state"`
	CreatedAt time.Time              `json:"created_at"`
	UpdatedAt time.Time              `json:"updated_at"`
	Traits    map[string]interface{} `json:"traits"`
	// MetadataPublic holds the authorization fields (groups, org, aff) that
	// moved off self-editable traits per F-572 / ADR 1188. Kratos writes it only
	// through the admin API, never through the self-service settings flow, so a
	// user cannot self-grant a group. It is returned to the identity (read-only)
	// on whoami and is present on the admin-API identity the consent flow reads,
	// so the JWT claim is unchanged. The authz accessors read it first and fall
	// back to Traits during the backfill window.
	MetadataPublic      map[string]interface{} `json:"metadata_public,omitempty"`
	VerifiableAddresses []VerifiableAddress    `json:"verifiable_addresses,omitempty"`
	// Credentials is populated only when the caller passes include_credential
	// on the list/get call (Kratos omits it otherwise). Kratos returns real
	// secret material under include_credential — the decrypted TOTP seed and
	// the lookup-secret backup codes ride in each credential's config — so the
	// value type is Credential, whose UnmarshalJSON keeps only the
	// enrolled-key count and discards everything else at decode time. The
	// console never retains, renders, or re-serializes secret material.
	Credentials map[string]Credential `json:"credentials,omitempty"`
}

// Credential is the console's reduced view of one Kratos credential: only
// what HasSecondFactor needs. The custom UnmarshalJSON is the security
// boundary — Kratos's include_credential responses carry secret material
// (TOTP seed, backup codes) inside config, and reducing here means it never
// outlives the JSON decode and cannot leak through any later Marshal of an
// Identity (this struct holds nothing secret to marshal).
type Credential struct {
	// EnrolledKeys is len(config.credentials) — the number of actually-enrolled
	// WebAuthn keys. Zero for the Kratos-auto-created webauthn placeholder and
	// for credential types whose config carries no credentials array (totp,
	// lookup_secret — for those, presence in the map is what matters).
	EnrolledKeys int `json:"enrolled_keys"`
}

func (c *Credential) UnmarshalJSON(data []byte) error {
	var raw struct {
		Config struct {
			Credentials []json.RawMessage `json:"credentials"`
		} `json:"config"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	c.EnrolledKeys = len(raw.Config.Credentials)
	return nil
}

// HasSecondFactor reports whether the identity carries any *enrolled* second
// factor (TOTP authenticator, WebAuthn security key, or backup/lookup-secret
// codes). Only meaningful when the identity was fetched with include_credential
// set for these types; otherwise Credentials is empty and this returns false.
//
// WebAuthn needs an enrolled-key check, not mere key-presence: when the identity
// schema declares email a webauthn identifier (the goat schema does), Kratos
// auto-creates a webauthn *placeholder* credential — a user_handle with no
// enrolled keys — for EVERY identity (see the ory role's kratos.yaml note and
// F-151). Counting that placeholder would falsely report 2FA for every
// password-only user, so webauthn only counts when config.credentials is
// non-empty (Credential.EnrolledKeys > 0). TOTP and lookup_secret are never
// placeholder-created (not schema identifiers), so their presence alone is a
// real factor. Verified against Kratos v1.3.1.
func (i Identity) HasSecondFactor() bool {
	if _, ok := i.Credentials["totp"]; ok {
		return true
	}
	if _, ok := i.Credentials["lookup_secret"]; ok {
		return true
	}
	return i.Credentials["webauthn"].EnrolledKeys > 0
}

type VerifiableAddress struct {
	ID       string `json:"id,omitempty"`
	Value    string `json:"value"`
	Verified bool   `json:"verified"`
	Via      string `json:"via"`
	Status   string `json:"status,omitempty"`
}

// VerifiedEmail is the verifiable_addresses entry a mint sends so the
// identity is born with its email verified. Kratos v1.3.1 marks an
// address verified only when a SELF-SERVICE recovery code is redeemed; the
// admin-minted welcome link never does, so an identity created with traits
// alone keeps email_verified=false for life and every OIDC fence that checks
// the claim (oauth2-proxy by default) refuses it. The admin asserted the
// address and the welcome package goes to it, which is the same footing the
// onboarding playbook and the site bootstrap already mint on.
func VerifiedEmail(email string) []VerifiableAddress {
	return []VerifiableAddress{{Value: email, Via: "email", Verified: true, Status: "completed"}}
}

// Email returns the identity's email trait if present.
func (i Identity) Email() string {
	if v, ok := i.Traits["email"].(string); ok {
		return v
	}
	return ""
}

// Name returns the identity's name trait. Kratos schemas vary — some use
// a flat string, some a {first, last} object. We try both shapes.
func (i Identity) Name() string {
	switch v := i.Traits["name"].(type) {
	case string:
		return v
	case map[string]interface{}:
		first, _ := v["first"].(string)
		last, _ := v["last"].(string)
		if first == "" && last == "" {
			return ""
		}
		if first == "" {
			return last
		}
		if last == "" {
			return first
		}
		return first + " " + last
	}
	return ""
}

// authzVal returns an authorization field (groups/org/aff/vendor) from
// admin-managed metadata_public, per F-572 / ADR 1188.
//
// Fail-closed (GHSA-9v3v): traits are never consulted, migrated or not. While
// ory_authz_in_metadata is false the holder can write traits through the
// settings flow, so a traits value is a claim, not an assignment. A
// not-yet-backfilled identity therefore reads as holding no groups, org or aff
// until the backfill moves them (ops/identity/backfill-authz-traits-to-metadata.py).
func (i Identity) authzVal(key string) interface{} {
	return i.MetadataPublic[key]
}

// Groups returns the identity's login groups — the array of strings that the
// patched consent UI (ADR 0857) forwards into the JWT groups claim.
func (i Identity) Groups() []string {
	arr, ok := i.authzVal("groups").([]interface{})
	if !ok {
		return nil
	}
	out := make([]string, 0, len(arr))
	for _, e := range arr {
		if s, ok := e.(string); ok {
			out = append(out, s)
		}
	}
	return out
}

func (i Identity) HasGroup(g string) bool {
	for _, x := range i.Groups() {
		if x == g {
			return true
		}
	}
	return false
}

// Org returns the identity's org affiliation, populated by the
// operator-onboard orchestrator from the manifest's org.{short_tag,
// display_name} block. It reads traits.org first (the ADR 1052 Phase 4
// successor) and falls back to the legacy traits.vendor block for
// identities minted before the trait migration / not yet backfilled.
// Operator-shape manifests carry neither block, so both return values are
// "" for them.
func (i Identity) Org() (shortTag, displayName string) {
	for _, key := range []string{"org", "vendor"} {
		v, ok := i.authzVal(key).(map[string]interface{})
		if !ok {
			continue
		}
		st, _ := v["short_tag"].(string)
		dn, _ := v["display_name"].(string)
		if st != "" || dn != "" {
			return st, dn
		}
	}
	return "", ""
}

// Aff returns the identity grammar's affiliation dimension as carried in
// traits.aff — the WHO of the identity (ADR 1043), distinct from Org() which is
// the display pair. Populated by operator-onboard.yml only when the goatnet has
// flipped ory_site_affiliation_trait_enabled; the trait is additive and the
// injection is default-OFF, so an identity minted on a goatnet that has not
// enabled it carries no aff block and every return value is "".
//
// coalition is deliberately NOT here and never will be: standing is
// nation-derived and effective-dated, so it rides the registry (currently-is)
// rather than a claim that would freeze a geopolitical fact at mint time
// (identity-grammar.md, ADR 1051). Callers wanting standing resolve `nation`
// through internal/coalition.
func (i Identity) Aff() (id, kind, nation string) {
	v, ok := i.authzVal("aff").(map[string]interface{})
	if !ok {
		return "", "", ""
	}
	id, _ = v["id"].(string)
	kind, _ = v["kind"].(string)
	nation, _ = v["nation"].(string)
	return id, kind, nation
}

type Session struct {
	ID                          string    `json:"id"`
	Active                      bool      `json:"active"`
	ExpiresAt                   time.Time `json:"expires_at"`
	AuthenticatedAt             time.Time `json:"authenticated_at"`
	IssuedAt                    time.Time `json:"issued_at"`
	AuthenticatorAssuranceLevel string    `json:"authenticator_assurance_level"`
	Devices                     []Device  `json:"devices"`
	// AuthenticationMethods is the per-factor ledger Kratos keeps on a
	// session: one entry per authentication event, each stamped with the
	// AAL it satisfied and WHEN it completed. This is the load-bearing
	// field for step-up (#174) — the session's own
	// AuthenticatorAssuranceLevel says only "this session has reached
	// aal2 at some point", which is not enough to gate a high-blast
	// mutation. The freshness window is measured off the newest aal2
	// entry's CompletedAt. See docs/design/id-admin-console.md
	// §"Auth + step-up".
	AuthenticationMethods []AuthenticationMethod `json:"authentication_methods"`
}

// AuthenticationMethod is one entry in a session's authentication ledger.
// Method is the Kratos credential type that was presented (password, totp,
// webauthn, lookup_secret, oidc, code, link_recovery, code_recovery); AAL is
// the assurance level THAT factor satisfied ("aal1" for a first factor,
// "aal2" for a second).
type AuthenticationMethod struct {
	Method      string    `json:"method"`
	AAL         string    `json:"aal"`
	CompletedAt time.Time `json:"completed_at"`
}

// FreshAAL2At returns the timestamp of the most recent aal2 authentication on
// this session, and whether one exists at all. A session that reached aal2
// hours ago returns that old timestamp — the caller applies the freshness
// window; this function does not decide policy.
//
// Kratos does not guarantee ordering of authentication_methods, so we scan for
// the max rather than trusting the last element.
func (s Session) FreshAAL2At() (time.Time, bool) {
	var best time.Time
	found := false
	for _, m := range s.AuthenticationMethods {
		if m.AAL != "aal2" {
			continue
		}
		if !found || m.CompletedAt.After(best) {
			best, found = m.CompletedAt, true
		}
	}
	return best, found
}

type Device struct {
	ID        string `json:"id"`
	IPAddress string `json:"ip_address"`
	UserAgent string `json:"user_agent"`
}

// ListIdentities returns all identities. The admin endpoint paginates;
// we follow page links until the server stops returning more rows or we
// hit the safety cap to avoid runaway memory in v1.
func (c *Client) ListIdentities(ctx context.Context) ([]Identity, error) {
	const safetyCap = 5000
	const pageSize = 250
	var out []Identity
	page := 0
	for {
		// include_credential returns each identity's second-factor state inline
		// in this same paginated call, so the users-list MFA column costs no
		// extra Kratos round-trip per row (no N+1). The secret material is not
		// rendered — we only read which credential types are present.
		u := fmt.Sprintf("%s/admin/identities?page=%d&per_page=%d"+
			"&include_credential=totp&include_credential=webauthn&include_credential=lookup_secret",
			c.AdminURL, page, pageSize)
		var batch []Identity
		if err := c.doJSON(ctx, http.MethodGet, u, nil, &batch); err != nil {
			return nil, err
		}
		out = append(out, batch...)
		if len(batch) < pageSize || len(out) >= safetyCap {
			break
		}
		page++
	}
	return out, nil
}

// CountIdentitiesWithGroup returns the number of identities whose
// metadata_public.groups contains `group`. Used by the last-ADMIN guard.
func (c *Client) CountIdentitiesWithGroup(ctx context.Context, group string) (int, error) {
	all, err := c.ListIdentities(ctx)
	if err != nil {
		return 0, err
	}
	n := 0
	for _, i := range all {
		if i.HasGroup(group) {
			n++
		}
	}
	return n, nil
}

func (c *Client) GetIdentity(ctx context.Context, id string) (*Identity, error) {
	// include_credential mirrors ListIdentities so HasSecondFactor is meaningful
	// on the single-identity fetch (the user-detail factor state). Secret
	// material is never rendered — only the presence of the credential types.
	u := fmt.Sprintf("%s/admin/identities/%s"+
		"?include_credential=totp&include_credential=webauthn&include_credential=lookup_secret",
		c.AdminURL, url.PathEscape(id))
	var i Identity
	if err := c.doJSON(ctx, http.MethodGet, u, nil, &i); err != nil {
		return nil, err
	}
	return &i, nil
}

// PatchGroups sets the login groups on admin-managed metadata_public, not on
// self-editable traits (F-572 / ADR 1188 — a user must not be able to
// self-grant a group through the settings flow).
//
// Kratos v1.3.1 enables EnsurePathExistsOnAdd for admin identity patches, so a
// single add on the child path creates metadata_public for a legacy identity
// when necessary. Touching only the child keeps sibling authorization metadata
// intact and avoids a client-side read/merge/write race. Kratos's admin patch
// endpoint accepts add, remove, and replace only; in particular, do not add a
// JSON-Patch test operation here.
func (c *Client) PatchGroups(ctx context.Context, id string, groups []string) (*Identity, error) {
	body := groupsPatch(groups)
	u := fmt.Sprintf("%s/admin/identities/%s", c.AdminURL, url.PathEscape(id))
	var i Identity
	if err := c.doJSON(ctx, http.MethodPatch, u, body, &i); err != nil {
		return nil, err
	}
	return &i, nil
}

// groupsPatch builds the one-operation patch accepted by Kratos v1.3.1.
func groupsPatch(groups []string) []map[string]interface{} {
	return []map[string]interface{}{
		{"op": "add", "path": "/metadata_public/groups", "value": groupsArray(groups)},
	}
}

// groupsArray returns v unchanged unless it is nil, which it replaces with an
// empty array. metadata_public.groups is the migration marker (F-572 / ADR
// 1188): a JSON null there reads as "not migrated" to the consent image and
// the jq and python readers, and they fall back to self-editable traits.
func groupsArray(v interface{}) interface{} {
	switch g := v.(type) {
	case nil:
		return []string{}
	case []string:
		if g == nil {
			return []string{}
		}
	case []interface{}:
		if g == nil {
			return []interface{}{}
		}
	}
	return v
}

// PatchState flips identity.state between "active" and "inactive".
// Kratos auto-deactivates fresh password-only identities when webauthn
// is enabled in passwordless mode and an enrolled key is missing (nova-a
// pattern observed); operators need to reactivate from the
// console without dropping to a shell-curl. The state value is validated
// at the caller; Kratos rejects anything other than "active" / "inactive".
func (c *Client) PatchState(ctx context.Context, id, state string) (*Identity, error) {
	body := []map[string]interface{}{
		{"op": "replace", "path": "/state", "value": state},
	}
	u := fmt.Sprintf("%s/admin/identities/%s", c.AdminURL, url.PathEscape(id))
	var i Identity
	if err := c.doJSON(ctx, http.MethodPatch, u, body, &i); err != nil {
		return nil, err
	}
	// Database revocation guards can retain inactive even when PATCH echoes
	// the requested state. Verify through a separate read before callers audit
	// success or tell an operator the account is active.
	stored, err := c.GetIdentity(ctx, id)
	if err != nil {
		return nil, fmt.Errorf("identity state change could not be verified: %w", err)
	}
	if stored.ID != id || stored.State != state {
		return nil, fmt.Errorf("identity state change did not persist: requested %q, observed %q; check the revocation ledger and explicit reinstatement", state, stored.State)
	}
	return stored, nil
}

// CreateIdentityRequest is a minimal create payload — enough for an
// admin to mint a new identity with name + email + initial groups, plus
// the verified email address (see VerifiedEmail).
type CreateIdentityRequest struct {
	SchemaID            string                 `json:"schema_id"`
	Traits              map[string]interface{} `json:"traits"`
	MetadataPublic      map[string]interface{} `json:"metadata_public,omitempty"`
	VerifiableAddresses []VerifiableAddress    `json:"verifiable_addresses,omitempty"`
}

// authzMetadataKeys are the authorization fields that must live in
// admin-managed metadata_public, never in self-editable traits (F-572 / ADR
// 1188). "vendor" is the legacy org block (ADR 1052) and rides along.
var authzMetadataKeys = []string{"groups", "org", "aff", "vendor"}

func (c *Client) CreateIdentity(ctx context.Context, req CreateIdentityRequest) (*Identity, error) {
	if req.SchemaID == "" {
		req.SchemaID = "default"
	}
	// Route authorization fields to metadata_public at this single chokepoint,
	// even when a caller still builds them into traits — so no mint path can seed
	// a group/org/aff where the self-service settings flow could edit it, and so
	// create stays valid once those fields leave the traits schema (F-572 / ADR
	// 1188).
	for _, k := range authzMetadataKeys {
		v, ok := req.Traits[k]
		if !ok {
			continue
		}
		if req.MetadataPublic == nil {
			req.MetadataPublic = map[string]interface{}{}
		}
		if _, exists := req.MetadataPublic[k]; !exists {
			req.MetadataPublic[k] = v
		}
		delete(req.Traits, k)
	}
	// Every new identity carries the migration marker, even with no groups:
	// without it a reader falls back to traits.groups, which the holder can
	// write through the settings flow.
	if req.MetadataPublic == nil {
		req.MetadataPublic = map[string]interface{}{}
	}
	req.MetadataPublic["groups"] = groupsArray(req.MetadataPublic["groups"])
	var i Identity
	u := fmt.Sprintf("%s/admin/identities", c.AdminURL)
	if err := c.doJSON(ctx, http.MethodPost, u, req, &i); err != nil {
		return nil, err
	}
	return &i, nil
}

func (c *Client) DeleteIdentity(ctx context.Context, id string) error {
	u := fmt.Sprintf("%s/admin/identities/%s", c.AdminURL, url.PathEscape(id))
	return c.doJSON(ctx, http.MethodDelete, u, nil, nil)
}

// DeleteIdentityCredential removes a single credential (e.g. "totp",
// "webauthn", "lookup_secret") from an identity via
// DELETE /admin/identities/{id}/credentials/{type}, leaving the identity and
// its other credentials intact. Used by the admin 2FA-reset flow.
//
// A 404 is treated as a benign no-op: it means the identity simply has no
// credential of that type, which is the desired end state anyway. Any other
// non-2xx is returned as an *HTTPError.
func (c *Client) DeleteIdentityCredential(ctx context.Context, id, credType string) error {
	u := fmt.Sprintf("%s/admin/identities/%s/credentials/%s",
		c.AdminURL, url.PathEscape(id), url.PathEscape(credType))
	err := c.doJSON(ctx, http.MethodDelete, u, nil, nil)
	var httpErr *HTTPError
	if errors.As(err, &httpErr) && httpErr.StatusCode == http.StatusNotFound {
		return nil
	}
	return err
}

// ListIdentitySessions returns the identity's ACTIVE sessions, paginating
// until exhausted (same pattern and page numbering as LastSessionByIdentity,
// which is proven live against the sibling /admin/sessions endpoint).
//
// Both properties are load-bearing for the step-up gate (goat#247 defect 4):
// the old single unpaginated call, with Kratos ordering unspecified, could
// leave a genuinely fresh aal2 session on a page that was never fetched —
// so a real verify would loop. active=true also keeps expired/revoked rows
// from padding the page the newest session must fit into.
//
// Page numbering, reconciled with ListIdentities (which starts at page=0):
// the two endpoints count pages DIFFERENTLY, and each caller matches its
// own. This endpoint's legacy paginator is 1-based with page=0 clamped to
// the first page — verified live against ZAGROS's Kratos v1.3.1 on
// (per_page=1: page=0 and page=1 return the same first row,
// page=2 the second). /admin/identities is 0-based with no clamp (kratos
// v1.3.1 persistence/sql/identity/persister_identity.go:917 wraps the
// request page as pop.NewPaginator(page+1, perPage)), so ListIdentities'
// page=0 start is also correct. Do not harmonize the start values —
// starting ListIdentities at 1 would silently skip the first 250
// identities. Starting at 1 here matches LastSessionByIdentity.
func (c *Client) ListIdentitySessions(ctx context.Context, id string) ([]Session, error) {
	const safetyCap = 2000
	const pageSize = 250
	var out []Session
	page := 1
	for {
		u := fmt.Sprintf("%s/admin/identities/%s/sessions?active=true&page=%d&per_page=%d",
			c.AdminURL, url.PathEscape(id), page, pageSize)
		var batch []Session
		if err := c.doJSON(ctx, http.MethodGet, u, nil, &batch); err != nil {
			return nil, err
		}
		out = append(out, batch...)
		if len(batch) < pageSize || len(out) >= safetyCap {
			break
		}
		page++
	}
	return out, nil
}

// SessionWithIdentity is the shape Kratos returns from /admin/sessions —
// each session carries an embedded identity. We use this to bucket-by-id
// in a single fleet-wide call for the list-page last-session column.
type SessionWithIdentity struct {
	Session
	Identity Identity `json:"identity"`
}

// LastSessionByIdentity returns a map[identity_id] -> most-recent active
// session issued_at, across the entire IdP, in a single Kratos call.
// Solves the N+1 problem the list-page would otherwise hit if it ran
// ListIdentitySessions per row. Paginates until exhausted or hits the
// safety cap.
//
// Negative results: identities with no active session are NOT in the
// returned map. Callers render "—" for the missing keys.
func (c *Client) LastSessionByIdentity(ctx context.Context) (map[string]time.Time, error) {
	const safetyCap = 5000
	const pageSize = 250
	out := map[string]time.Time{}
	page := 1
	for {
		u := fmt.Sprintf("%s/admin/sessions?active=true&page=%d&per_page=%d", c.AdminURL, page, pageSize)
		var batch []SessionWithIdentity
		if err := c.doJSON(ctx, http.MethodGet, u, nil, &batch); err != nil {
			return nil, err
		}
		for _, s := range batch {
			id := s.Identity.ID
			if id == "" {
				continue
			}
			if prev, ok := out[id]; !ok || s.IssuedAt.After(prev) {
				out[id] = s.IssuedAt
			}
		}
		if len(batch) < pageSize || len(out) >= safetyCap {
			break
		}
		page++
	}
	return out, nil
}

// LastActiveSession returns the most-recent session for the identity by
// issued_at, or nil if the identity has never authenticated. We sort
// in-process rather than relying on Kratos ordering (which is unspecified
// for this endpoint).
func (c *Client) LastActiveSession(ctx context.Context, id string) (*Session, error) {
	sessions, err := c.ListIdentitySessions(ctx, id)
	if err != nil {
		return nil, err
	}
	var best *Session
	for i := range sessions {
		s := sessions[i]
		if best == nil || s.IssuedAt.After(best.IssuedAt) {
			best = &s
		}
	}
	return best, nil
}

func (c *Client) RevokeIdentitySessions(ctx context.Context, id string) error {
	u := fmt.Sprintf("%s/admin/identities/%s/sessions", c.AdminURL, url.PathEscape(id))
	return c.doJSON(ctx, http.MethodDelete, u, nil, nil)
}

type RecoveryLink struct {
	RecoveryLink string    `json:"recovery_link"`
	RecoveryCode string    `json:"recovery_code"`
	ExpiresAt    time.Time `json:"expires_at"`
}

// MintRecoveryLink returns the one-time URL (and code) the user opens to set
// their credentials.
//
// Kratos has routed this admin endpoint under several paths across versions,
// and the path depends on the recovery strategy. On v1.3.x with the `code`
// strategy (our default when ory_recovery_enabled is on) the live endpoint is
// POST /admin/recovery/code, which returns both recovery_link and
// recovery_code. The older `/admin/recovery/link` (link strategy) and the
// `/admin/recovery/link/code/admin` alias 404 on v1.3.x ("endpoint disabled" /
// "not found"). We try the code endpoint first and fall back to the legacy
// aliases so the console keeps working against older Kratos builds. Recovery
// must be enabled in kratos.yaml (ory_recovery_enabled) or every path 400s
// with "Recovery is not allowed because it was disabled".
func (c *Client) MintRecoveryLink(ctx context.Context, identityID string) (*RecoveryLink, error) {
	body := map[string]interface{}{"identity_id": identityID}
	candidates := []string{
		"/admin/recovery/code",            // v1.3.x code strategy (current)
		"/admin/recovery/link/code/admin", // alias on some builds
		"/admin/recovery/link",            // legacy link strategy
	}
	var lastErr error
	for _, p := range candidates {
		u := c.AdminURL + p
		var out RecoveryLink
		err := c.doJSON(ctx, http.MethodPost, u, body, &out)
		if err == nil {
			return &out, nil
		}
		var httpErr *HTTPError
		if errors.As(err, &httpErr) && httpErr.StatusCode == http.StatusNotFound {
			lastErr = err
			continue
		}
		return nil, err
	}
	return nil, fmt.Errorf("no Kratos recovery-link endpoint matched: %w", lastErr)
}

// HTTPError carries the status code so callers can distinguish 4xx vs 5xx.
type HTTPError struct {
	StatusCode int
	Body       string
	URL        string
	Method     string
}

func (e *HTTPError) Error() string {
	return fmt.Sprintf("kratos %s %s: %d: %s", e.Method, e.URL, e.StatusCode, truncate(e.Body, 200))
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "…"
}

// doJSON performs an HTTP request, encodes `in` as JSON, and decodes the
// JSON response into `out`. Either can be nil. Non-2xx responses are
// returned as *HTTPError.
func (c *Client) doJSON(ctx context.Context, method, u string, in, out interface{}) error {
	var body io.Reader
	if in != nil {
		buf, err := json.Marshal(in)
		if err != nil {
			return fmt.Errorf("marshal request: %w", err)
		}
		body = bytes.NewReader(buf)
	}
	req, err := http.NewRequestWithContext(ctx, method, u, body)
	if err != nil {
		return err
	}
	if in != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if method == http.MethodPatch {
		// Kratos PATCH on identity uses RFC6902 JSON Patch.
		req.Header.Set("Content-Type", "application/json-patch+json")
	}
	req.Header.Set("Accept", "application/json")

	resp, err := c.HTTP.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	raw, readErr := io.ReadAll(resp.Body)
	if resp.StatusCode >= 400 {
		body := string(raw)
		if readErr != nil {
			if strings.TrimSpace(body) != "" {
				body = fmt.Sprintf("%s (body read error: %v)", strings.TrimSpace(body), readErr)
			} else {
				body = fmt.Sprintf("body read error: %v", readErr)
			}
		}
		return &HTTPError{
			StatusCode: resp.StatusCode,
			Body:       body,
			URL:        u,
			Method:     method,
		}
	}
	if out == nil {
		return nil
	}
	if readErr != nil {
		return fmt.Errorf("read response body: %w", readErr)
	}
	if len(raw) == 0 {
		return nil
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return fmt.Errorf("decode response (%d bytes): %w", len(raw), err)
	}
	return nil
}

// FormatExpiresIn renders a human-readable "in 23m" / "12d ago" string for
// session timestamps in templates. Lives here so the kratos package owns
// the time-string format consistent with its data.
func FormatExpiresIn(t time.Time) string {
	if t.IsZero() {
		return "—"
	}
	d := time.Until(t)
	if d > 0 {
		return "in " + shortDuration(d)
	}
	return shortDuration(-d) + " ago"
}

func shortDuration(d time.Duration) string {
	if d < time.Minute {
		return strconv.Itoa(int(d.Seconds())) + "s"
	}
	if d < time.Hour {
		return strconv.Itoa(int(d.Minutes())) + "m"
	}
	if d < 24*time.Hour {
		return strconv.Itoa(int(d.Hours())) + "h"
	}
	return strconv.Itoa(int(d.Hours()/24)) + "d"
}
