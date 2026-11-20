package handlers

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func TestIsServiceOperatorKeyHandle(t *testing.T) {
	configured := []string{"e2e-operator"}
	for _, tc := range []struct {
		h    string
		want bool
	}{
		{"", false}, {"  ", false}, {"operator", true}, {"onboard-api", true}, {"id-console", true},
		{"onboard-runner", true}, {"onboard-runner-zagros", true}, {" onboard-runner-zagros ", true},
		{"e2e-operator", true},
		{"Operator", false}, {"onboard-runnerx", false}, {"op-alice", false}, {"e2e-runner", false},
	} {
		if got := isServiceKeyHandle(tc.h, configured); got != tc.want {
			t.Errorf("isServiceKeyHandle(%q) = %v, want %v", tc.h, got, tc.want)
		}
	}
}

// A requested key that was never issued is revoked in the record only; a
// file under the same handle is not this console's and stays.
func TestOffboard_neverIssuedRowTouchesNoFile(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	other := filepath.Join(f.dir, f.pending.Handle+".key")
	if err := os.WriteFile(other, []byte("op-alice-2:not-ours\n"), 0o640); err != nil {
		t.Fatal(err)
	}
	if rec := postState(t, f.s, "id-alice", "other-admin@example.test", "inactive"); rec.Code != 303 {
		t.Fatalf("deactivation: %d", rec.Code)
	}
	row, err := f.s.Onboard.OperatorKey(context.Background(), f.pending.ID)
	if err != nil || row.Status != onboardstore.OperatorKeyRevoked {
		t.Fatalf("never-issued row after offboard = %+v %v; want revoked", row, err)
	}
	if b, err := os.ReadFile(other); err != nil || string(b) != "op-alice-2:not-ours\n" {
		t.Fatalf("the offboard touched a file it never issued: %q %v", b, err)
	}
	if secretOnDisk(t, f.dir, "op-alice", f.mine.Secret) {
		t.Fatal("issued key still on disk after offboard")
	}
}
