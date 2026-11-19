package kratos

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

type deleteRecordingRoundTripper struct {
	status int
	method string
	path   string
}

func (rt *deleteRecordingRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	rt.method = req.Method
	rt.path = req.URL.EscapedPath()
	w := httptest.NewRecorder()
	w.WriteHeader(rt.status)
	return w.Result(), nil
}

type listIdentitiesRoundTripper struct {
	gotQuery string
}

func (rt *listIdentitiesRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	rt.gotQuery = req.URL.RawQuery
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	if req.URL.Query().Get("page") == "0" {
		_, _ = w.Write([]byte(`[
			{"id":"a","credentials":{"password":{},"totp":{}}},
			{"id":"b","credentials":{"password":{}}}
		]`))
	} else {
		_, _ = w.Write([]byte(`[]`))
	}
	return w.Result(), nil
}

type listSessionsRoundTripper struct{}

func (listSessionsRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	_, _ = w.Write([]byte(`[{"id":"sess-1","active":true,
	  "expires_at":"2026-08-05T12:00:00Z",
	  "authenticated_at":"2026-08-04T12:00:30Z",
	  "authenticator_assurance_level":"aal2",
	  "authentication_methods":[
	    {"method":"password","aal":"aal1","completed_at":"2026-08-04T12:00:00Z"},
	    {"method":"totp","aal":"aal2","completed_at":"2026-08-04T12:00:30Z"}
	  ]
	}]`))
	return w.Result(), nil
}

// ADR 1052 Phase 4 — Org() reads metadata_public.org first, falling back to
// the legacy metadata_public.vendor block (GHSA-9v3v: never traits).
func TestVendor_PrefersOrgFallsBackToVendor(t *testing.T) {
	block := func(tag, name string) map[string]interface{} {
		return map[string]interface{}{"short_tag": tag, "display_name": name}
	}
	cases := []struct {
		name        string
		traits      map[string]interface{}
		wantTag     string
		wantDisplay string
	}{
		{"org only", map[string]interface{}{"org": block("org67x", "org63x")}, "org67x", "org63x"},
		{"vendor only (legacy)", map[string]interface{}{"vendor": block("demo-vendor-pat", "Demo Vendor")}, "demo-vendor-pat", "Demo Vendor"},
		{"both present prefers org", map[string]interface{}{
			"org":    block("new", "New Co"),
			"vendor": block("old", "Old Co"),
		}, "new", "New Co"},
		{"display-only legacy (no short_tag)", map[string]interface{}{
			"vendor": map[string]interface{}{"display_name": "G6 JCND E&I"},
		}, "", "G6 JCND E&I"},
		{"neither", map[string]interface{}{}, "", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			gotTag, gotDisplay := Identity{MetadataPublic: tc.traits}.Org()
			if gotTag != tc.wantTag || gotDisplay != tc.wantDisplay {
				t.Fatalf("Org() = (%q, %q), want (%q, %q)", gotTag, gotDisplay, tc.wantTag, tc.wantDisplay)
			}
		})
	}
}

