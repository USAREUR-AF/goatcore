package api

import (
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
)

func TestMatchPeerHandle(t *testing.T) {
	roster := []access.RosterEntry{
		{Handle: "pat-example", Org: "usareur-af"},
		{Handle: "pat-example-2", Org: "usareur-af"}, // longer handle that shares a prefix
		{Handle: "alex-smith-example-org", Org: "example-org"},
		{Handle: "", Org: "x"}, // empty handle must never match
	}
	cases := []struct {
		name string
		peer string
		want string
	}{
		{"exact + site + date", "peer-pat-example-galaxy-2026-06-09", "pat-example"},
		{"longest prefix wins (—2 not —)", "peer-pat-example-2-galaxy-2026-06-09", "pat-example-2"},
		{"handle with hyphens + short_tag", "peer-alex-smith-example-org-alex-galaxy-2026-06-14-WSAMZN", "alex-smith-example-org"},
		{"no peer- prefix", "pat-example-galaxy", ""},
		{"unknown handle", "peer-nobody-here-galaxy-2026-01-01", ""},
		{"handle must be followed by a hyphen", "peer-pat-exampleXgalaxy", ""},
	}
	for _, c := range cases {
		if got := access.MatchPeerHandle(c.peer, roster); got != c.want {
			t.Errorf("%s: matchPeerHandle(%q) = %q, want %q", c.name, c.peer, got, c.want)
		}
	}
}

func TestEmailHashRoundTrips(t *testing.T) {
	// The resolver matches a roster email-hash to a live identity via access.EmailHash;
	// confirm case/space-insensitivity so a real identity reconciles.
	if access.EmailHash("person-902@Example.org ") != access.EmailHash("person-902@example.org") {
		t.Fatal("EmailHash is not case/space-insensitive; roster reconcile would miss")
	}
	if access.EmailHash("") != "" {
		t.Fatal("EmailHash(\"\") must be empty so no-email members don't false-match")
	}
}
