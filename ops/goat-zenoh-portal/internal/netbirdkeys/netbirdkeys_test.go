package netbirdkeys_test

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/netbirdkeys"
)

// fakeNetbird stands in for a goatnet's mgmt API: it serves a group roster and
// records the setup-key body it was asked to create.
func fakeNetbird(t *testing.T, groups map[string]string) (*httptest.Server, *map[string]any) {
	t.Helper()
	var captured map[string]any
	mux := http.NewServeMux()
	mux.HandleFunc("/api/groups", func(w http.ResponseWriter, r *http.Request) {
		out := make([]map[string]string, 0, len(groups))
		for name, id := range groups {
			out = append(out, map[string]string{"id": id, "name": name})
		}
		_ = json.NewEncoder(w).Encode(out)
	})
	mux.HandleFunc("/api/setup-keys", func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&captured)
		w.WriteHeader(http.StatusCreated)
		_ = json.NewEncoder(w).Encode(map[string]string{"id": "key-1", "key": "AAAA-BBBB-CCCC"})
	})
	ts := httptest.NewServer(mux)
	t.Cleanup(ts.Close)
	return ts, &captured
}

func TestMintForGroups_DerivedGroupsAndOneOffShape(t *testing.T) {
	ts, captured := fakeNetbird(t, map[string]string{
		"GALAXY-PROD-KID":      "gid-kid",
		"GALAXY-PROD-OPSYSTEM": "gid-opsystem",
	})
	c := netbirdkeys.New(ts.URL, "tok", "GALAXY-PROD")
	if !c.Enabled() {
		t.Fatal("client should be enabled")
	}

	sk, err := c.MintForGroups(t.Context(), "machine-gw-rtr-001", []string{"KID"}, 24*time.Hour)
	if err != nil {
		t.Fatalf("MintForGroups: %v", err)
	}
	if sk.Key != "AAAA-BBBB-CCCC" {
		t.Fatalf("key = %q", sk.Key)
	}
	if len(sk.Groups) != 1 || sk.Groups[0] != "GALAXY-PROD-KID" {
		t.Fatalf("groups = %v, want the prefixed KID group", sk.Groups)
	}

	body := *captured
	// One-off, single-use: a machine key is minted for ONE machine at a mint
	// event with a recorded custody edge. A reusable key is a standing
	// invitation nobody re-reads, which is the shape that let three unlike
	// things land in one group in a single sitting (ADR 1116 §9).
	if body["type"] != "one-off" {
		t.Errorf("type = %v, want one-off", body["type"])
	}
	if ul, _ := body["usage_limit"].(float64); ul != 1 {
		t.Errorf("usage_limit = %v, want 1", body["usage_limit"])
	}
	ag, _ := body["auto_groups"].([]any)
	if len(ag) != 1 || ag[0] != "gid-kid" {
		t.Errorf("auto_groups = %v, want exactly the resolved KID group id", body["auto_groups"])
	}
	// The catch-all must not ride along just because it exists on the server.
	for _, g := range ag {
		if g == "gid-opsystem" {
			t.Fatal("OPSYSTEM leaked into a derived key")
		}
	}

	// auto_peer_name is what makes the resulting PEER traceable. Without it the
	// patched mgmt image has no prefix to combine with the joining hostname, so
	// the machine registers as bare `<hostname>` — and the revoke selects peers
	// by matching `^peer-<handle>-<site>-` (operator-onboard.yml R1.2), so a
	// bare-named peer is not cut and the cut still reports success.
	//
	// Without this field every machine minted through this path lands
	// unattributable, and its peer can only be recovered from the management
	// server's enrolment events (ops/scripts/peer-attribution.py).
	if body["auto_peer_name"] != "machine-gw-rtr-001" {
		t.Errorf("auto_peer_name = %v, want the key name — a peer with no name prefix "+
			"cannot be traced to its machine record and is invisible to the revoke",
			body["auto_peer_name"])
	}
}

// A derived group that does not exist on the live server is a REFUSAL. Minting
// with the groups that do resolve would quietly under-enroll the machine, and an
// under-enrolled peer looks configured while reaching nothing.
func TestMintForGroups_MissingGroupRefuses(t *testing.T) {
	ts, captured := fakeNetbird(t, map[string]string{"GALAXY-PROD-KID": "gid-kid"})
	c := netbirdkeys.New(ts.URL, "tok", "GALAXY-PROD")

	_, err := c.MintForGroups(t.Context(), "m", []string{"KID", "SENSOR-FEED"}, time.Hour)
	if !errors.Is(err, netbirdkeys.ErrGroupMissing) {
		t.Fatalf("err = %v, want ErrGroupMissing", err)
	}
	if !strings.Contains(err.Error(), "GALAXY-PROD-SENSOR-FEED") {
		t.Fatalf("refusal should name the missing group: %v", err)
	}
	if *captured != nil {
		t.Fatal("a key was minted despite a missing group — that is a partial grant")
	}
}

func TestMintForGroups_DisabledAndEmpty(t *testing.T) {
	// No site prefix ⇒ a suffix cannot resolve to a real group, so the client
	// reports itself disabled rather than minting against a bare suffix.
	if netbirdkeys.New("https://x", "tok", "").Enabled() {
		t.Fatal("client with no site prefix should be disabled")
	}
	if _, err := netbirdkeys.New("", "", "").MintForGroups(t.Context(), "m", []string{"KID"}, time.Hour); !errors.Is(err, netbirdkeys.ErrDisabled) {
		t.Fatalf("want ErrDisabled, got %v", err)
	}

	ts, _ := fakeNetbird(t, map[string]string{"GALAXY-KID": "g"})
	c := netbirdkeys.New(ts.URL, "tok", "GALAXY")
	if _, err := c.MintForGroups(t.Context(), "m", nil, time.Hour); err == nil {
		t.Fatal("a key with no auto_groups is a peer that enrols into nothing; want a refusal")
	}
}

func TestGroupName_TrimsTrailingSeparator(t *testing.T) {
	c := netbirdkeys.New("https://x", "t", "GALAXY-PROD-")
	if got := c.GroupName("KID"); got != "GALAXY-PROD-KID" {
		t.Fatalf("GroupName = %q", got)
	}
}
