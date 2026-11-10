package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// CollabProvider delivers a person's access to the cloud collaboration
// environment when their onboarding manifest carries `collab.enabled: true`.
//
// It is a SWAPPABLE, provider-agnostic seam (docs/design/collab-onboarding-provisioner.md):
// today the only implementation mints an account in our own Dex+Kanidm plane
// (an identity plane SEPARATE from the mesh Ory/Hydra); once the collab env is
// offloaded to an environment we do not fully control, a new implementation
// performs a HANDOFF into that environment's own onboarding flow. The mesh
// onboarding engine NEVER governs the collab account — this leg is best-effort
// and non-fatal, and nothing in an enforcement path depends on it.
type CollabProvider interface {
	// Name is the provider slug recorded as `collab.provider`.
	Name() string
	// Provision ensures an account exists for (handle, email, displayName) and
	// returns the account identifier plus a one-time credential-setup URL the
	// person completes themselves (passkey, or password + TOTP — MFA required).
	//
	// Idempotent, and "already there" is ACHIEVED, not failed: when the account
	// already exists the account id comes back with an EMPTY setupURL and no
	// error. The empty URL is deliberate — minting a fresh credential-setup link
	// for a live account hands out an account-takeover link for credentials that
	// already work, and re-onboarding a person who kept their collab access is
	// exactly the F-274 case. The caller records the account as provisioned and
	// omits the setup link.
	Provision(ctx context.Context, handle, email, displayName string) (account, setupURL string, err error)
	// Deprovision disables the person's collab account. Idempotent: an absent
	// account is success (nothing to cut). NEVER called by the onboarding runner
	// — collab revocation is a DELIBERATE operator act, because a mesh revoke
	// does not imply a collab revoke (docs/design/collab-onboarding-provisioner.md
	// §"Revocation is a separate decision").
	Deprovision(ctx context.Context, handle string) error
	// Exists reports whether a collab account is present for handle — the
	// read-only half operators and reconcilers use; no side effects.
	Exists(ctx context.Context, handle string) (bool, error)
}

// manifestFacts is the minimal slice of a manifest_json the collab leg reads.
// Everything else in the manifest is irrelevant to collab provisioning by
// design — collab accounts are flat (see the provider's Provision doc).
type manifestFacts struct {
	Identity struct {
		Name  string `json:"name"`
		Email string `json:"email"`
	} `json:"identity"`
	Collab struct {
		Enabled  bool   `json:"enabled"`
		Provider string `json:"provider"`
	} `json:"collab"`
}

// parseManifestFacts extracts the collab-relevant fields from a manifest_json.
// Best-effort: an unparseable manifest yields the zero value (collab disabled),
// which is safe — the leg then no-ops.
func parseManifestFacts(manifestJSON string) manifestFacts {
	var m manifestFacts
	_ = json.Unmarshal([]byte(manifestJSON), &m)
	return m
}

// newCollabProviderFromEnv builds the configured collab provider, or nil when
// collab provisioning is not enabled/configured for this runner (the leg then
// no-ops — the common case for goatnets without a collab env). Master switch
// COLLAB_PROVISION_ENABLED; today only the "goat-collab" (Dex/Kanidm) provider
// exists.
func newCollabProviderFromEnv(timeout time.Duration, log *slog.Logger) CollabProvider {
	if !envBool("COLLAB_PROVISION_ENABLED") {
		return nil
	}
	// Prefer a scoped service-account API token (least privilege — it can only
	// create persons + mint credential reset intents). Fall back to the idm_admin
	// password only when no token is configured; that path puts a full-admin
	// credential on this host, so the token is the recommended production path.
	apiToken := os.Getenv("COLLAB_KANIDM_API_TOKEN")
	pw := os.Getenv("COLLAB_KANIDM_IDM_ADMIN_PW")
	if apiToken == "" && pw == "" {
		log.Warn("COLLAB_PROVISION_ENABLED set but neither COLLAB_KANIDM_API_TOKEN nor COLLAB_KANIDM_IDM_ADMIN_PW is set — collab leg stays disabled")
		return nil
	}
	// No built-in door: the role renders COLLAB_KANIDM_URL on every goatnet that
	// enables the leg (and renders no credential without it).
	base := strings.TrimRight(os.Getenv("COLLAB_KANIDM_URL"), "/")
	if base == "" {
		log.Warn("COLLAB_PROVISION_ENABLED set but COLLAB_KANIDM_URL is empty — collab leg stays disabled")
		return nil
	}
	authMode := "service-account-token"
	if apiToken == "" {
		authMode = "idm_admin-password"
	}
	log.Info("collab provisioning enabled", "provider", "goat-collab", "kanidm", base, "auth", authMode)
	return &kanidmProvider{base: base, apiToken: apiToken, adminPW: pw, hc: &http.Client{Timeout: timeout}}
}