// F-572 / ADR 1188 — the authorization fields (groups, org, aff) move from
// self-editable traits to admin-managed metadata_public. The accessors read
// ONLY metadata_public (GHSA-9v3v: no traits fallback, even before the
// backfill), so a self-service settings edit — which can only touch traits —
// can never change what the console and the JWT claim see.
func TestAuthzFields_MetadataPublicOnly(t *testing.T) {
	orgBlock := func(tag, name string) map[string]interface{} {
		return map[string]interface{}{"short_tag": tag, "display_name": name}
	}

	t.Run("groups from metadata_public win over a self-edited traits value", func(t *testing.T) {
		id := Identity{
			MetadataPublic: map[string]interface{}{"groups": []interface{}{"PLATFORM-ADMIN"}},
			Traits:         map[string]interface{}{"groups": []interface{}{"OBSERVER"}},
		}
		got := id.Groups()
		if len(got) != 1 || got[0] != "PLATFORM-ADMIN" {
			t.Fatalf("Groups() = %v, want [PLATFORM-ADMIN] (metadata_public wins)", got)
		}
	})
	t.Run("GHSA-9v3v: traits never grant groups, even with metadata_public absent", func(t *testing.T) {
		id := Identity{Traits: map[string]interface{}{"groups": []interface{}{"PLATFORM-ADMIN"}}}
		if got := id.Groups(); len(got) != 0 {
			t.Fatalf("Groups() = %v, want none (fail closed)", got)
		}
		if id.HasGroup("PLATFORM-ADMIN") {
			t.Fatal("HasGroup read a self-editable trait")
		}
		id.MetadataPublic = map[string]interface{}{"org": map[string]interface{}{}}
		if got := id.Groups(); len(got) != 0 {
			t.Fatalf("Groups() with metadata_public but no groups key = %v, want none", got)
		}
	})
	t.Run("GHSA-9v3v: traits never supply org or aff for a not-yet-backfilled identity", func(t *testing.T) {
		id := Identity{Traits: map[string]interface{}{
			"org": orgBlock("self", "Self"),
			"aff": map[string]interface{}{"id": "self", "kind": "partner"},
		}}
		if tag, _ := id.Org(); tag != "" {
			t.Fatalf("Org() tag = %q, want empty", tag)
		}
	})
	t.Run("org prefers metadata_public", func(t *testing.T) {
		id := Identity{
			MetadataPublic: map[string]interface{}{"org": orgBlock("org67x", "org63x")},
			Traits:         map[string]interface{}{"org": orgBlock("old", "Old")},
		}
		if tag, name := id.Org(); tag != "org67x" || name != "org63x" {
			t.Fatalf("Org() = (%q,%q), want (org67x, org63x)", tag, name)
		}
	})
	t.Run("migrated identity ignores a self-added traits org/aff", func(t *testing.T) {
		id := Identity{
			MetadataPublic: map[string]interface{}{"groups": []interface{}{}},
			Traits: map[string]interface{}{
				"org":    orgBlock("self", "Self"),
				"aff":    map[string]interface{}{"id": "self", "kind": "partner"},
				"groups": []interface{}{"PLATFORM-ADMIN"},
			},
		}
		if g := id.Groups(); len(g) != 0 {
			t.Fatalf("Groups() = %v, want [] (seeded metadata groups win)", g)
		}
		if tag, _ := id.Org(); tag != "" {
			t.Fatalf("Org() tag = %q, want empty (no traits fallback once migrated)", tag)
		}
		if aid, _, _ := id.Aff(); aid != "" {
			t.Fatalf("Aff() id = %q, want empty (no traits fallback once migrated)", aid)
		}
	})
	t.Run("aff prefers metadata_public", func(t *testing.T) {
		id := Identity{
			MetadataPublic: map[string]interface{}{"aff": map[string]interface{}{"id": "org63x", "kind": "vendor"}},
			Traits:         map[string]interface{}{"aff": map[string]interface{}{"id": "old", "kind": "organic"}},
		}
		if aid, kind, _ := id.Aff(); aid != "org63x" || kind != "vendor" {
			t.Fatalf("Aff() = (%q,%q,_), want (org63x, vendor)", aid, kind)
		}
	})
}

// F-572 / ADR 1188 — a mint that still builds groups/org/aff into traits must
// have them routed to metadata_public at the CreateIdentity chokepoint, so no
// onboarding path can seed an authorization field where settings could edit it
// (and so create stays valid once those traits leave the schema).
func TestCreateIdentity_RoutesAuthzFieldsToMetadataPublic(t *testing.T) {
	rt := &createCapturingRoundTripper{status: http.StatusCreated, body: `{"id":"x"}`}
	c := NewClient("https://kratos.example.test")
	c.HTTP = &http.Client{Transport: rt}

	_, err := c.CreateIdentity(context.Background(), CreateIdentityRequest{
		Traits: map[string]interface{}{
			"email":  "a@example.test",
			"name":   "A",
			"groups": []interface{}{"IAM-ADMIN"},
			"org":    map[string]interface{}{"short_tag": "org63x", "display_name": "org63x"},
			"aff":    map[string]interface{}{"id": "org63x", "kind": "vendor"},
		},
	})
	if err != nil {
		t.Fatalf("create: %v", err)
	}
	sent := rt.captured
	traits, _ := sent["traits"].(map[string]interface{})
	meta, _ := sent["metadata_public"].(map[string]interface{})
	for _, k := range []string{"groups", "org", "aff"} {
		if _, inTraits := traits[k]; inTraits {
			t.Errorf("%q still in traits; must be routed to metadata_public", k)
		}
		if _, inMeta := meta[k]; !inMeta {
			t.Errorf("%q missing from metadata_public", k)
		}
	}
	if _, ok := traits["email"]; !ok {
		t.Error("email must remain in traits")
	}
}

