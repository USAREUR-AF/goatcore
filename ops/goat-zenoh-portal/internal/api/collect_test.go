package api_test

// Mint-on-collect (docs/design/fabric-credential-collection.md): a person's
// fabric credential is minted when they collect it, not when an operator
// onboards them. These tests pin the properties the design rests on — the
// credential works the moment it is handed over, it is handed over exactly
// once, the collector never names their own slot, and neither a credential
// already collected nor one already minted at onboard is silently replaced.

import (
	"net/http"
	"path/filepath"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

const collectorIdentity = "9c1d2e3f-4a5b-4c6d-8e9f-0a1b2c3d4e5f"

// seedUncollectedSlot builds the state a deferred onboard leaves behind: a slot
// allocated to a handle, linked to a person, with an org row carrying the alias
// and approval — and NO credential. Built directly through the registry so these
// tests exercise the collection path against that state without depending on the
// onboard endpoint; TestCollect_DeferredOnboardThenCollect covers the real
// `defer_credential` onboard producing the same shape.
func seedUncollectedSlot(t *testing.T, reg *registry.Registry, handle, identityID string) string {
	t.Helper()
	slot, err := reg.AllocateSlot(t.Context(), handle)
	if err != nil {
		t.Fatalf("AllocateSlot(%q): %v", handle, err)
	}
	if err := reg.StampSlotIdentity(t.Context(), slot.SlotID, identityID); err != nil {
		t.Fatalf("StampSlotIdentity: %v", err)
	}
	now := time.Now().Unix()
	approver := "operator"
	if err := reg.CreateVendor(t.Context(), &registry.Vendor{
		Prefix:      slot.SlotID,
		DisplayName: handle,
		ApprovedAt:  &now,
		Approver:    &approver,
	}); err != nil {
		t.Fatalf("CreateVendor: %v", err)
	}
	return slot.SlotID
}

// The credential must be usable the instant it is shown — the whole point of
// coupling the cert and the api_key at collection is that the person walks away
// with a working identity, not a half-issued one they must ask an operator to
// finish.
func TestCollect_MintsAWorkingCredentialForTheSlotOwner(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}
	anchor := seedUncollectedSlot(t, reg, "fra-land-bde", collectorIdentity)

	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)

	if got, _ := body["slot_id"].(string); got != anchor {
		t.Errorf("slot_id = %q, want the identity's own anchor %q", got, anchor)
	}
	if got, _ := body["system_handle"].(string); got != "fra-land-bde" {
		t.Errorf("system_handle = %q, want the live alias", got)
	}
	if got, _ := body["identity_id"].(string); got != collectorIdentity {
		t.Errorf("identity_id = %q, want %q", got, collectorIdentity)
	}
	for _, k := range []string{"api_key", "bundle_url", "bundle_token", "uri_san"} {
		if v, _ := body[k].(string); v == "" {
			t.Errorf("%s missing from the collection response", k)
		}
	}
	if got, _ := body["collected_at"].(float64); got <= 0 {
		t.Errorf("collected_at = %v, want the collection timestamp", body["collected_at"])
	}

	// The handed-over key authenticates as this anchor. Without this the
	// response is just a plausible-looking string.
	apiKey, _ := body["api_key"].(string)
	who := doJSON(t, c, "GET", ts.URL+"/api/whoami", apiKey, nil)
	if who.StatusCode != http.StatusOK {
		t.Fatalf("whoami with the collected key: status %d body=%s", who.StatusCode, mustReadBody(t, who))
	}
	if got, _ := mustReadJSON(t, who)["vendor"].(string); got != anchor {
		t.Errorf("collected key resolves to vendor %q, want %q", got, anchor)
	}
}

