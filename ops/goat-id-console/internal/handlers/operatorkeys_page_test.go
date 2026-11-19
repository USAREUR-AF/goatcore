package handlers

import (
	"io/fs"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/web"
)

// The wording is the deliverable here as much as the controls. An operator key
// differs from an SSH key precisely at the tail: approval IS the grant, with no
// installer step afterwards. Borrowing the SSH page's "not yet" would leave
// people waiting for something that never happens.
func TestTemplates_operatorKeysPage(t *testing.T) {
	base, err := loadTemplates()
	if err != nil {
		t.Fatalf("loadTemplates: %v", err)
	}
	requested := &onboardstore.OperatorKey{
		ID: "opk-1", Handle: "ada", Env: "zagros", Reason: "minting slots",
		Status: onboardstore.OperatorKeyRequested,
	}
	active := &onboardstore.OperatorKey{
		ID: "opk-2", Handle: "bob", Env: "zagros", Status: onboardstore.OperatorKeyActive,
		Fingerprint: "sha256:beef", DecidedBy: "admin@example.test", DeliveredAt: 1,
	}
	collectable := &onboardstore.OperatorKey{
		ID: "opk-3", Handle: "ada", Env: "zagros", Status: onboardstore.OperatorKeyActive,
		Fingerprint: "sha256:cafe", Secret: "op_still_here",
	}

	cases := []struct {
		name string
		vars *pageVars
		must []string
		not  []string
	}{
		{
			name: "self view says a request grants nothing",
			vars: &pageVars{OperatorKeySelf: true, OperatorKeyHandle: "ada",
				OperatorKeys: []*onboardstore.OperatorKey{requested}},
			must: []string{"grants nothing", "Waiting on an admin. No access yet."},
			// The person whose key it is cannot be the one who approves it.
			not: []string{"/approve", "/reject", "/operator-keys/opk-1/revoke"},
		},
		{
			name: "a collectable key offers exactly one collection and says so",
			vars: &pageVars{OperatorKeySelf: true, OperatorKeyHandle: "ada",
				OperatorKeys: []*onboardstore.OperatorKey{collectable}},
			must: []string{"/operator-key/collect/opk-3", "shown only once"},
			// The list must never print the value; only the collect page does.
			not: []string{"op_still_here"},
		},
		{
			name: "an already-collected key does not pretend it can be shown again",
			vars: &pageVars{OperatorKeySelf: true, OperatorKeyHandle: "bob",
				OperatorKeys: []*onboardstore.OperatorKey{active}},
			must: []string{"already collected", "cannot show it again"},
			not:  []string{"/operator-key/collect/opk-2"},
		},
		{
			name: "the collect page shows the key and warns before they navigate away",
			vars: &pageVars{OperatorKeySelf: true, OperatorKeyHandle: "ada",
				OperatorKeySecret: "op_abc123"},
			must: []string{"op_abc123", "shown once and cannot be shown again", "revoke it and request a new one"},
		},
		{
			name: "queue says approving is the grant, not a step before one",
			vars: &pageVars{OperatorKeyRows: []operatorKeyRow{{OperatorKey: requested}, {OperatorKey: active}},
				OperatorKeyStatus: onboardstore.OperatorKeyRequested, OperatorKeyDirPath: "/opt/keys.d"},
			must: []string{
				"Approving <strong>is</strong> the\ngrant",
				"no installer step",
				"/operator-keys/opk-1/approve", "/operator-keys/opk-1/reject",
				"/operator-keys/opk-2/revoke",
			},
			// A request that has not been decided cannot be revoked.
			not: []string{"/operator-keys/opk-1/revoke"},
		},
		{
			// #210: the viewing admin's own pending row is VISIBLE — vanishing
			// it would read as a bug to the person who filed it — but carries
			// plain text where approve/reject would be.
			name: "the queue disables decisions on your own row and says why",
			vars: &pageVars{OperatorKeyRows: []operatorKeyRow{{OperatorKey: requested, Own: true}},
				OperatorKeyStatus: onboardstore.OperatorKeyRequested, OperatorKeyDirPath: "/opt/keys.d"},
			must: []string{"ada", "Your request — another admin must decide."},
			not:  []string{"/operator-keys/opk-1/approve", "/operator-keys/opk-1/reject"},
		},
		{
			// #210 review addendum 3: the dashboard tab navigates here, so the
			// round-trip it implies must exist.
			name: "the queue links back to the onboarding review",
			vars: &pageVars{OperatorKeyStatus: onboardstore.OperatorKeyRequested},
			must: []string{`href="/onboarding"`, "Onboarding review"},
		},
		{
			// #210 follow-up: the row this viewer cannot be compared against
			// carries no controls either, and says which door to use instead.
			name: "a row that cannot be compared says so instead of offering controls",
			vars: &pageVars{OperatorKeyRows: []operatorKeyRow{{OperatorKey: requested, Unverifiable: true}},
				OperatorKeyStatus: onboardstore.OperatorKeyRequested, OperatorKeyDirPath: "/opt/keys.d"},
			must: []string{"Not decidable from this session", "goat admin operator-key"},
			not:  []string{"/operator-keys/opk-1/approve", "/operator-keys/opk-1/reject"},
		},
		{
			// GHSA-2w7v: a request filed before the filer was recorded cannot be
			// decided by anyone; the page says to withdraw and re-request.
			name: "a pre-upgrade request says to re-request instead of offering controls",
			vars: &pageVars{OperatorKeyRows: []operatorKeyRow{{OperatorKey: requested, Unverifiable: true, LegacyRequest: true}},
				OperatorKeyStatus: onboardstore.OperatorKeyRequested, OperatorKeyDirPath: "/opt/keys.d"},
			must: []string{"Filed before the upgrade", "request again"},
			not:  []string{"/operator-keys/opk-1/approve", "/operator-keys/opk-1/reject", "Not decidable from this session"},
		},
		{
			// Self-revoke is de-escalation and stays enabled on an own row.
			name: "your own live key can still be revoked by you",
			vars: &pageVars{OperatorKeyRows: []operatorKeyRow{{OperatorKey: active, Own: true}},
				OperatorKeyStatus: "active", OperatorKeyDirPath: "/opt/keys.d"},
			must: []string{"/operator-keys/opk-2/revoke"},
		},
		{
			name: "an unconfigured directory is stated, not silently ignored",
			vars: &pageVars{OperatorKeyRows: []operatorKeyRow{{OperatorKey: requested}},
				OperatorKeyStatus: onboardstore.OperatorKeyRequested},
			must: []string{"No key directory is configured", "approving one would grant nothing"},
		},
		{
			name: "drift is split three ways, and not-ours is not alarming",
			vars: &pageVars{OperatorKeyStatus: "all", OperatorKeyDirPath: "/opt/keys.d",
				OperatorKeyStale:     []string{"ghost"},
				OperatorKeyUnmanaged: []string{"onboard-runner"},
				OperatorKeyMissing:   []string{"ada"}},
			must: []string{
				"Revoked here, still granted", "ghost",
				"Not issued here", "onboard-runner", "never removed",
				"Recorded as live but absent", "the Portal will",
			},
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			tpl, err := base.Clone()
			if err != nil {
				t.Fatal(err)
			}
			data, err := fs.ReadFile(web.FS, "templates/operator_keys.html")
			if err != nil {
				t.Fatalf("read template: %v", err)
			}
			if _, err := tpl.New("operator_keys").Parse(string(data)); err != nil {
				t.Fatalf("parse: %v", err)
			}
			var sb strings.Builder
			if err := tpl.ExecuteTemplate(&sb, "layout", tc.vars); err != nil {
				t.Fatalf("execute: %v", err)
			}
			out := sb.String()
			for _, want := range tc.must {
				if !strings.Contains(out, want) {
					t.Errorf("missing %q", want)
				}
			}
			for _, banned := range tc.not {
				if strings.Contains(out, banned) {
					t.Errorf("must not contain %q", banned)
				}
			}
		})
	}
}