// F-572 — metadata_public.groups is the migration marker every reader keys on;
// an identity created without it, or with it null, reads its groups from
// self-editable traits. Create must always write it as an array.
func TestCreateIdentity_AlwaysWritesGroupsArray(t *testing.T) {
	cases := map[string]map[string]interface{}{
		"no groups key":     {"email": "a@example.test"},
		"nil []string":      {"email": "a@example.test", "groups": []string(nil)},
		"nil interface":     {"email": "a@example.test", "groups": nil},
		"empty []string":    {"email": "a@example.test", "groups": []string{}},
		"nil []interface{}": {"email": "a@example.test", "groups": []interface{}(nil)},
	}
	for name, traits := range cases {
		t.Run(name, func(t *testing.T) {
			rt := &createCapturingRoundTripper{status: http.StatusCreated, body: `{"id":"x"}`}
			c := NewClient("https://kratos.example.test")
			c.HTTP = &http.Client{Transport: rt}
			if _, err := c.CreateIdentity(context.Background(), CreateIdentityRequest{Traits: traits}); err != nil {
				t.Fatalf("create: %v", err)
			}
			meta, _ := rt.captured["metadata_public"].(map[string]interface{})
			g, ok := meta["groups"].([]interface{})
			if !ok || len(g) != 0 {
				t.Fatalf("metadata_public.groups = %#v, want an empty array", meta["groups"])
			}
			if tr, _ := rt.captured["traits"].(map[string]interface{}); tr["groups"] != nil {
				t.Fatalf("groups left in traits: %#v", tr["groups"])
			}
		})
	}
}

// createCapturingRoundTripper records the POST /admin/identities body.
type createCapturingRoundTripper struct {
	status   int
	body     string
	captured map[string]interface{}
}

func (rt *createCapturingRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	if req.Body != nil {
		b, _ := io.ReadAll(req.Body)
		_ = json.Unmarshal(b, &rt.captured)
	}
	w := httptest.NewRecorder()
	w.WriteHeader(rt.status)
	_, _ = w.WriteString(rt.body)
	return w.Result(), nil
}

// TestDeleteIdentityCredential_URLAndNoOp proves the client hits the right
// per-credential DELETE path and treats a 404 as a benign no-op.
func TestDeleteIdentityCredential_URLAndNoOp(t *testing.T) {
	rt := &deleteRecordingRoundTripper{status: http.StatusNoContent}
	c := NewClient("https://kratos.example.test")
	c.HTTP = &http.Client{Transport: rt}

	if err := c.DeleteIdentityCredential(context.Background(), "id 1", "totp"); err != nil {
		t.Fatalf("delete: %v", err)
	}
	if rt.method != http.MethodDelete {
		t.Fatalf("method = %s, want DELETE", rt.method)
	}
	if want := "/admin/identities/id%201/credentials/totp"; rt.path != want {
		t.Fatalf("path = %q, want %q (both segments PathEscaped)", rt.path, want)
	}

	// 404 → no error (identity simply has no such credential).
	rt.status = http.StatusNotFound
	if err := c.DeleteIdentityCredential(context.Background(), "id-2", "webauthn"); err != nil {
		t.Fatalf("404 should be a no-op, got: %v", err)
	}

	// Any other non-2xx surfaces as an error.
	rt.status = http.StatusInternalServerError
	if err := c.DeleteIdentityCredential(context.Background(), "id-3", "totp"); err == nil {
		t.Fatal("500 should return an error, got nil")
	}
}

