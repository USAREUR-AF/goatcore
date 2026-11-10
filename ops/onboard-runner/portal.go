package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

// portalClient is a thin operator-tier client for the onboard-runner contract
// (goat-zenoh-portal): read the approved queue, atomically claim a proposal, and
// write back the terminal result. It holds only a bearer operator key — the mint
// credentials (NetBird token / SSH / portal operator key for the cert mint) live
// in the process env and are handed to `goat`, not to this client.
type portalClient struct {
	base string // e.g. https://portal.galaxy.example
	key  string // operator-tier bearer key
	http *http.Client
}

func newPortalClient(base, key string, timeout time.Duration) *portalClient {
	return newPortalClientWithHTTP(base, key, &http.Client{Timeout: timeout})
}

// newPortalClientWithHTTP is newPortalClient with a caller-supplied *http.Client
// — used to inject the SPIFFE mTLS transport for the onboarding-contract leg.
func newPortalClientWithHTTP(base, key string, hc *http.Client) *portalClient {
	return &portalClient{
		base: strings.TrimRight(base, "/"),
		key:  key,
		http: hc,
	}
}

// proposal is the subset of the portal's onboard_proposals row the runner needs.
// SetupKey/BundlePath are populated only on already-onboarded rows (the dedup
// lookup reads them off the original so a duplicate is finalized pointing at the
// same enrollment rather than a keyless onboarded row).
type proposal struct {
	ID           string `json:"id"`
	Env          string `json:"env"`
	Handle       string `json:"handle"`
	Email        string `json:"email"`
	Status       string `json:"status"`
	ManifestJSON string `json:"manifest_json"`
	SetupKey     string `json:"setup_key,omitempty"`
	BundlePath   string `json:"bundle_path,omitempty"`
	// OffboardRequestedBy is the console admin whose step-up-verified click
	// queued this offboard (offboard-requested rows only) — the principal the
	// revoke's audit event attributes the approval to (goat#372).
	OffboardRequestedBy string `json:"offboard_requested_by,omitempty"`
}

func (c *portalClient) do(ctx context.Context, method, path string, body io.Reader) (int, []byte, error) {
	req, err := http.NewRequestWithContext(ctx, method, c.base+path, body)
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.key)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	return resp.StatusCode, b, nil
}

// approved returns the proposals currently in the approved state (the runner's
// work queue).
func (c *portalClient) approved(ctx context.Context) ([]proposal, error) {
	code, b, err := c.do(ctx, http.MethodGet, "/api/onboard/proposals?status=approved", nil)
	if err != nil {
		return nil, err
	}
	if code != http.StatusOK {
		return nil, fmt.Errorf("list approved: HTTP %d: %s", code, snippet(b))
	}
	var ps []proposal
	if err := json.Unmarshal(b, &ps); err != nil {
		return nil, fmt.Errorf("decode approved list: %w", err)
	}
	return ps, nil
}

// listByStatus returns proposals in a given status — used for the email-dedup
// check against already-onboarded proposals.
func (c *portalClient) listByStatus(ctx context.Context, status string) ([]proposal, error) {
	code, b, err := c.do(ctx, http.MethodGet, "/api/onboard/proposals?status="+status, nil)
	if err != nil {
		return nil, err
	}
	if code != http.StatusOK {
		return nil, fmt.Errorf("list %s: HTTP %d: %s", status, code, snippet(b))
	}
	var ps []proposal
	if err := json.Unmarshal(b, &ps); err != nil {
		return nil, err
	}
	return ps, nil
}

// claim atomically flips approved → onboarding. Returns (true,nil) on a win,
// (false,nil) on a 409 (another runner won, or no longer approved), or an error.
func (c *portalClient) claim(ctx context.Context, id string) (bool, error) {
	code, b, err := c.do(ctx, http.MethodPost, "/api/onboard/proposals/"+id+"/claim", nil)
	if err != nil {
		return false, err
	}
	switch code {
	case http.StatusOK:
		return true, nil
	case http.StatusConflict, http.StatusNotFound:
		return false, nil
	default:
		return false, fmt.Errorf("claim %s: HTTP %d: %s", id, code, snippet(b))
	}
}

