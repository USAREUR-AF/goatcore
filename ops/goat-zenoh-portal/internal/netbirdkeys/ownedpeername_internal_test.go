package netbirdkeys

import "testing"

// `auto_peer_name` is a PREFIX: the patched mgmt image renders the joining peer
// as "<auto_peer_name>-<hostname>", so a machine minted as "machine-<handle>"
// never appears as "<handle>". CutMachine must not match on exact equality
// against the bare handle: that finds nothing, deletes the setup key and reports
// a clean mesh cut while the peer keeps its groups and its reach.
func TestOwnedPeerName(t *testing.T) {
	cases := []struct {
		name    string // the peer name on the mesh
		handle  string // the registry machine handle
		want    bool
		because string
	}{
		{"machine-rtr-gw-1-gw-pi-1", "rtr-gw-1", true,
			"the shape the mgmt image actually produces"},
		{"machine-octo-pod-test-machine-octo-pod-test-machine", "octo-pod-test-machine", true,
			"handle and hostname identical, so the handle appears twice"},
		{"machine-sam-baxter-1-sam-baxter-1", "sam-baxter-1", true,
			"same shape, different machine"},
		{"gw-lab-001", "gw-lab-001", true,
			"renamed to the bare handle by hand — the documented workaround, still has to cut"},
		{"machine-pod-lab-001", "pod-lab-001", true,
			"auto_peer_name with no hostname appended"},

		// The reason the trailing hyphen is in the implementation.
		{"machine-gw-11-thor", "gw-1", false,
			"handle gw-1 must NOT claim gw-11 — cutting the wrong machine is worse than not cutting"},
		{"machine-rtr-gw-10-x", "rtr-gw-1", false,
			"same trap one digit further out"},

		{"guestshell", "gw-lab-001", false, "an unrelated peer is not owned"},
		{"", "gw-lab-001", false, "empty name owns nothing"},
	}

	for _, c := range cases {
		t.Run(c.name+"/"+c.handle, func(t *testing.T) {
			got := ownedPeerName(c.name, "machine-"+c.handle, c.handle)
			if got != c.want {
				t.Errorf("ownedPeerName(%q, machine-%s, %s) = %v, want %v — %s",
					c.name, c.handle, c.handle, got, c.want, c.because)
			}
		})
	}
}