func TestHasSecondFactor_Matrix(t *testing.T) {
	// Each case decodes a full identity document so the matrix also exercises
	// Credential.UnmarshalJSON (the decode-time secret-material reduction), not
	// just the map-key logic. The webauthn placeholder (a user_handle, no
	// enrolled keys — Kratos auto-creates it for every identity whose schema
	// declares email a webauthn identifier) must NOT count as a second factor;
	// a webauthn credential with an enrolled key must. Real-Kratos v1.3.1 shapes.
	cases := []struct {
		name  string
		creds string // JSON for the identity credentials map; "" = field absent
		want  bool
	}{
		{"absent", ``, false},
		{"password-only", `{"password":{"type":"password","config":{}}}`, false},
		{"totp", `{"totp":{"type":"totp","config":{"totp_url":"otpauth://totp/x?secret=SEED"}}}`, true},
		{"webauthn-placeholder", `{"password":{},"webauthn":{"type":"webauthn","config":{"user_handle":"rUlXn39cR46yJycQb7ztEQ=="}}}`, false},
		{"webauthn-enrolled", `{"webauthn":{"type":"webauthn","config":{"user_handle":"x","credentials":[{"id":"abc","is_passwordless":false}]}}}`, true},
		{"lookup_secret", `{"lookup_secret":{"type":"lookup_secret","config":{"recovery_codes":[{"code":"aaaa-bbbb"}]}}}`, true},
		{"password+totp", `{"password":{},"totp":{}}`, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			doc := `{"id":"x"}`
			if tc.creds != "" {
				doc = `{"id":"x","credentials":` + tc.creds + `}`
			}
			var i Identity
			if err := json.Unmarshal([]byte(doc), &i); err != nil {
				t.Fatalf("unmarshal: %v", err)
			}
			if got := i.HasSecondFactor(); got != tc.want {
				t.Fatalf("HasSecondFactor() = %v, want %v", got, tc.want)
			}
		})
	}
}

// TestCredentials_SecretMaterialDiscardedAtDecode pins the security boundary:
// Kratos include_credential responses carry real secret material — the
// decrypted TOTP seed (config.totp_url) and the lookup-secret backup codes
// (config.recovery_codes). Credential.UnmarshalJSON must discard all of it at
// decode time so no later Marshal of an Identity (debug logging, a future API
// endpoint) can leak it.
func TestCredentials_SecretMaterialDiscardedAtDecode(t *testing.T) {
	doc := `{"id":"x","credentials":{
		"totp":{"type":"totp","config":{"totp_url":"otpauth://totp/u@example.org?secret=JBSWY3DPEHPK3PXP"}},
		"lookup_secret":{"type":"lookup_secret","config":{"recovery_codes":[{"code":"aaaa-bbbb-cccc"}]}},
		"webauthn":{"type":"webauthn","config":{"user_handle":"USERHANDLE","credentials":[{"id":"pubkeyid"}]}}}}`
	var i Identity
	if err := json.Unmarshal([]byte(doc), &i); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if !i.HasSecondFactor() {
		t.Fatal("identity has enrolled factors — HasSecondFactor should be true")
	}
	out, err := json.Marshal(i)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	for _, leak := range []string{"JBSWY3DPEHPK3PXP", "aaaa-bbbb-cccc", "USERHANDLE", "pubkeyid", "totp_url", "recovery_codes", "user_handle"} {
		if strings.Contains(string(out), leak) {
			t.Fatalf("re-serialized identity leaks %q: %s", leak, out)
		}
	}
}

// TestListIdentities_IncludeCredential proves the list call requests inline
// credential state (no N+1) and parses it into HasSecondFactor.
func TestListIdentities_IncludeCredential(t *testing.T) {
	rt := &listIdentitiesRoundTripper{}
	cli := NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	ids, err := cli.ListIdentities(context.Background())
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	for _, want := range []string{"include_credential=totp", "include_credential=webauthn", "include_credential=lookup_secret"} {
		if !strings.Contains(rt.gotQuery, want) {
			t.Fatalf("query %q missing %q", rt.gotQuery, want)
		}
	}
	if len(ids) != 2 {
		t.Fatalf("got %d identities, want 2", len(ids))
	}
	if !ids[0].HasSecondFactor() {
		t.Fatal("identity a has totp — HasSecondFactor should be true")
	}
	if ids[1].HasSecondFactor() {
		t.Fatal("identity b is password-only — HasSecondFactor should be false")
	}
}