// offboardClaim atomically flips offboard-requested → offboarding. Returns
// (true,nil) on a win, (false,nil) on a 409 (another runner won, or the row
// moved on), or an error — the exact contract of claim, on the offboard queue.
func (c *portalClient) offboardClaim(ctx context.Context, id string) (bool, error) {
	code, b, err := c.do(ctx, http.MethodPost, "/api/onboard/proposals/"+id+"/offboard-claim", nil)
	if err != nil {
		return false, err
	}
	switch code {
	case http.StatusOK:
		return true, nil
	case http.StatusConflict:
		if strings.Contains(string(b), `"reissue_running"`) {
			return false, fmt.Errorf("offboarding waits for running reissue: %s", snippet(b))
		}
		return false, nil
	case http.StatusNotFound:
		return false, nil
	default:
		return false, fmt.Errorf("offboard-claim %s: HTTP %d: %s", id, code, snippet(b))
	}
}

// resultBody is the writeback payload for a claimed proposal's terminal outcome.
// Status + error are always sent; the rest are onboarded-only and omitted when
// empty so a partial writeback never blanks a console-side value. The collab
// fields carry the collaboration-environment account + its one-time setup link
// so the console's welcome email includes collab access (the link is
// credential-bearing + one-time — the console uses it for the email but does not
// persist it; only the account + activation flag are recorded).
type resultBody struct {
	Status       string `json:"status"`
	Error        string `json:"error"`
	SetupKey     string `json:"setup_key,omitempty"`
	HandsetKey   string `json:"handset_key,omitempty"`
	PortalAPIKey string `json:"portal_api_key,omitempty"`
	BundlePath   string `json:"bundle_path,omitempty"`
	// SlotAnchor is the mint's write anchor (= slot_id = cert CN), carried
	// EXPLICITLY so the console can record the person→slot linkage without
	// re-deriving it by regexing BundlePath. That derivation breaks the moment a
	// slot legitimately has no bundle — which is exactly what mint-on-collect
	// produces (docs/design/fabric-credential-collection.md) — and silently, since
	// "no bundle" currently reads as "no fabric leg, nothing to link".
	SlotAnchor string `json:"slot_anchor,omitempty"`
	// CredentialDeferred says the mint DELIBERATELY issued no fabric credential
	// (mint-on-collect), so the welcome email can point the person at the
	// collection page. It is stated rather than inferred from an empty
	// PortalAPIKey: that is ALSO what a failed scrape looks like, and telling
	// someone whose key is sitting in their bundle to go and collect one would
	// send them to a page that refuses them (409 already_minted).
	CredentialDeferred bool   `json:"credential_deferred,omitempty"`
	CollabAccount      string `json:"collab_account,omitempty"`
	CollabSetupLink    string `json:"collab_setup_link,omitempty"`
	CollabActivated    bool   `json:"collab_activated,omitempty"`
}

// result writes back the terminal outcome (onboarded|failed) of a claimed
// (onboarding) proposal.
func (c *portalClient) result(ctx context.Context, id, status, errMsg string) error {
	return c.postResult(ctx, id, resultBody{Status: status, Error: errMsg})
}

// resultWithKey is result plus the freshly minted NetBird setup key (onboarded
// only); the console persists it for the welcome package.
func (c *portalClient) resultWithKey(ctx context.Context, id, status, errMsg, setupKey string) error {
	return c.postResult(ctx, id, resultBody{Status: status, Error: errMsg, SetupKey: setupKey})
}

// resultWithKeyBundle is resultWithKey plus the fabric bundle's mesh download
// path (onboarded only) — the console prepends the mesh portal base and puts the
// download link in the welcome email. Both extras optional.
func (c *portalClient) resultWithKeyBundle(ctx context.Context, id, status, errMsg, setupKey, bundlePath string) error {
	return c.postResult(ctx, id, resultBody{Status: status, Error: errMsg, SetupKey: setupKey, BundlePath: bundlePath})
}

// postResult POSTs a terminal onboarding result. The onboarded path builds a
// resultBody directly so it can also carry the collab account + one-time setup
// link (docs/design/collab-onboarding-provisioner.md §welcome-email).
func (c *portalClient) postResult(ctx context.Context, id string, rb resultBody) error {
	body, _ := json.Marshal(rb)
	code, b, err := c.do(ctx, http.MethodPost, "/api/onboard/proposals/"+id+"/result", bytes.NewReader(body))
	if err != nil {
		return err
	}
	if code != http.StatusOK {
		return fmt.Errorf("result %s=%s: HTTP %d: %s", id, rb.Status, code, snippet(b))
	}
	return nil
}

