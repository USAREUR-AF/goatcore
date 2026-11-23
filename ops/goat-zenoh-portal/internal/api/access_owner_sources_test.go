package api

import (
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
)

// Review of #977, item 1: a layer the owner tiers read that did not answer
// arrived as HTTP 200 with every owner "none". The batch answer now says so
// machine-readably, and every row's owner is unread.
func TestPeersAnswerSaysWhenAnOwnerSourceFailed(t *testing.T) {
	src := recordSources()
	ok := peersAnswer(src, nil, []string{"machine-anvil-pod-7"})
	if s := ok["owner_sources"].(map[string]any); s["ok"] != true {
		t.Fatalf("owner_sources = %v, want ok", s)
	}
	row := ok["peers"].(map[string]map[string]any)["machine-anvil-pod-7"]
	if row["owner"].(map[string]any)["source"] != "record" {
		t.Fatalf("owner = %v, want record when every source answered", row["owner"])
	}

	for _, kind := range []string{"registry_unavailable", "slot_aliases_unavailable", "machines_unavailable", "identity_layer_error", "identity_layer_unavailable"} {
		warn := []access.Note{{Kind: kind, Severity: access.SeverityError, Text: kind}}
		out := peersAnswer(src, warn, []string{"machine-anvil-pod-7", "edge-gw-unlisted"})
		s := out["owner_sources"].(map[string]any)
		if s["ok"] != false || strings.Join(s["failed"].([]string), ",") != kind {
			t.Fatalf("%s: owner_sources = %v, want ok false naming it", kind, s)
		}
		for name, row := range out["peers"].(map[string]map[string]any) {
			o := row["owner"].(map[string]any)
			if o["source"] != "unread" || !strings.Contains(o["reason"].(string), kind) {
				t.Fatalf("%s: %s owner = %v, want unread naming the failed source", kind, name, o)
			}
		}
	}

	// A failure in a layer the owner tiers do not read leaves the owners read.
	mesh := []access.Note{{Kind: "mesh_layer_unavailable", Severity: access.SeverityError}}
	if s := peersAnswer(src, mesh, []string{"machine-anvil-pod-7"})["owner_sources"].(map[string]any); s["ok"] != true {
		t.Fatalf("mesh failure: owner_sources = %v, want ok", s)
	}
}