// Shown once, and the refusal has to name the way out. A silent re-mint would
// invalidate the copy the person already holds — the rotation trap this design
// closes.
func TestCollect_SecondAttemptRefusedAndFirstKeyStillWorks(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}
	anchor := seedUncollectedSlot(t, reg, "isr-recon-det", collectorIdentity)

	first := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if first.StatusCode != http.StatusCreated {
		t.Fatalf("first collect: status %d body=%s", first.StatusCode, mustReadBody(t, first))
	}
	apiKey, _ := mustReadJSON(t, first)["api_key"].(string)

	second := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if second.StatusCode != http.StatusConflict {
		t.Fatalf("second collect: status %d, want 409 (body=%s)", second.StatusCode, mustReadBody(t, second))
	}

	// The first credential is untouched by the refused retry.
	who := doJSON(t, c, "GET", ts.URL+"/api/whoami", apiKey, nil)
	if who.StatusCode != http.StatusOK {
		t.Fatalf("first key after refused retry: status %d", who.StatusCode)
	}
	if got, _ := mustReadJSON(t, who)["vendor"].(string); got != anchor {
		t.Errorf("first key now resolves to %q, want %q — the retry invalidated it", got, anchor)
	}
}

// Every slot onboarded before this change has a credential minted FOR it and
// delivered out-of-band. Collecting on top of one would invalidate a copy the
// person never asked to replace, so it is refused rather than served.
func TestCollect_RefusesWhenOnboardAlreadyMinted(t *testing.T) {
	dir := t.TempDir()
	ts, _ := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	onboard := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix":      "alpha-fires-bn",
		"identity_id": collectorIdentity,
	})
	if onboard.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status %d body=%s", onboard.StatusCode, mustReadBody(t, onboard))
	}
	onboardKey, _ := mustReadJSON(t, onboard)["api_key"].(string)

	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusConflict {
		t.Fatalf("collect over an onboard-minted slot: status %d, want 409 (body=%s)",
			resp.StatusCode, mustReadBody(t, resp))
	}
	if code := errCode(t, resp); code != "already_minted" {
		t.Errorf("error code = %q, want already_minted", code)
	}

	// The credential the person already holds still authenticates.
	who := doJSON(t, c, "GET", ts.URL+"/api/whoami", onboardKey, nil)
	if who.StatusCode != http.StatusOK {
		t.Errorf("onboard-minted key after refused collection: status %d — the refusal was not clean", who.StatusCode)
	}
}

// The collector names WHO they are; the portal decides WHAT is theirs. There is
// no slot field to steer the mint with, and a request that invents one is
// refused outright rather than quietly ignored — so an attempt to collect
// against someone else's anchor fails loudly at the door.
func TestCollect_CallerCannotNameSomeoneElsesSlot(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}
	const otherIdentity = "11112222-3333-4444-5555-666677778888"
	mine := seedUncollectedSlot(t, reg, "fra-land-bde", collectorIdentity)
	theirs := seedUncollectedSlot(t, reg, "isr-recon-det", otherIdentity)

	steered := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key", map[string]any{
		"identity_id": collectorIdentity,
		"slot_id":     theirs,
	})
	if steered.StatusCode != http.StatusBadRequest {
		t.Fatalf("request naming a slot: status %d, want 400 (body=%s)",
			steered.StatusCode, mustReadBody(t, steered))
	}

	// The honest request mints the caller's own slot, resolved from the identity.
	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key", map[string]any{
		"identity_id": collectorIdentity,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got, _ := mustReadJSON(t, resp)["slot_id"].(string); got != mine {
		t.Fatalf("minted against %q, want the caller's own slot %q", got, mine)
	}

	// The other person's slot is untouched and still collectable by them.
	other, err := reg.SlotForIdentity(t.Context(), otherIdentity)
	if err != nil {
		t.Fatalf("SlotForIdentity(other): %v", err)
	}
	if other.CollectedAt != nil || other.CertMintedAt != nil {
		t.Errorf("other slot %s was minted/collected by someone else's request", theirs)
	}
}

// Nothing to collect must read as nothing to collect. Minting a fresh slot for
// an unrecognised identity would hand fabric access to whoever asked.
func TestCollect_UnknownAndMalformedIdentitiesRefused(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusNotFound {
		t.Errorf("unknown identity: status %d, want 404 (body=%s)", resp.StatusCode, mustReadBody(t, resp))
	}
	for _, bad := range []string{"", "dana", "dana@example.test", "not-a-uuid"} {
		resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
			map[string]any{"identity_id": bad})
		if resp.StatusCode != http.StatusBadRequest {
			t.Errorf("identity_id=%q: status %d, want 400", bad, resp.StatusCode)
		}
	}
	// No slot was consumed by any refusal.
	if _, assigned, err := reg.SlotPoolCounts(t.Context()); err != nil || assigned != 0 {
		t.Errorf("SlotPoolCounts assigned = (%d,%v), want (0,nil) — a refused collection burned a slot", assigned, err)
	}
}

