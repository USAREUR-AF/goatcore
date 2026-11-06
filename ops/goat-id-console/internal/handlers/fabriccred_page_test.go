package handlers

// Fabric-credential collection (mint-on-collect). These go through Routes(),
// which is the only path that exercises identity -> Kratos -> portal -> page.
// A template-only test would pass while the page refused every visitor, which
// is exactly how F-309 shipped.

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

const collectorEmail = "robin@example.test"
const collectorKratosID = "9c1d2e3f-4a5b-4c6d-8e9f-0a1b2c3d4e5f"

// staticJSONRoundTripper is a minimal RoundTripper that returns a fixed JSON
// body with HTTP 200 for any request. Used by tests to drive Kratos client
// calls without binding a real listener.
type staticJSONRoundTripper struct {
	body string
}

func (rt staticJSONRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	res := &http.Response{
		StatusCode: http.StatusOK,
		Header:     make(http.Header),
		Body:       io.NopCloser(strings.NewReader(rt.body)),
		Request:    req,
	}
	res.Header.Set("Content-Type", "application/json")
	return res, nil
}

// portalRoundTripper records what collectFabricCredential sends and returns a
// canned JSON body with a chosen status. It drives portal calls without
// binding a real listener.
type portalRoundTripper struct {
	status      int
	body        any
	identityID  string
	actorHeader string
	bearer      string
}

func (rt *portalRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	var reqBody struct {
		IdentityID string `json:"identity_id"`
	}
	_ = json.NewDecoder(req.Body).Decode(&reqBody)
	rt.identityID = reqBody.IdentityID
	rt.actorHeader = req.Header.Get("X-Goat-Operator-Actor")
	rt.bearer = strings.TrimPrefix(req.Header.Get("Authorization"), "Bearer ")
	var buf bytes.Buffer
	if err := json.NewEncoder(&buf).Encode(rt.body); err != nil {
		buf.Reset()
	}
	res := &http.Response{
		StatusCode: rt.status,
		Header:     make(http.Header),
		Body:       io.NopCloser(bytes.NewReader(buf.Bytes())),
		Request:    req,
	}
	res.Header.Set("Content-Type", "application/json")
	return res, nil
}

// fakeKratosDirectory serves the one identity the collector resolves to via a
// stubbed Kratos client.
func fakeKratosDirectory(t *testing.T, email, id string) *kratos.Client {
	t.Helper()
	body := fmt.Sprintf(`[{"id":"%s","traits":{"email":"%s"}}]`, id, email)
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: staticJSONRoundTripper{body: body}}
	return cli
}

// fakeCollectPortal answers POST /api/orgs/collect with a canned status+body,
// and records what the console actually sent via a stubbed client.
func fakeCollectPortal(t *testing.T, status int, body any) (string, func() (identityID, actorHdr, bearer string)) {
	t.Helper()
	rt := &portalRoundTripper{status: status, body: body}
	prev := onboardHTTP
	onboardHTTP = &http.Client{Transport: rt}
	t.Cleanup(func() { onboardHTTP = prev })
	portalURL := "https://portal.example.test"
	return portalURL, func() (string, string, string) { return rt.identityID, rt.actorHeader, rt.bearer }
}

func fabricCredServer(t *testing.T, portalURL string) *Server {
	t.Helper()
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"
	if s.Kratos == nil {
		s.Kratos = map[string]*kratos.Client{}
	}
	s.Kratos["tenant"] = fakeKratosDirectory(t, collectorEmail, collectorKratosID)
	return s
}

func collectPost(t *testing.T, h http.Handler, s *Server, email string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/fabric-credential", strings.NewReader(form.Encode()))
	r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	if email != "" {
		r.Header.Set("X-Forwarded-Email", email)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	return w
}

// The landing page explains and offers; it must not leak anything, and must be
// reachable by an ordinary member with no groups at all.
func TestFabricCredential_LandingPageOffersCollectionToAnyMember(t *testing.T) {
	s := fabricCredServer(t, "http://portal.invalid")
	h := s.Routes()

	r := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/fabric-credential", nil)
	r.Header.Set("X-Forwarded-Email", collectorEmail)
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)

	if w.Code != http.StatusOK {
		t.Fatalf("GET /fabric-credential = %d, want 200; %s", w.Code, w.Body.String())
	}
	body := w.Body.String()
	if !strings.Contains(body, "Collect my fabric credential") {
		t.Error("landing page does not offer collection")
	}
	if strings.Contains(body, "shown once and cannot be shown again") {
		t.Error("landing page renders the post-collection hand-over copy")
	}
}

