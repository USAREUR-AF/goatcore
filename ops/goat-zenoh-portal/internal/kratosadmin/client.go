// Package kratosadmin is a minimal read-only Kratos admin client the
// portal uses to populate the per-vendor identity roster on the
// /admin/vendor-activity.html page. The site-Kratos admin port has no
// bearer auth (network-segmentation gate per id-admin-console.md
// §Architecture), so an empty bearer is OK on the trial fabric. Total
// scope: list-identities + extract traits.vendor.short_tag. We deliberately
// don't share the id-admin-console's kratos client package since the two
// services are separate Go modules; copying ~50 lines keeps the dependency
// surface narrow.
package kratosadmin

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"time"
)

// Client speaks to a single Kratos admin endpoint.
type Client struct {
	AdminURL string
	HTTP     *http.Client
}

// New returns a Client backed by a short-timeout http.Client. AdminURL
// must not include a trailing slash.
func New(adminURL string) *Client {
	return &Client{
		AdminURL: adminURL,
		HTTP:     &http.Client{Timeout: 5 * time.Second},
	}
}

// Identity mirrors the bits of the Kratos identity payload we care about
// for the vendor-activity roster. Full payload is bigger; we only
// deserialize what we render.
type Identity struct {
	ID     string                 `json:"id"`
	State  string                 `json:"state"`
	Traits map[string]interface{} `json:"traits"`
	// MetadataPublic holds the authorization fields (org, aff, groups) that moved
	// off self-editable traits per F-572 / ADR 1188. Kratos returns it on the
	// admin-API identity; the authorization accessors read only it.
	MetadataPublic map[string]interface{} `json:"metadata_public"`
}

// authz returns an authorization field from admin-managed metadata_public
// only. Fail-closed (GHSA-9v3v): self-editable traits are never consulted,
// even for a not-yet-backfilled identity, so a user cannot add an org or aff
// to their own traits and have it read here.
func (i Identity) authz(key string) interface{} {
	return i.MetadataPublic[key]
}

// Email returns traits.email if present.
func (i Identity) Email() string {
	if v, ok := i.Traits["email"].(string); ok {
		return v
	}
	return ""
}

// Name returns traits.name (string-shape; the operator-onboard
// orchestrator always sets a flat string per the site Kratos schema).
func (i Identity) Name() string {
	if v, ok := i.Traits["name"].(string); ok {
		return v
	}
	return ""
}

// VendorShortTag returns the identity's org short_tag — traits.org.short_tag
// (the ADR 1052 Phase 4 successor), falling back to the legacy
// traits.vendor.short_tag for identities minted before the trait migration /
// not yet backfilled. Org is preferred when both are present.
func (i Identity) VendorShortTag() string {
	if v, ok := i.authz("org").(map[string]interface{}); ok {
		if s, _ := v["short_tag"].(string); s != "" {
			return s
		}
	}
	if v, ok := i.authz("vendor").(map[string]interface{}); ok {
		s, _ := v["short_tag"].(string)
		return s
	}
	return ""
}

// Aff returns traits.aff ({id, kind, nation?}) if present — the identity-
// grammar Phase D affiliation claim (ADR 1043), written at mint from the
// manifest's affiliation block. All-empty when the identity predates Phase D
// (the trait is optional in the site schema).
func (i Identity) Aff() (id, kind, nation string) {
	a, ok := i.authz("aff").(map[string]interface{})
	if !ok {
		return "", "", ""
	}
	id, _ = a["id"].(string)
	kind, _ = a["kind"].(string)
	nation, _ = a["nation"].(string)
	return id, kind, nation
}

// ErrNotFound is returned by GetIdentity when Kratos has no identity with the
// given id (a 404), so callers can fail closed on an unknown principal without
// string-matching the error.
var ErrNotFound = fmt.Errorf("kratosadmin: identity not found")

// GetIdentity fetches one identity by its stable Kratos id (the identity UUID —
// the persistent principal key per ADR 1051/1069). A 404 returns ErrNotFound so
// the data-roles resolver fails closed on an unknown UUID.
func (c *Client) GetIdentity(ctx context.Context, id string) (Identity, error) {
	var out Identity
	u := fmt.Sprintf("%s/admin/identities/%s", c.AdminURL, id)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return out, fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Accept", "application/json")
	resp, err := c.HTTP.Do(req)
	if err != nil {
		return out, fmt.Errorf("kratos admin %s: %w", u, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		return out, ErrNotFound
	}
	if resp.StatusCode >= 400 {
		return out, fmt.Errorf("kratos admin %s returned %d", u, resp.StatusCode)
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return out, fmt.Errorf("decode kratos response: %w", err)
	}
	return out, nil
}

// ListIdentities fetches every identity (paginated). The trial fabric
// has dozens; we cap at safetyCap to avoid runaway memory if Kratos
// returns more than expected.
func (c *Client) ListIdentities(ctx context.Context) ([]Identity, error) {
	const safetyCap = 5000
	const pageSize = 250
	var out []Identity
	page := 0
	for {
		u := fmt.Sprintf("%s/admin/identities?page=%d&per_page=%d", c.AdminURL, page, pageSize)
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
		if err != nil {
			return nil, fmt.Errorf("build request: %w", err)
		}
		req.Header.Set("Accept", "application/json")
		resp, err := c.HTTP.Do(req)
		if err != nil {
			return nil, fmt.Errorf("kratos admin %s: %w", u, err)
		}
		if resp.StatusCode >= 400 {
			resp.Body.Close()
			return nil, fmt.Errorf("kratos admin %s returned %d", u, resp.StatusCode)
		}
		var batch []Identity
		if err := json.NewDecoder(resp.Body).Decode(&batch); err != nil {
			resp.Body.Close()
			return nil, fmt.Errorf("decode kratos response: %w", err)
		}
		resp.Body.Close()
		out = append(out, batch...)
		if len(batch) < pageSize || len(out) >= safetyCap {
			break
		}
		page++
	}
	return out, nil
}

// IdentitiesByVendor groups identities by their traits.vendor.short_tag.
// Identities without a vendor tag are dropped (they're operators or
// pre-83C.4 identities). The bucket map's keys match vendor.prefix in
// the portal registry, which is the natural join key on the activity page.
func IdentitiesByVendor(ids []Identity) map[string][]Identity {
	out := make(map[string][]Identity)
	for _, i := range ids {
		tag := i.VendorShortTag()
		if tag == "" {
			continue
		}
		out[tag] = append(out[tag], i)
	}
	return out
}
