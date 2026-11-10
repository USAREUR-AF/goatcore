package measure

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

func TestKillSwitch(t *testing.T) {
	t.Run("absent", func(t *testing.T) {
		t.Setenv("SNITCH_PROBE_DISABLED", "")
		if err := os.Unsetenv("SNITCH_PROBE_DISABLED"); err != nil {
			t.Fatalf("unsetenv: %v", err)
		}
		_, ok := killSwitchActive()
		if ok {
			t.Error("kill switch should be inactive when env unset")
		}
	})
	t.Run("set", func(t *testing.T) {
		t.Setenv("SNITCH_PROBE_DISABLED", "operator-fiat-2026-05-04")
		reason, ok := killSwitchActive()
		if !ok {
			t.Error("kill switch should be active when env set")
		}
		if reason == "" {
			t.Error("reason should be non-empty")
		}
	})
}

func TestPolicyAllowsTable(t *testing.T) {
	tests := []struct {
		mode             string
		ping, relay, tls bool
		mtu, dialer      bool
		uncached         bool
	}{
		{types.PolicyOff, false, false, false, false, false, false},
		{types.PolicyMinimal, true, true, false, false, false, false},
		{types.PolicyDefault, true, true, true, true, true, false},
		{types.PolicyFocusedResearch, true, true, true, true, true, true},
	}
	for _, tt := range tests {
		t.Run(tt.mode, func(t *testing.T) {
			if PolicyAllowsPeerPing(tt.mode) != tt.ping {
				t.Errorf("PolicyAllowsPeerPing(%s) = %v; want %v",
					tt.mode, PolicyAllowsPeerPing(tt.mode), tt.ping)
			}
			if PolicyAllowsRelayProbe(tt.mode) != tt.relay {
				t.Errorf("PolicyAllowsRelayProbe(%s) = %v; want %v",
					tt.mode, PolicyAllowsRelayProbe(tt.mode), tt.relay)
			}
			if PolicyAllowsTLSProbe(tt.mode) != tt.tls {
				t.Errorf("PolicyAllowsTLSProbe(%s) = %v; want %v",
					tt.mode, PolicyAllowsTLSProbe(tt.mode), tt.tls)
			}
			if PolicyAllowsMTU(tt.mode) != tt.mtu {
				t.Errorf("PolicyAllowsMTU(%s) = %v; want %v",
					tt.mode, PolicyAllowsMTU(tt.mode), tt.mtu)
			}
			if PolicyAllowsDialerCounters(tt.mode) != tt.dialer {
				t.Errorf("PolicyAllowsDialerCounters(%s) = %v; want %v",
					tt.mode, PolicyAllowsDialerCounters(tt.mode), tt.dialer)
			}
			if PolicyForcesUncached(tt.mode) != tt.uncached {
				t.Errorf("PolicyForcesUncached(%s) = %v; want %v",
					tt.mode, PolicyForcesUncached(tt.mode), tt.uncached)
			}
		})
	}
}

func TestAppliedPolicy_Expiry(t *testing.T) {
	now := time.Date(2026, 5, 4, 12, 0, 0, 0, time.UTC)
	t.Run("not-expired", func(t *testing.T) {
		p := types.ProbePolicy{
			Mode:      types.PolicyFocusedResearch,
			ExpiresAt: now.Add(5 * time.Minute),
		}
		got := AppliedPolicy(p, now)
		if got.Mode != types.PolicyFocusedResearch {
			t.Errorf("non-expired policy should pass through; got %q", got.Mode)
		}
	})
	t.Run("expired-reverts-to-default", func(t *testing.T) {
		p := types.ProbePolicy{
			Mode:      types.PolicyFocusedResearch,
			ExpiresAt: now.Add(-1 * time.Minute),
		}
		got := AppliedPolicy(p, now)
		if got.Mode != types.PolicyDefault {
			t.Errorf("expired policy should revert to default; got %q", got.Mode)
		}
	})
	t.Run("no-expiry-stable", func(t *testing.T) {
		p := types.ProbePolicy{
			Mode: types.PolicyMinimal,
			// ExpiresAt is zero
		}
		got := AppliedPolicy(p, now)
		if got.Mode != types.PolicyMinimal {
			t.Errorf("zero-ExpiresAt policy should pass through; got %q", got.Mode)
		}
	})
}

func TestPolicyClient_FallbackOnCollectorDown(t *testing.T) {
	// Fresh client pointing at a port that nothing's listening on.
	pc := NewPolicyClient("http://127.0.0.1:1") // RFC reserved port; unlikely listener
	got, source := pc.CurrentPolicy(context.Background())
	if source != "fallback-default" {
		t.Errorf("first call w/ unreachable collector: source = %q; want fallback-default", source)
	}
	if got.Mode != types.PolicyDefault {
		t.Errorf("first call: mode = %q; want default", got.Mode)
	}
}

func TestPolicyClient_KillSwitchOverridesCollector(t *testing.T) {
	// Stand up a stub collector that returns PolicyDefault.
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(types.ProbePolicy{Mode: types.PolicyDefault})
	}))
	defer srv.Close()

	t.Setenv("SNITCH_PROBE_DISABLED", "test")

	pc := NewPolicyClient(srv.URL)
	got, source := pc.CurrentPolicy(context.Background())
	if source != "kill-switch-env" {
		t.Errorf("kill switch should override collector; source = %q", source)
	}
	if got.Mode != types.PolicyOff {
		t.Errorf("kill switch should force PolicyOff; got %q", got.Mode)
	}
}

func TestPolicyClient_FetchSuccess(t *testing.T) {
	expires := time.Now().Add(10 * time.Minute).Truncate(time.Second)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(types.ProbePolicy{
			Mode:      types.PolicyFocusedResearch,
			ExpiresAt: expires,
			Reason:    "weekly net-health snapshot",
			SetBy:     "operator:alice",
		})
	}))
	defer srv.Close()

	if err := os.Unsetenv("SNITCH_PROBE_DISABLED"); err != nil {
		t.Fatalf("unsetenv: %v", err)
	}
	pc := NewPolicyClient(srv.URL)
	got, source := pc.CurrentPolicy(context.Background())
	if source != "collector" {
		t.Errorf("source = %q; want collector", source)
	}
	if got.Mode != types.PolicyFocusedResearch {
		t.Errorf("mode = %q; want focused-research", got.Mode)
	}
	if !got.ExpiresAt.Equal(expires) {
		t.Errorf("ExpiresAt mismatch: got %v want %v", got.ExpiresAt, expires)
	}
}