// The whole point: mint, show once, and say plainly that it will not be shown
// again — before the reader can navigate away.
func TestFabricCredential_CollectShowsTheSecretsOnceAndSaysSo(t *testing.T) {
	portalURL, sent := fakeCollectPortal(t, http.StatusCreated, map[string]any{
		"slot_id":       "a1b2c3d4e5f6",
		"system_handle": "robin-h",
		"api_key":       "vp_live_secret_value",
		"bundle_url":    "https://portal.example.test/bundles/a1b2c3d4e5f6/tok",
		"uri_san":       "urn:goat:zagros:org=robin-h",
	})
	s := fabricCredServer(t, portalURL)
	w := collectPost(t, s.Routes(), s, collectorEmail)

	if w.Code != http.StatusOK {
		t.Fatalf("collect = %d, want 200; %s", w.Code, w.Body.String())
	}
	body := w.Body.String()
	for _, must := range []string{
		"vp_live_secret_value",
		"https://portal.example.test/bundles/a1b2c3d4e5f6/tok",
		"shown once and cannot be shown again",
		"a1b2c3d4e5f6", // the write namespace
	} {
		if !strings.Contains(body, must) {
			t.Errorf("hand-over page missing %q", must)
		}
	}

	// The console sent WHO, not WHICH SLOT, and named the human for the audit.
	identityID, actorHdr, bearer := sent()
	if identityID != collectorKratosID {
		t.Errorf("portal got identity_id %q, want the session's Kratos id %q", identityID, collectorKratosID)
	}
	if actorHdr != collectorEmail {
		t.Errorf("X-Goat-Operator-Actor = %q, want the collector's address so the mint is attributable", actorHdr)
	}
	if bearer != "svc-token" {
		t.Errorf("portal bearer = %q, want the console's portal token", bearer)
	}
}

// A refusal must tell the reader what is true and who can change it. The
// already-minted case is the one every pre-existing member will hit, so it has
// to say "you already have one" rather than "409".
func TestFabricCredential_RefusalsAreActionableAndLeakNothing(t *testing.T) {
	cases := []struct {
		name   string
		status int
		code   string
		want   string
	}{
		{"already minted at onboarding", http.StatusConflict, "already_minted", "check your welcome email"},
		{"already collected", http.StatusConflict, "already_collected", "cannot be read again"},
		{"no slot for this person", http.StatusNotFound, "no_slot", "nothing to collect"},
		{"duplicate linkage", http.StatusConflict, "ambiguous_identity", "fix the duplicate"},
		{"onboarding incomplete", http.StatusConflict, "no_org_row", "did not finish"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			portalURL, _ := fakeCollectPortal(t, tc.status, map[string]any{
				"error": map[string]string{"code": tc.code, "message": "portal detail"},
			})
			s := fabricCredServer(t, portalURL)
			w := collectPost(t, s.Routes(), s, collectorEmail)

			if w.Code != http.StatusOK {
				t.Fatalf("refusal rendered %d, want the page with a flash; %s", w.Code, w.Body.String())
			}
			body := w.Body.String()
			if !strings.Contains(body, tc.want) {
				t.Errorf("refusal for %s does not say %q; got: %s", tc.code, tc.want, body)
			}
			if strings.Contains(body, "shown once and cannot be shown again") {
				t.Errorf("refusal for %s rendered the hand-over page", tc.code)
			}
		})
	}
}

// Authentication is required, and the CSRF nonce is required — the POST mints a
// live credential, so a cross-site form post must not be able to spend
// somebody's one collection.
func TestFabricCredential_AnonymousAndCSRFlessPostsRefused(t *testing.T) {
	portalURL, sent := fakeCollectPortal(t, http.StatusCreated, map[string]any{"slot_id": "x", "api_key": "should-never-mint"})
	s := fabricCredServer(t, portalURL)
	h := s.Routes()

	if w := collectPost(t, h, s, ""); w.Code != http.StatusUnauthorized {
		t.Errorf("anonymous collect = %d, want 401", w.Code)
	}

	r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/fabric-credential", strings.NewReader("csrf=wrong"))
	r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	r.Header.Set("X-Forwarded-Email", collectorEmail)
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	if w.Code != http.StatusForbidden {
		t.Errorf("collect with a bad CSRF nonce = %d, want 403", w.Code)
	}

	if id, _, _ := sent(); id != "" {
		t.Errorf("the portal was called (identity %q) despite both requests being refused", id)
	}
}