func snippet(b []byte) string {
	s := strings.TrimSpace(string(b))
	if len(s) > 300 {
		return s[:300]
	}
	return s
}

// action is one queued follow-up for an already-onboarded proposal (ADR 1175):
// the console asks, this runner claims it exactly once and runs the matching
// goat verb. Handle and Env are the proposal's, joined in by the console so no
// second lookup is needed.
type action struct {
	ID         string `json:"id"`
	ProposalID string `json:"proposal_id"`
	Kind       string `json:"kind"`
	Status     string `json:"status"`
	Handle     string `json:"handle"`
	Env        string `json:"env"`
	// RequestedBy is the console operator who asked; passed to goat so its
	// audit event names them beside the runner that ran the mint.
	RequestedBy string `json:"requested_by,omitempty"`
	ClaimToken  string `json:"claim_token,omitempty"`
	// Params is the JSON object the console attached (reissue_device: uses).
	Params string `json:"params,omitempty"`
}

const (
	// actionReissueHandset revokes the person's live handset keys and mints a
	// fresh one (`goat admin onboard <handle> --native --reissue-handset-key`).
	actionReissueHandset = "reissue_handset"
	// actionReissueDevice mints a fresh device setup key for a person whose key
	// expired or was used up (`... --reissue-device-key --uses N`).
	actionReissueDevice = "reissue_device"
	actionReleaseHold   = "release_hold"
)

// queuedActions returns the follow-up actions waiting for a runner.
func (c *portalClient) queuedActions(ctx context.Context) ([]action, error) {
	code, b, err := c.do(ctx, http.MethodGet, "/api/onboard/actions?status=queued", nil)
	if err != nil {
		return nil, err
	}
	if code == http.StatusNotFound {
		// A console that predates the action queue: nothing to do, not an error.
		return nil, nil
	}
	if code != http.StatusOK {
		return nil, fmt.Errorf("list queued actions: HTTP %d: %s", code, snippet(b))
	}
	var as []action
	if err := json.Unmarshal(b, &as); err != nil {
		return nil, fmt.Errorf("decode queued actions: %w", err)
	}
	return as, nil
}

// claimAction atomically flips queued → claimed and returns the lease token
// required by consoles that support claim leases. claimed is false when
// another runner won or the action already ran.
func (c *portalClient) claimAction(ctx context.Context, id string) (claimToken string, claimed bool, err error) {
	code, b, err := c.do(ctx, http.MethodPost, "/api/onboard/actions/"+id+"/claim", nil)
	if err != nil {
		return "", false, err
	}
	switch code {
	case http.StatusOK:
		var claimed action
		if err := json.Unmarshal(b, &claimed); err != nil {
			return "", false, fmt.Errorf("decode claimed action: %w", err)
		}
		return claimed.ClaimToken, true, nil
	case http.StatusConflict, http.StatusNotFound:
		return "", false, nil
	default:
		return "", false, fmt.Errorf("claim action %s: HTTP %d: %s", id, code, snippet(b))
	}
}

// actionResultBody is the writeback for a claimed action. The key is the one
// thing a reissue produces; the console stores it on the proposal.
type actionResultBody struct {
	Status     string               `json:"status"`
	Error      string               `json:"error,omitempty"`
	ClaimToken string               `json:"claim_token"`
	HandsetKey string               `json:"handset_key,omitempty"`
	SetupKey   string               `json:"setup_key,omitempty"`
	Release    *releaseActionResult `json:"release,omitempty"`
}

func (c *portalClient) actionResult(ctx context.Context, id string, rb actionResultBody) error {
	body, _ := json.Marshal(rb)
	code, b, err := c.do(ctx, http.MethodPost, "/api/onboard/actions/"+id+"/result", bytes.NewReader(body))
	if err != nil {
		return err
	}
	if code != http.StatusOK {
		return fmt.Errorf("action result %s=%s: HTTP %d: %s", id, rb.Status, code, snippet(b))
	}
	return nil
}