// kanidmProvider mints accounts in our own Dex+Kanidm collab plane. It folds the
// logic of the interim ops/onboard-collab.py into the engine: authenticate →
// create the person (FLAT — name/displayname/mail only, NO mesh groups, per the
// loosely-coupled encoding decision) → mint a 24h one-time credential-setup link.
// Auth is a scoped service-account bearer (apiToken) when set, else the idm_admin
// 3-step password flow.
type kanidmProvider struct {
	base     string
	apiToken string // scoped service-account bearer (preferred); empty ⇒ use adminPW
	adminPW  string // idm_admin password (fallback 3-step auth)
	hc       *http.Client
}

func (k *kanidmProvider) Name() string { return "goat-collab" }

// SignInURL is the collab front a person signs in at once their account is set
// up: COLLAB_SIGNIN_URL when set, else the Kanidm door with its leading "idm."
// label dropped (the collab plane serves Kanidm at idm.<front>).
func (k *kanidmProvider) SignInURL() string {
	if v := strings.TrimRight(os.Getenv("COLLAB_SIGNIN_URL"), "/"); v != "" {
		return v
	}
	u, err := url.Parse(k.base)
	if err != nil || u.Host == "" {
		return k.base
	}
	u.Host = strings.TrimPrefix(u.Host, "idm.")
	u.Path, u.RawQuery, u.Fragment = "", "", ""
	return u.String()
}

// collabSignInURL asks a provider for its sign-in front; a provider without one
// gets the generic wording.
func collabSignInURL(p CollabProvider) string {
	if s, ok := p.(interface{ SignInURL() string }); ok {
		if v := s.SignInURL(); v != "" {
			return v
		}
	}
	return "the collaboration environment"
}

func (k *kanidmProvider) Provision(ctx context.Context, handle, email, displayName string) (string, string, error) {
	if handle == "" || email == "" {
		return "", "", fmt.Errorf("collab: handle and email are both required")
	}
	if displayName == "" {
		displayName = handle
	}
	tok, err := k.token(ctx)
	if err != nil {
		return "", "", fmt.Errorf("collab: auth: %w", err)
	}
	// Create the person. Kanidm answers a successful create with HTTP 200 + body
	// "null". A repeat create for someone who already has an account answers
	// HTTP 409 + an attribute-uniqueness body — VERIFIED live:
	//
	//   {"conflicting_attributes":["mail","name","spn"],"error":"Attribute uniqueness error"}
	//
	// That is ACHIEVED, not failed: the account we were asked to ensure exists,
	// exists. Return it with NO setup link (see the interface doc — never mint a
	// fresh credential-reset link for an account whose credentials already work).
	// The earlier tolerance test matched on the strings "already exists" /
	// "duplicate", which this server never emits, so every re-onboard of a
	// collab-retaining person was recorded as a collab FAILURE.
	body := map[string]any{"attrs": map[string][]string{
		"name": {handle}, "displayname": {displayName}, "mail": {email}}}
	st, rb, err := k.call(ctx, http.MethodPost, "/v1/person", tok, body)
	if err != nil {
		return "", "", fmt.Errorf("collab: create person: %w", err)
	}
	if isAlreadyExists(st, rb) {
		return handle, "", nil
	}
	if st != http.StatusOK && st != http.StatusCreated {
		if s := strings.TrimSpace(rb); s != "null" && s != "" {
			return "", "", fmt.Errorf("collab: create person: HTTP %d: %s", st, snippet([]byte(rb)))
		}
	}
	// Mint a one-time credential-setup link. The TTL is a PATH segment
	// (_update_intent/<seconds>), NOT a query param — and the Kanidm default is
	// only 1h, useless for an emailed onboarding link (recipients hit "token not
	// found" once it lapses). 86400s = 24h is the real ceiling.
	//
	// It is NOT "clamped to the account-policy authsession_expiry" as this
	// comment previously claimed — that was a guess and it is wrong. Measured
	// against the live server:
	//
	//	requested 604800s (7 days) -> granted 86400s (24h exactly)
	//	requested      3s          -> granted   300s (5 min floor)
	//
	// The ceiling is a server constant. `kanidm group account-policy` offers
	// auth-expiry and privilege-expiry and nothing for credential-update
	// intents, so no goatnet can raise it by configuration. Asking for more
	// than 24h here would silently get 24h and make any "valid for N days"
	// wording in the welcome email a lie.
	//
	// The response body carries `expiry_time` (unix seconds) — the expiry the
	// server actually chose. Anything that tells a person when their code dies
	// should read that field rather than computing now+TTL.
	st, rb, err = k.call(ctx, http.MethodGet, "/v1/person/"+handle+"/_credential/_update_intent/86400", tok, nil)
	if err != nil {
		return "", "", fmt.Errorf("collab: mint reset intent: %w", err)
	}
	var intent struct {
		Token string `json:"token"`
	}
	if err := json.Unmarshal([]byte(rb), &intent); err != nil || intent.Token == "" {
		return "", "", fmt.Errorf("collab: mint reset intent: HTTP %d: %s", st, snippet([]byte(rb)))
	}
	return handle, k.base + "/ui/reset?token=" + intent.Token, nil
}

