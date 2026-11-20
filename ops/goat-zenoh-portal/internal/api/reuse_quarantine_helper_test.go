package api_test

import (
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// pastReuseQuarantine moves the reuse-quarantine clock past the expiry of
// every leaf the test has minted, standing in for a year having passed.
func pastReuseQuarantine(t *testing.T) {
	t.Helper()
	saved := registry.SlotLeafClock
	registry.SlotLeafClock = func() time.Time { return time.Now().Add(400 * 24 * time.Hour) }
	t.Cleanup(func() { registry.SlotLeafClock = saved })
}
