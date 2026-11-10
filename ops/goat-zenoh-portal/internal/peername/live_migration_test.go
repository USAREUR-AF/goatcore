package peername

import "testing"

// Name shapes as minted on a slot-pool goatnet. Each was minted
// with the degenerate tag==handle. The point of the dual accept is that a
// revoke selector still claims every one of them after mint stopped emitting
// that form; a miss here is a credential that stays live after a cut.
func TestLiveDoubledNamesStillClaimed(t *testing.T) {
	cases := []struct{ name, handle string }{
		{"peer-jane-doe-jane-doe-galaxy-2026-06-16-JD-2.local", "jane-doe"},
		{"peer-demo-org-008-demo-org-008-galaxy-2026-06-24-ZENOH3", "demo-org-008"},
		{"peer-sam-lee-sam-lee-galaxy-2026-06-18-Sams-MacBook-Air.local", "sam-lee"},
		{"peer-alex-smith-alex-smith-galaxy-2026-06-14-Laptops-MacBook-Pro.local", "alex-smith"},
		{"peer-kim-park-kim-park-galaxy-2026-06-16-DESKTOP-0A1B2C3", "kim-park"},
		{"peer-example-co-example-co-galaxy-2026-06-17-ip-10-0-0-12", "example-co"},
		{"peer-lee-hollis-lee-hollis-galaxy-2026-06-23-HOST-S-03", "lee-hollis"},
		{"peer-demo-org-500-demo-org-500-galaxy-2026-06-24-demo-mac.local", "demo-org-500"},
	}
	for _, c := range cases {
		if !BelongsTo(c.name, c.handle, c.handle, "galaxy") {
			t.Errorf("REVOKE WOULD MISS %s (handle %s)", c.name, c.handle)
		}
		// And the form mint produces from today.
		fresh := Mint(c.handle, c.handle, "galaxy", "2026-08-15")
		if !BelongsTo(fresh+"-host", c.handle, c.handle, "galaxy") {
			t.Errorf("selector misses the NEW form for %s: %s", c.handle, fresh)
		}
	}
}