// isAlreadyExists reports whether a create response means "this person is
// already provisioned". Kanidm signals it as HTTP 409 + an attribute-uniqueness
// body (verified live); the string forms are kept as belt-and-braces
// for other providers behind this seam and for older/newer server wording.
func isAlreadyExists(status int, body string) bool {
	if status == http.StatusConflict {
		return true
	}
	lb := strings.ToLower(body)
	return strings.Contains(lb, "already exists") ||
		strings.Contains(lb, "duplicate") ||
		strings.Contains(lb, "uniqueness")
}

// CollabAccount is one account as the provider sees it — the inventory row the
// operator surfaces and the F-274 reconcile consume.
type CollabAccount struct {
	Handle      string
	Email       string
	DisplayName string
}

// List enumerates every collab account. Read-only; operator/reconcile surface.
func (k *kanidmProvider) List(ctx context.Context) ([]CollabAccount, error) {
	tok, err := k.token(ctx)
	if err != nil {
		return nil, fmt.Errorf("collab: auth: %w", err)
	}
	st, rb, err := k.call(ctx, http.MethodGet, "/v1/person", tok, nil)
	if err != nil {
		return nil, fmt.Errorf("collab: list persons: %w", err)
	}
	if st != http.StatusOK {
		return nil, fmt.Errorf("collab: list persons: HTTP %d: %s", st, snippet([]byte(rb)))
	}
	var raw []struct {
		Attrs map[string][]string `json:"attrs"`
	}
	if err := json.Unmarshal([]byte(rb), &raw); err != nil {
		return nil, fmt.Errorf("collab: list persons: decode: %w", err)
	}
	first := func(m map[string][]string, k string) string {
		if v := m[k]; len(v) > 0 {
			return v[0]
		}
		return ""
	}
	out := make([]CollabAccount, 0, len(raw))
	for _, r := range raw {
		out = append(out, CollabAccount{
			Handle:      first(r.Attrs, "name"),
			Email:       first(r.Attrs, "mail"),
			DisplayName: first(r.Attrs, "displayname"),
		})
	}
	return out, nil
}

// Exists reports whether the collab account is present. Read-only.
func (k *kanidmProvider) Exists(ctx context.Context, handle string) (bool, error) {
	if handle == "" {
		return false, fmt.Errorf("collab: handle required")
	}
	tok, err := k.token(ctx)
	if err != nil {
		return false, fmt.Errorf("collab: auth: %w", err)
	}
	st, rb, err := k.call(ctx, http.MethodGet, "/v1/person/"+handle, tok, nil)
	if err != nil {
		return false, fmt.Errorf("collab: get person: %w", err)
	}
	switch {
	case st == http.StatusNotFound:
		return false, nil
	case st != http.StatusOK:
		return false, fmt.Errorf("collab: get person: HTTP %d: %s", st, snippet([]byte(rb)))
	}
	// A 200 with a null/empty body means "no such person" on this server.
	if s := strings.TrimSpace(rb); s == "null" || s == "" {
		return false, nil
	}
	return true, nil
}