// Two live slots under one identity is broken state, and picking one of them is
// a coin-flip over which anchor a person's data lands under. Fail closed.
func TestCollect_DuplicateLinkageRefusedRatherThanGuessed(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}
	seedUncollectedSlot(t, reg, "fra-land-bde", collectorIdentity)
	seedUncollectedSlot(t, reg, "isr-recon-det", collectorIdentity)

	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusConflict {
		t.Fatalf("duplicate linkage: status %d, want 409 (body=%s)", resp.StatusCode, mustReadBody(t, resp))
	}
	if code := errCode(t, resp); code != "ambiguous_identity" {
		t.Errorf("error code = %q, want ambiguous_identity", code)
	}
}

// The console's operator bearer is the trust boundary: it is what lets the
// caller assert who the authenticated human is. An unauthenticated caller — or
// an ordinary org key — must not be able to mint for an arbitrary identity.
func TestCollect_RequiresOperatorBearer(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}
	seedUncollectedSlot(t, reg, "fra-land-bde", collectorIdentity)

	// No credential at all.
	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("no bearer: status %d, want 401", resp.StatusCode)
	}

	// A legitimate org key, which is not an operator key.
	onboard := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "alpha-fires-bn"})
	if onboard.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status %d", onboard.StatusCode)
	}
	orgKey, _ := mustReadJSON(t, onboard)["api_key"].(string)
	resp = doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", orgKey,
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusForbidden {
		t.Errorf("org key: status %d, want 403", resp.StatusCode)
	}

	// And the slot is still uncollected after both refusals.
	slot, err := reg.SlotForIdentity(t.Context(), collectorIdentity)
	if err != nil {
		t.Fatalf("SlotForIdentity: %v", err)
	}
	if slot.CollectedAt != nil {
		t.Error("slot was collected by an unauthorized caller")
	}
}

// The whole arc, in one test: onboarding defers the credential, and the person
// then collects it. This is the property steps 1-3 exist to produce, and it is
// the only test that would catch the two halves being built against different
// assumptions about what onboarding leaves behind.
func TestCollect_DeferredOnboardThenCollect(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	onboard := doJSON(t, c, "POST", ts.URL+"/api/orgs", "dev-operator-key", map[string]any{
		"prefix":           "fra-land-bde",
		"identity_id":      collectorIdentity,
		"defer_credential": true,
	})
	if onboard.StatusCode != http.StatusCreated {
		t.Fatalf("deferred onboard: status %d body=%s", onboard.StatusCode, mustReadBody(t, onboard))
	}
	ob := mustReadJSON(t, onboard)
	anchor, _ := ob["slot_id"].(string)
	if anchor == "" {
		t.Fatalf("no slot_id from the deferred onboard: %#v", ob)
	}
	// Nothing credential-bearing came back — that is the point.
	for _, k := range []string{"api_key", "bundle_url", "bundle_token"} {
		if v, _ := ob[k].(string); v != "" {
			t.Errorf("deferred onboard returned %s=%q; it must mint nothing", k, v)
		}
	}
	slot, err := reg.SlotForIdentity(t.Context(), collectorIdentity)
	if err != nil {
		t.Fatalf("SlotForIdentity after deferred onboard: %v", err)
	}
	if slot.CertMintedAt != nil {
		t.Error("deferred onboard minted a cert")
	}

	// Now the person collects, and gets a credential that actually works.
	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect after deferred onboard: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	if got, _ := body["slot_id"].(string); got != anchor {
		t.Errorf("collected against %q, want the slot onboarding allocated %q", got, anchor)
	}
	apiKey, _ := body["api_key"].(string)
	who := doJSON(t, c, "GET", ts.URL+"/api/whoami", apiKey, nil)
	if who.StatusCode != http.StatusOK {
		t.Fatalf("collected key does not authenticate: status %d", who.StatusCode)
	}
	if got, _ := mustReadJSON(t, who)["vendor"].(string); got != anchor {
		t.Errorf("collected key resolves to %q, want %q", got, anchor)
	}
}

