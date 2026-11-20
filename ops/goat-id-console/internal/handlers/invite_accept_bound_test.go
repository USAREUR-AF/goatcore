package handlers

// F-475 boundary on the login-invite path: the address is imported verified
// only when an admin BOUND it to the invite at mint. The accept route is
// unauthenticated (the token is the auth) and the welcome link is shown on
// screen, so an address the redeemer types proves nothing about the mailbox.
// These tests pin both sides against a fake Kratos that records the create
// body: a bound invite mints the bound address verified whatever the form
// says; an unbound invite mints the typed address with no verifiable_addresses
// at all; and the mint form is where the binding happens.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
)

// fakeKratosMint answers the two admin calls the accept path makes and keeps
// every identity-create body for the assertions.
type fakeKratosMint struct {
	creates []map[string]any
}

func newFakeKratosMint(t *testing.T) (*httptest.Server, *fakeKratosMint) {
	t.Helper()
	f := &fakeKratosMint{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/admin/identities":
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			f.creates = append(f.creates, body)
			w.WriteHeader(http.StatusCreated)
			_ = json.NewEncoder(w).Encode(map[string]any{"id": "id-1", "traits": body["traits"]})
		case r.Method == http.MethodPost && r.URL.Path == "/admin/recovery/code":
			_ = json.NewEncoder(w).Encode(map[string]any{
				"recovery_link": "https://idp.example/self-service/recovery?flow=1",
				"recovery_code": "12345678",
				"expires_at":    time.Now().Add(time.Hour).UTC().Format(time.RFC3339),
			})
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(srv.Close)
	return srv, f
}

func newInviteTestServer(t *testing.T) (*Server, *fakeKratosMint) {
	t.Helper()
	s := newOnboardTestServer(t)
	srv, fake := newFakeKratosMint(t)
	s.Kratos = map[string]*kratos.Client{"tenant": kratos.NewClient(srv.URL)}
	return s, fake
}

func postForm(t *testing.T, s *Server, path string, form url.Values) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, req)
	return w
}

func onlyCreate(t *testing.T, fake *fakeKratosMint) map[string]any {
	t.Helper()
	if len(fake.creates) != 1 {
		t.Fatalf("Kratos saw %d identity creates, want 1", len(fake.creates))
	}
	return fake.creates[0]
}

func TestInviteAccept_BoundInviteMintsTheBoundAddressVerified(t *testing.T) {
	s, fake := newInviteTestServer(t)
	inv, err := s.Invites.Create("tenant", "admin@example.org", "alice@example.org", []string{"IAM-ADMIN"}, time.Hour)
	if err != nil {
		t.Fatal(err)
	}

	// The redeemer types someone else's address. It must not matter.
	w := postForm(t, s, "/invites/"+inv.Token+"/accept", url.Values{
		"email": {"mallory@example.org"}, "name": {"Mallory"},
	})
	if w.Code != http.StatusOK {
		t.Fatalf("accept = %d: %s", w.Code, w.Body.String())
	}
	body := onlyCreate(t, fake)
	traits, _ := body["traits"].(map[string]any)
	if traits["email"] != "alice@example.org" {
		t.Errorf("minted traits.email = %v, want the bound address", traits["email"])
	}
	addrs, ok := body["verifiable_addresses"].([]any)
	if !ok || len(addrs) != 1 {
		t.Fatalf("verifiable_addresses = %v, want exactly one entry", body["verifiable_addresses"])
	}
	addr, _ := addrs[0].(map[string]any)
	if addr["value"] != "alice@example.org" || addr["verified"] != true {
		t.Errorf("verifiable address = %v, want the bound address verified", addr)
	}
	got, _ := s.Invites.Get(inv.Token)
	if got.UsedByEmail != "alice@example.org" {
		t.Errorf("invite used_by = %q, want the bound address", got.UsedByEmail)
	}
	assertCandidateInviteResponse(t, w)
	if !strings.Contains(w.Body.String(), `href="https://idp.example/self-service/recovery?flow=1"`) {
		t.Error("bound invite must retain the clickable candidate setup link")
	}
	for _, event := range s.Audit.Recent(10) {
		if event.Action == "create_identity" {
			if event.After["email_verified"] != true || event.After["email"] != "alice@example.org" {
				t.Fatalf("bound identity audit lost the verification boundary: %v", event.After)
			}
			return
		}
	}
	t.Error("bound identity creation audit missing")
}

func assertCandidateInviteResponse(t *testing.T, w *httptest.ResponseRecorder) {
	t.Helper()
	for _, forbidden := range []string{"<nav", "stepup-strip", "All users", "User detail"} {
		if strings.Contains(w.Body.String(), forbidden) {
			t.Errorf("candidate invite response contains %q", forbidden)
		}
	}
	if w.Header().Get("Cache-Control") != "no-store" || w.Header().Get("Referrer-Policy") != "no-referrer" {
		t.Error("candidate invitation response lost its privacy headers")
	}
}

