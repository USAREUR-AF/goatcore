package api

import (
	"context"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

// GHSA-q7qx hardening, through the real loaders: a person's key filed under a
// service label (in the env map or the key directory) is not accepted, so it
// cannot delegate attribution; the provisioned service keys (legacy
// PORTAL_OPERATOR_API_KEY, the env-pinned id-console key) still can, and an
// ordinary directory service key still authenticates as itself.
func TestServiceHandleKeys_onlyProvisionedServiceKeysDelegate(t *testing.T) {
	dir := t.TempDir()
	for name, body := range map[string]string{
		"operator.key":    "operator:k-person-dir-operator\n",
		"onboard-api.key": "onboard-api:k-person-dir-onboard\n",
		"id-console.key":  "id-console:k-person-dir-console\n",
		"panoscope.key":   "panoscope:k-panoscope\n",
	} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	t.Setenv("PORTAL_OPERATOR_API_KEY", "k-legacy")
	t.Setenv("PORTAL_OPERATOR_API_KEYS", "operator:k-person-env,id-console:k-console,jane-operator:k-jane")
	t.Setenv("PORTAL_OPERATOR_API_KEYS_DIR", dir)
	cfg, err := config.LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	s := &Server{cfg: cfg}

	cases := []struct {
		name, token, wantActor string // wantActor "" = token refused
	}{
		{"person key under operator in env", "k-person-env", ""},
		{"person key under operator in dir", "k-person-dir-operator", ""},
		{"person key under onboard-api in dir", "k-person-dir-onboard", ""},
		{"person key under id-console in dir", "k-person-dir-console", ""},
		{"legacy shared key delegates", "k-legacy", "alice"},
		{"env-pinned id-console key delegates", "k-console", "alice"},
		{"per-person key does not delegate", "k-jane", "jane-operator"},
		{"directory service key does not delegate", "k-panoscope", "panoscope"},
	}
	for _, c := range cases {
		p, err := s.resolvePrincipal(context.Background(), c.token)
		if err != nil {
			t.Fatalf("%s: resolvePrincipal: %v", c.name, err)
		}
		if c.wantActor == "" {
			if p != nil {
				t.Errorf("%s: accepted as %+v, want refused", c.name, *p)
			}
			continue
		}
		if p == nil {
			t.Errorf("%s: refused, want actor %q", c.name, c.wantActor)
			continue
		}
		r := httptest.NewRequest("POST", "/api/orgs", nil)
		r.Header.Set("X-Goat-Operator-Actor", "alice")
		r = r.WithContext(context.WithValue(r.Context(), principalKey, p))
		if got := actorOf(r); got != c.wantActor {
			t.Errorf("%s: actorOf = %q, want %q", c.name, got, c.wantActor)
		}
	}
}
