package types

import (
	"encoding/json"
	"testing"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
)

func TestSatlinkRangeEmittedUnderBothKeys(t *testing.T) {
	for name, v := range map[string]any{
		"probe result":  &SatlinkProbeResult{EgressIP: "192.0.2.1", InSatlinkRange: true},
		"pop event":     SatlinkPopEvent{EgressIP: "192.0.2.1", InSatlinkRange: true},
		"probe (value)": SatlinkProbeResult{InSatlinkRange: false},
	} {
		b, err := json.Marshal(v)
		if err != nil {
			t.Fatalf("%s: marshal: %v", name, err)
		}
		var m map[string]any
		if err := json.Unmarshal(b, &m); err != nil {
			t.Fatalf("%s: output is not a JSON object: %v (%s)", name, err, b)
		}
		cur, ok1 := m["in_satlink_range"]
		old, ok2 := m[compat.SatlinkRangeJSONKey]
		if !ok1 || !ok2 || cur != old {
			t.Fatalf("%s: want both keys equal, got %s", name, b)
		}
		if _, ok := m["egress_ip"]; !ok {
			t.Fatalf("%s: other fields lost: %s", name, b)
		}
	}
}