// TestSession_FreshAAL2At covers the per-factor ledger the step-up gate reads
// (#174). Kratos does not guarantee ordering of authentication_methods, and a
// session's own authenticator_assurance_level says only "aal2 was reached at
// some point" — never when. FreshAAL2At must return the NEWEST aal2 entry.
func TestSession_FreshAAL2At(t *testing.T) {
	t0 := time.Date(2026, 8, 4, 12, 0, 0, 0, time.UTC)

	cases := []struct {
		name    string
		methods []AuthenticationMethod
		want    time.Time
		wantOK  bool
	}{
		{
			name:   "no methods at all",
			wantOK: false,
		},
		{
			name: "password only — first factor never satisfies aal2",
			methods: []AuthenticationMethod{
				{Method: "password", AAL: "aal1", CompletedAt: t0},
			},
			wantOK: false,
		},
		{
			name: "password + totp",
			methods: []AuthenticationMethod{
				{Method: "password", AAL: "aal1", CompletedAt: t0},
				{Method: "totp", AAL: "aal2", CompletedAt: t0.Add(30 * time.Second)},
			},
			want: t0.Add(30 * time.Second), wantOK: true,
		},
		{
			name: "picks the NEWEST aal2 even when Kratos returns them out of order",
			methods: []AuthenticationMethod{
				{Method: "totp", AAL: "aal2", CompletedAt: t0.Add(2 * time.Hour)},
				{Method: "password", AAL: "aal1", CompletedAt: t0.Add(3 * time.Hour)},
				{Method: "webauthn", AAL: "aal2", CompletedAt: t0.Add(time.Hour)},
			},
			want: t0.Add(2 * time.Hour), wantOK: true,
		},
		{
			name: "backup code counts as a second factor",
			methods: []AuthenticationMethod{
				{Method: "lookup_secret", AAL: "aal2", CompletedAt: t0},
			},
			want: t0, wantOK: true,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := Session{AuthenticationMethods: tc.methods}.FreshAAL2At()
			if ok != tc.wantOK {
				t.Fatalf("ok = %v, want %v", ok, tc.wantOK)
			}
			if ok && !got.Equal(tc.want) {
				t.Fatalf("at = %v, want %v", got, tc.want)
			}
		})
	}
}

// TestListIdentitySessions_ParsesAuthenticationMethods proves the new field is
// actually decoded from a realistic Kratos payload — a struct-tag typo here
// would silently make every admin look password-only and refuse every gated
// action, which is the exact failure #174 exists to fix.
func TestListIdentitySessions_ParsesAuthenticationMethods(t *testing.T) {
	cli := NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: listSessionsRoundTripper{}}
	sessions, err := cli.ListIdentitySessions(context.Background(), "actor-1")
	if err != nil {
		t.Fatal(err)
	}
	if len(sessions) != 1 {
		t.Fatalf("got %d sessions, want 1", len(sessions))
	}
	if n := len(sessions[0].AuthenticationMethods); n != 2 {
		t.Fatalf("authentication_methods parsed as %d entries, want 2", n)
	}
	at, ok := sessions[0].FreshAAL2At()
	if !ok {
		t.Fatal("aal2 entry not recognized")
	}
	if want := time.Date(2026, 8, 4, 12, 0, 30, 0, time.UTC); !at.Equal(want) {
		t.Fatalf("aal2 at %v, want %v", at, want)
	}
}

// TestListIdentitySessions_PaginatesAndFiltersActive pins the goat#247
// defect-4 fix: the step-up gate must see EVERY active session, so the client
// paginates to exhaustion and asks Kratos for active sessions only. Before
// this, one unpaginated page with unspecified ordering could hide a genuinely
// fresh aal2 session and loop a real verify.
func TestListIdentitySessions_PaginatesAndFiltersActive(t *testing.T) {
	var sawActive, sawPages []string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		sawActive = append(sawActive, r.URL.Query().Get("active"))
		page := r.URL.Query().Get("page")
		sawPages = append(sawPages, page)
		w.Header().Set("Content-Type", "application/json")
		n := 250 // full first page forces a second fetch
		if page == "2" {
			n = 10
		}
		rows := make([]map[string]interface{}, n)
		for i := range rows {
			rows[i] = map[string]interface{}{"id": "sess", "active": true}
		}
		_ = json.NewEncoder(w).Encode(rows)
	}))
	defer srv.Close()

	sessions, err := NewClient(srv.URL).ListIdentitySessions(context.Background(), "actor-1")
	if err != nil {
		t.Fatal(err)
	}
	if len(sessions) != 260 {
		t.Fatalf("got %d sessions, want 260 across two pages", len(sessions))
	}
	if len(sawPages) != 2 || sawPages[0] != "1" || sawPages[1] != "2" {
		t.Fatalf("pages fetched = %v, want [1 2]", sawPages)
	}
	for _, a := range sawActive {
		if a != "true" {
			t.Fatalf("active param = %q, want true on every page", a)
		}
	}
}