// Deprovision deletes the person's collab account. Idempotent: an account that
// is already gone is success. This is the cut a mesh revoke deliberately does
// NOT perform — see the interface doc.
func (k *kanidmProvider) Deprovision(ctx context.Context, handle string) error {
	if handle == "" {
		return fmt.Errorf("collab: handle required")
	}
	tok, err := k.token(ctx)
	if err != nil {
		return fmt.Errorf("collab: auth: %w", err)
	}
	st, rb, err := k.call(ctx, http.MethodDelete, "/v1/person/"+handle, tok, nil)
	if err != nil {
		return fmt.Errorf("collab: delete person: %w", err)
	}
	if st == http.StatusOK || st == http.StatusNoContent || st == http.StatusNotFound {
		return nil
	}
	return fmt.Errorf("collab: delete person: HTTP %d: %s", st, snippet([]byte(rb)))
}

// token returns the bearer to use: a preconfigured scoped service-account token
// when present (no round-trip), else a freshly minted idm_admin session token.
func (k *kanidmProvider) token(ctx context.Context) (string, error) {
	if k.apiToken != "" {
		return k.apiToken, nil
	}
	return k.adminToken(ctx)
}

// adminToken runs Kanidm's 3-step idm_admin password auth and returns the bearer
// token carried in the final step's state.success.
func (k *kanidmProvider) adminToken(ctx context.Context) (string, error) {
	sid, _, err := k.authStep(ctx, "", map[string]any{"step": map[string]any{"init": "idm_admin"}})
	if err != nil {
		return "", err
	}
	if sid == "" {
		return "", fmt.Errorf("no x-kanidm-auth-session-id returned by init")
	}
	if _, _, err := k.authStep(ctx, sid, map[string]any{"step": map[string]any{"begin": "password"}}); err != nil {
		return "", err
	}
	_, rb, err := k.authStep(ctx, sid, map[string]any{"step": map[string]any{"cred": map[string]any{"password": k.adminPW}}})
	if err != nil {
		return "", err
	}
	var out struct {
		State struct {
			Success string `json:"success"`
		} `json:"state"`
	}
	if err := json.Unmarshal([]byte(rb), &out); err != nil || out.State.Success == "" {
		return "", fmt.Errorf("auth did not yield a token: %s", snippet([]byte(rb)))
	}
	return out.State.Success, nil
}

// authStep posts one step of the Kanidm auth flow, threading the auth session id
// header. Returns the (possibly refreshed) session id and the response body.
func (k *kanidmProvider) authStep(ctx context.Context, sid string, payload any) (string, string, error) {
	b, _ := json.Marshal(payload)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, k.base+"/v1/auth", strings.NewReader(string(b)))
	if err != nil {
		return sid, "", err
	}
	req.Header.Set("Content-Type", "application/json")
	if sid != "" {
		req.Header.Set("x-kanidm-auth-session-id", sid)
	}
	resp, err := k.hc.Do(req)
	if err != nil {
		return sid, "", err
	}
	defer resp.Body.Close()
	rb, _ := io.ReadAll(resp.Body)
	newSid := resp.Header.Get("x-kanidm-auth-session-id")
	if newSid == "" {
		newSid = sid
	}
	if resp.StatusCode >= 400 {
		return newSid, string(rb), fmt.Errorf("auth step HTTP %d: %s", resp.StatusCode, snippet(rb))
	}
	return newSid, string(rb), nil
}

// call makes one authenticated JSON request and returns (status, body).
func (k *kanidmProvider) call(ctx context.Context, method, path, token string, body any) (int, string, error) {
	var r io.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		r = strings.NewReader(string(b))
	}
	req, err := http.NewRequestWithContext(ctx, method, k.base+path, r)
	if err != nil {
		return 0, "", err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+token)
	resp, err := k.hc.Do(req)
	if err != nil {
		return 0, "", err
	}
	defer resp.Body.Close()
	rb, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(rb), nil
}

// writeCollabOut records the provisioned collab account + one-time setup link to
// ops/operators/out/<handle>-collab.md, mirroring the fabric bundle-md pattern
// (readBundlePath). The welcome-package flow / operator picks this up to send the
// person their collab username + setup link. The link is credential-bearing, so
// the file is mode 0600 and the link is never logged.
func writeCollabOut(repoRoot, handle, provider, account, setupURL, signIn string) error {
	dir := filepath.Join(repoRoot, "ops", "operators", "out")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	md := fmt.Sprintf(`# Collaboration environment — %s

- provider: %s
- username: %s   (they sign in with THIS)
- setup link (valid 24h, single-use — they set a passkey, or a password + TOTP; MFA is required):

  %s

After setup, sign in at %s — single sign-on across Outline, Files, and Chat.
`, handle, provider, account, setupURL, signIn)
	return os.WriteFile(filepath.Join(dir, handle+"-collab.md"), []byte(md), 0o600)
}