func TestInviteAccept_BoundInviteDoesNotRequireAPostedEmail(t *testing.T) {
	s, fake := newInviteTestServer(t)
	inv, _ := s.Invites.Create("tenant", "admin@example.org", "alice@example.org", []string{"IAM-ADMIN"}, time.Hour)
	w := postForm(t, s, "/invites/"+inv.Token+"/accept", url.Values{"name": {"Alice"}})
	if w.Code != http.StatusOK {
		t.Fatalf("bound accept without posted email = %d: %s", w.Code, w.Body.String())
	}
	traits, _ := onlyCreate(t, fake)["traits"].(map[string]any)
	if traits["email"] != "alice@example.org" {
		t.Fatalf("bound email replaced by missing posted field: %v", traits)
	}
}

func TestInviteAccept_UnboundInviteMintsTheTypedAddressUnverified(t *testing.T) {
	s, fake := newInviteTestServer(t)
	inv, err := s.Invites.Create("tenant", "admin@example.org", "", []string{"IAM-ADMIN"}, time.Hour)
	if err != nil {
		t.Fatal(err)
	}

	w := postForm(t, s, "/invites/"+inv.Token+"/accept", url.Values{"email": {"bob@example.org"}})
	if w.Code != http.StatusOK {
		t.Fatalf("accept = %d: %s", w.Code, w.Body.String())
	}
	body := onlyCreate(t, fake)
	traits, _ := body["traits"].(map[string]any)
	if traits["email"] != "bob@example.org" {
		t.Errorf("minted traits.email = %v, want the typed address", traits["email"])
	}
	if v, has := body["verifiable_addresses"]; has {
		t.Errorf("an unbound invite imported verifiable_addresses %v; a self-asserted address is never verified", v)
	}
	assertCandidateInviteResponse(t, w)
}

func TestInviteAccept_UnboundInviteStillNeedsAnAddress(t *testing.T) {
	s, fake := newInviteTestServer(t)
	inv, _ := s.Invites.Create("tenant", "admin@example.org", "", []string{"IAM-ADMIN"}, time.Hour)

	w := postForm(t, s, "/invites/"+inv.Token+"/accept", url.Values{"name": {"Nobody"}})
	if w.Code != http.StatusBadRequest {
		t.Fatalf("accept without an address = %d, want 400", w.Code)
	}
	if len(fake.creates) != 0 {
		t.Errorf("Kratos saw %d creates for a refused accept", len(fake.creates))
	}
}

// The accept page shows a bound address read-only, so the redeemer learns the
// invite is not theirs before they submit anything.
func TestInviteAccept_FormShowsTheBoundAddressReadOnly(t *testing.T) {
	s, _ := newInviteTestServer(t)
	inv, _ := s.Invites.Create("tenant", "admin@example.org", "alice@example.org", []string{"IAM-ADMIN"}, time.Hour)

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/invites/"+inv.Token+"/accept", nil)
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("accept form = %d: %s", w.Code, w.Body.String())
	}
	page := w.Body.String()
	if !strings.Contains(page, `value="alice@example.org" readonly`) {
		t.Errorf("accept form does not render the bound address read-only:\n%s", page)
	}
	assertCandidateInviteResponse(t, w)
}

// Binding happens on the mint form: an admin-typed address is kept on the
// invite, a malformed one is refused before anything is minted.
func TestInviteCreate_BindsTheAddressTheAdminTyped(t *testing.T) {
	s, _ := newInviteTestServer(t)
	// A non-admin preset: admin-bearing presets are refused (GHSA-7788).
	s.Permissions.Groups = append(s.Permissions.Groups, permissions.Group{Name: "DEVELOPER"})
	form := url.Values{
		"csrf": {s.csrfNonce}, "preset_groups": {"DEVELOPER"}, "ttl_hours": {"24"},
		"email": {"Alice Example <alice@example.org>"},
	}
	w := postForm(t, s, "/invites?idp=tenant", form)
	if w.Code != http.StatusOK {
		t.Fatalf("mint = %d: %s", w.Code, w.Body.String())
	}
	all, _ := s.Invites.List()
	if len(all) != 1 {
		t.Fatalf("store holds %d invites, want 1", len(all))
	}
	if all[0].Email != "alice@example.org" {
		t.Errorf("invite bound to %q, want the bare address the admin typed", all[0].Email)
	}
	if !strings.Contains(w.Body.String(), "alice@example.org") {
		t.Errorf("minted page does not show the bound address")
	}
}

func TestInviteCreate_RefusesAMalformedAddress(t *testing.T) {
	s, _ := newInviteTestServer(t)
	// A non-admin preset: admin-bearing presets are refused (GHSA-7788).
	s.Permissions.Groups = append(s.Permissions.Groups, permissions.Group{Name: "DEVELOPER"})
	form := url.Values{
		"csrf": {s.csrfNonce}, "preset_groups": {"DEVELOPER"}, "ttl_hours": {"24"},
		"email": {"alice at example dot org"},
	}
	w := postForm(t, s, "/invites?idp=tenant", form)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("mint with a malformed address = %d, want 400", w.Code)
	}
	all, _ := s.Invites.List()
	if len(all) != 0 {
		t.Errorf("store holds %d invites after a refused mint", len(all))
	}
}