// F-475 — an admin-minted recovery code never verifies the address, so the
// console imports it verified at create time. The POST body must carry the
// entry, or every OIDC fence that checks email_verified refuses the identity.
func TestCreateIdentity_ImportsTheEmailVerified(t *testing.T) {
	var got map[string]interface{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/admin/identities" {
			t.Errorf("unexpected %s %s", r.Method, r.URL.Path)
		}
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Errorf("decode body: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"i1","traits":{"email":"a@example.test"},"verifiable_addresses":[{"id":"va1","value":"a@example.test","via":"email","verified":true,"status":"completed"}]}`))
	}))
	defer srv.Close()

	id, err := NewClient(srv.URL).CreateIdentity(context.Background(), CreateIdentityRequest{
		Traits:              map[string]interface{}{"email": "a@example.test"},
		VerifiableAddresses: VerifiedEmail("a@example.test"),
	})
	if err != nil {
		t.Fatalf("CreateIdentity: %v", err)
	}
	if got["schema_id"] != "default" {
		t.Errorf("schema_id = %v, want default", got["schema_id"])
	}
	addrs, _ := got["verifiable_addresses"].([]interface{})
	if len(addrs) != 1 {
		t.Fatalf("verifiable_addresses = %v, want one entry", got["verifiable_addresses"])
	}
	entry := addrs[0].(map[string]interface{})
	if entry["value"] != "a@example.test" || entry["via"] != "email" || entry["verified"] != true || entry["status"] != "completed" {
		t.Errorf("verifiable_addresses[0] = %v", entry)
	}
	if _, present := entry["id"]; present {
		t.Errorf("an empty id must be omitted; Kratos parses it as a UUID: %v", entry)
	}
	if len(id.VerifiableAddresses) != 1 || !id.VerifiableAddresses[0].Verified || id.VerifiableAddresses[0].Status != "completed" {
		t.Errorf("response not decoded: %+v", id.VerifiableAddresses)
	}
}