// A deferred identity authenticates to NOTHING until it is collected — the org
// row carries an empty hash, so an uncollected slot cannot be a live principal.
func TestCollect_DeferredIdentityHoldsNoLiveCredential(t *testing.T) {
	dir := t.TempDir()
	ts, _ := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs", "dev-operator-key", map[string]any{
		"prefix": "isr-recon-det", "identity_id": collectorIdentity, "defer_credential": true,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("deferred onboard: status %d", resp.StatusCode)
	}
	// The empty string must not authenticate as this (or any) principal.
	if w := doJSON(t, c, "GET", ts.URL+"/api/whoami", "", nil); w.StatusCode != http.StatusUnauthorized {
		t.Errorf("empty bearer against a deferred identity = %d, want 401", w.StatusCode)
	}
}

// A RECYCLED slot must not carry the prior tenant's credential stamps: a stale
// cert_minted_at refuses a fresh deferred onboard with `already_minted`,
// permanently, telling the new person to check a welcome email that never
// carried a credential. Fresh-pool tests cannot see it; only a slot with history
// can.
func TestCollect_RecycledSlotDoesNotInheritThePriorTenantsCredential(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}
	const firstIdentity = "aaaa1111-2222-4333-8444-555566667777"

	// Tenant one: onboard normally (mints a cert) and then offboard.
	first := doJSON(t, c, "POST", ts.URL+"/api/orgs", "dev-operator-key", map[string]any{
		"prefix": "fra-land-bde", "identity_id": firstIdentity,
	})
	if first.StatusCode != http.StatusCreated {
		t.Fatalf("first onboard: status %d body=%s", first.StatusCode, mustReadBody(t, first))
	}
	anchor, _ := mustReadJSON(t, first)["slot_id"].(string)
	slot, err := reg.SlotForIdentity(t.Context(), firstIdentity)
	if err != nil || slot.CertMintedAt == nil {
		t.Fatalf("first tenant should hold a minted cert: %v %+v", err, slot)
	}
	if _, err := reg.RevokeAssignedSlot(t.Context(), anchor); err != nil {
		t.Fatalf("RevokeAssignedSlot: %v", err)
	}
	if err := reg.DeleteVendor(t.Context(), anchor); err != nil {
		t.Fatalf("DeleteVendor: %v", err)
	}

	// Tenant two claims the SAME slot with the credential deferred.
	second := doJSON(t, c, "POST", ts.URL+"/api/orgs", "dev-operator-key", map[string]any{
		"prefix": "isr-recon-det", "identity_id": collectorIdentity, "defer_credential": true,
	})
	if second.StatusCode != http.StatusCreated {
		t.Fatalf("deferred re-onboard: status %d body=%s", second.StatusCode, mustReadBody(t, second))
	}
	reclaimed, _ := mustReadJSON(t, second)["slot_id"].(string)
	if reclaimed != anchor {
		t.Skipf("pool handed out a different slot (%s vs %s) — recycling not exercised", reclaimed, anchor)
	}

	// The prior tenant's stamps must be gone, or collection is refused forever.
	row, err := reg.SlotForIdentity(t.Context(), collectorIdentity)
	if err != nil {
		t.Fatalf("SlotForIdentity after re-claim: %v", err)
	}
	if row.CertMintedAt != nil {
		t.Errorf("re-claimed slot inherited cert_minted_at=%d from the prior tenant", *row.CertMintedAt)
	}
	if row.BundleToken != nil || row.BundlePath != nil {
		t.Error("re-claimed slot inherited the prior tenant's bundle pointer")
	}
	if row.CollectedAt != nil {
		t.Error("re-claimed slot inherited the prior tenant's collected_at")
	}

	// And the new owner can actually collect.
	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs/collect", "dev-operator-key",
		map[string]any{"identity_id": collectorIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect on a recycled slot: status %d body=%s — this is the live GALAXY failure",
			resp.StatusCode, mustReadBody(t, resp))
	}
	apiKey, _ := mustReadJSON(t, resp)["api_key"].(string)
	who := doJSON(t, c, "GET", ts.URL+"/api/whoami", apiKey, nil)
	if who.StatusCode != http.StatusOK {
		t.Errorf("credential collected from a recycled slot does not authenticate: %d", who.StatusCode)
	}
}
