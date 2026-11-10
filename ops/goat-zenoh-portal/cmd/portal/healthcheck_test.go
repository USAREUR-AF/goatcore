package main

// The healthcheck subcommand is the portal's container healthcheck: the runtime
// image is distroless, so no shell/curl/wget probe can run in it. These tests pin the exit-code contract the compose
// stanza depends on: nil only for a 200 that says status "ok".

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestHealthcheck_HealthyPortal(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"status":"ok","registry":"ok","version":"0.1.0"}`))
	}))
	defer ts.Close()

	if err := cmdHealthcheck([]string{"--url", ts.URL + "/api/health"}); err != nil {
		t.Errorf("healthy portal reported unhealthy: %v", err)
	}
}

func TestHealthcheck_UnhealthyPortalCarriesTheReason(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = w.Write([]byte(`{"status":"unhealthy","reason":"registry probe failed: database is closed"}`))
	}))
	defer ts.Close()

	err := cmdHealthcheck([]string{"--url", ts.URL + "/api/health"})
	if err == nil {
		t.Fatal("503 reported healthy")
	}
	if !strings.Contains(err.Error(), "registry probe failed") {
		t.Errorf("error drops the portal's reason, leaving the operator nothing to act on: %v", err)
	}
}

// A 200 whose body says otherwise must not pass — trusting the status code
// alone is how a payload reading "unhealthy" gets reported as healthy.
func TestHealthcheck_200ButNotOkFails(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{"status":"degraded"}`))
	}))
	defer ts.Close()

	if err := cmdHealthcheck([]string{"--url", ts.URL + "/api/health"}); err == nil {
		t.Error("200 with status=degraded reported healthy")
	}
}

// A dead portal — nothing listening — is the case the healthcheck exists for.
func TestHealthcheck_UnreachablePortalFails(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {}))
	url := ts.URL + "/api/health"
	ts.Close() // nothing listening now

	if err := cmdHealthcheck([]string{"--url", url, "--timeout", "1s"}); err == nil {
		t.Error("unreachable portal reported healthy")
	}
}

// Non-JSON (an HTML error page from something else on the port) is unhealthy,
// not a crash.
func TestHealthcheck_NonJSONBodyFails(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("<html>not the portal</html>"))
	}))
	defer ts.Close()

	if err := cmdHealthcheck([]string{"--url", ts.URL + "/api/health"}); err == nil {
		t.Error("non-JSON body reported healthy")
	}
}