// A mint that sends no addresses keeps the old wire shape: the field is
// omitted, not sent as null, so Kratos treats it as absent.
func TestCreateIdentity_OmitsAddressesWhenNoneGiven(t *testing.T) {
	var got map[string]interface{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&got)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"i1","traits":{}}`))
	}))
	defer srv.Close()
	if _, err := NewClient(srv.URL).CreateIdentity(context.Background(), CreateIdentityRequest{Traits: map[string]interface{}{}}); err != nil {
		t.Fatalf("CreateIdentity: %v", err)
	}
	if _, present := got["verifiable_addresses"]; present {
		t.Errorf("verifiable_addresses should be omitted, got %v", got["verifiable_addresses"])
	}
}

// Kratos v1.3.1's identity-patch allowlist accepts add/remove/replace, not
// test. The group patch is deliberately one child-path add: Kratos creates a
// missing parent, and an existing parent's unrelated keys are never sent back
// by this client (so they cannot be overwritten by a read/merge/write race).
func TestGroupsPatch_UsesSupportedChildPathAdd(t *testing.T) {
	ops := groupsPatch([]string{"IAM-ADMIN"})
	if len(ops) != 1 {
		t.Fatalf("ops = %v, want exactly one operation", ops)
	}
	if ops[0]["op"] != "add" || ops[0]["path"] != "/metadata_public/groups" {
		t.Fatalf("op = %v, want add on /metadata_public/groups", ops[0])
	}
	if g, _ := ops[0]["value"].([]string); len(g) != 1 || g[0] != "IAM-ADMIN" {
		t.Fatalf("groups = %v, want [IAM-ADMIN]", ops[0]["value"])
	}
}

func newKratosV131Fake(t *testing.T, identity map[string]interface{}, getCalls *int) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.Method == http.MethodGet {
			(*getCalls)++
			_ = json.NewEncoder(w).Encode(identity)
			return
		}
		if r.Method != http.MethodPatch {
			w.WriteHeader(http.StatusMethodNotAllowed)
			return
		}
		var ops []map[string]interface{}
		if err := json.NewDecoder(r.Body).Decode(&ops); err != nil {
			t.Errorf("decode patch: %v", err)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		for _, op := range ops {
			kind, _ := op["op"].(string)
			if kind != "add" && kind != "remove" && kind != "replace" {
				w.WriteHeader(http.StatusBadRequest)
				_, _ = w.Write([]byte(`{"error":"unsupported patch operation"}`))
				return
			}
			if kind != "add" || op["path"] != "/metadata_public/groups" {
				t.Errorf("patch op = %v, want child-path add", op)
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			mp, _ := identity["metadata_public"].(map[string]interface{})
			if mp == nil {
				mp = map[string]interface{}{}
				identity["metadata_public"] = mp
			}
			mp["groups"] = op["value"]
		}
		_ = json.NewEncoder(w).Encode(identity)
	}))
}

// This transport models the operation allowlist and EnsurePathExistsOnAdd
// behavior of Kratos v1.3.1. It rejects the previously emitted test operation,
// creates a missing/null metadata_public parent, and applies only the groups
// child so sibling metadata cannot be clobbered.
func TestPatchGroups_KratosV131PatchContract(t *testing.T) {
	tests := []struct {
		name      string
		setMeta   bool
		meta      interface{}
		wantOther bool
	}{
		{name: "migrated", setMeta: true, wantOther: true, meta: map[string]interface{}{
			"groups": []interface{}{"PLATFORM-ADMIN"},
			"org":    map[string]interface{}{"short_tag": "acme"},
			"aff":    map[string]interface{}{"id": "acme", "kind": "partner"},
			"vendor": map[string]interface{}{"short_tag": "legacy"},
			"other":  "keep-me",
		}},
		{name: "missing parent"},
		{name: "null parent", setMeta: true, meta: nil},
		{name: "null groups", setMeta: true, wantOther: true, meta: map[string]interface{}{"groups": nil, "other": "keep-me"}},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			identity := map[string]interface{}{
				"id": "id-1", "schema_id": "default",
				"traits": map[string]interface{}{"email": "a@example.test"},
			}
			if tc.setMeta {
				identity["metadata_public"] = tc.meta
			}
			getCalls := 0
			srv := newKratosV131Fake(t, identity, &getCalls)
			defer srv.Close()

			updated, err := NewClient(srv.URL).PatchGroups(context.Background(), "id-1", []string{"GRAFANA-READONLY"})
			if err != nil {
				t.Fatalf("PatchGroups: %v", err)
			}
			if getCalls != 0 {
				t.Fatalf("PatchGroups made %d GET request(s), want none", getCalls)
			}
			if got := updated.Groups(); len(got) != 1 || got[0] != "GRAFANA-READONLY" {
				t.Fatalf("stored groups = %v, want [GRAFANA-READONLY]", got)
			}
			if mp := identity["metadata_public"].(map[string]interface{}); tc.wantOther && mp["other"] != "keep-me" {
				t.Fatalf("unrelated metadata changed: %v", mp)
			}
			if tc.name == "migrated" {
				mp := identity["metadata_public"].(map[string]interface{})
				for _, key := range []string{"org", "aff", "vendor", "other"} {
					if mp[key] == nil {
						t.Fatalf("metadata_public.%s was lost: %v", key, mp)
					}
				}
			}
		})
	}
}

func TestKratosV131PatchContract_RejectsTestOperation(t *testing.T) {
	identity := map[string]interface{}{
		"id": "id-1",
		"metadata_public": map[string]interface{}{
			"groups": []interface{}{"PLATFORM-ADMIN"},
			"org":    map[string]interface{}{"short_tag": "acme"},
		},
	}
	getCalls := 0
	srv := newKratosV131Fake(t, identity, &getCalls)
	defer srv.Close()

	body := `[{"op":"test","path":"/metadata_public","value":{"groups":["PLATFORM-ADMIN"],"org":{"short_tag":"acme"}}},{"op":"add","path":"/metadata_public","value":{"groups":["GRAFANA-READONLY"],"org":{"short_tag":"acme"}}}]`
	req, err := http.NewRequestWithContext(context.Background(), http.MethodPatch, srv.URL+"/admin/identities/id-1", strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("PATCH old two-op body: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400", resp.StatusCode)
	}
}

// F-572 — removing every group must store an empty array, not null: the consent
// image and the jq/python readers treat a null as "not migrated" and fall back
// to the self-editable traits.groups.
func TestGroupsPatch_NoGroupsIsEmptyArrayNotNull(t *testing.T) {
	ops := groupsPatch(nil)
	b, err := json.Marshal(ops[len(ops)-1]["value"])
	if err != nil {
		t.Fatal(err)
	}
	if got := string(b); got != `[]` {
		t.Fatalf("metadata_public.groups after removing every group = %s, want []", got)
	}
}
