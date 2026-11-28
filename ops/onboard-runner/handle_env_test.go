package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The same person onboarded on one goatnet and then submitted for another is a
// new enrollment on the second: the first goatnet's setup key and bundle must
// not be handed out for it.
func TestRunOnce_DuplicateEmailOnAnotherEnvMints(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-a", Env: "env-a", Handle: "alice", Email: "alice@example.org",
		Status: "onboarded", SetupKey: "ENV-A-KEY", BundlePath: "/api/org-bundle/a/tok.tar.gz"})
	f.add(proposal{ID: "prop-b", Env: "env-b", Handle: "alice", Email: "alice@example.org",
		Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	minted := 0
	r.mint = func(context.Context, proposal, string) (mintResult, error) {
		minted++
		return mintResult{SetupKey: "ENV-B-KEY"}, nil
	}

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if minted != 1 {
		t.Fatalf("minted %d on env-b, want 1 (env-a's onboarding is not a duplicate)", minted)
	}
	if f.resultKeys["prop-b"] == "ENV-A-KEY" || f.resultBun["prop-b"] == "/api/org-bundle/a/tok.tar.gz" {
		t.Fatalf("env-b proposal finalized with env-a's setup key / bundle (key %q, bundle %q)",
			f.resultKeys["prop-b"], f.resultBun["prop-b"])
	}
	if f.results["prop-b"] != "onboarded" {
		t.Fatalf("result = %q, want onboarded", f.results["prop-b"])
	}
}

// A handle or env that is not a slug never reaches a path or a goat argument:
// the proposal is failed, nothing is written outside ops/operators, nothing is
// minted.
func TestRunOnce_RefusesHandleOrEnvThatIsNotASlug(t *testing.T) {
	cases := []proposal{
		{Env: "zagros", Handle: "../../escape"},
		{Env: "../../escape", Handle: "jane"},
		{Env: "zagros", Handle: "--repo-root=/tmp"},
		{Env: "zagros", Handle: "Jane"},
		{Env: "zagros", Handle: ""},
		{Env: "zagros", Handle: "a/b"},
		{Env: "Zagros", Handle: "jane"},
	}
	for _, c := range cases {
		t.Run(c.Env+"|"+c.Handle, func(t *testing.T) {
			f := newFakePortal()
			c.ID, c.Status, c.Email, c.ManifestJSON = "prop-1", "approved", "j@example.org", testManifest
			f.add(c)
			r, _ := testRunner(t, f)
			r.repoRoot = filepath.Join(t.TempDir(), "repo")
			minted := 0
			r.mint = func(context.Context, proposal, string) (mintResult, error) { minted++; return mintResult{}, nil }

			if err := r.runOnce(context.Background()); err != nil {
				t.Fatal(err)
			}
			if minted != 0 {
				t.Errorf("minted %d for handle %q env %q, want 0", minted, c.Handle, c.Env)
			}
			if f.results["prop-1"] != "failed" {
				t.Errorf("result = %q, want failed", f.results["prop-1"])
			}
			allowed := filepath.Join(r.repoRoot, "ops", "operators")
			_ = filepath.Walk(filepath.Dir(r.repoRoot), func(path string, info os.FileInfo, err error) error {
				if err == nil && !info.IsDir() && !strings.HasPrefix(path, allowed+string(filepath.Separator)) {
					t.Errorf("file written outside ops/operators: %s", path)
				}
				return nil
			})
		})
	}
}

func TestRunOnce_OffboardRefusesHandleThatIsNotASlug(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "--purge", Status: "offboard-requested"})
	r, _ := testRunner(t, f)
	revoked := 0
	r.revoke = func(context.Context, proposal) error { revoked++; return nil }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if revoked != 0 {
		t.Errorf("revoke ran %d times for handle --purge, want 0", revoked)
	}
	if f.results["prop-1"] != "offboard-failed" {
		t.Errorf("result = %q, want offboard-failed", f.results["prop-1"])
	}
}

func TestValidSlug(t *testing.T) {
	for _, ok := range []string{"a", "jane", "ada-lovelace", "zagros-prod", "user8x-example", "0abc"} {
		if !validSlug(ok) {
			t.Errorf("validSlug(%q) = false, want true", ok)
		}
	}
	long := "a"
	for len(long) < 64 {
		long += "b"
	}
	for _, bad := range []string{"", "-a", "A", "a_b", "a.b", "a/b", "..", "a\n", long} {
		if validSlug(bad) {
			t.Errorf("validSlug(%q) = true, want false", bad)
		}
	}
}

func TestRunOnce_ReissueRefusesHandleThatIsNotASlug(t *testing.T) {
	f := newFakePortal()
	q := newFakeActionQueue()
	q.add(action{ID: "act-1", ProposalID: "prop-1", Kind: actionReissueDevice, Status: "queued", Handle: "--revoke", Env: "zagros"})
	r := testRunnerWithActions(t, f, q)
	ran := 0
	r.reissue = func(context.Context, action) ([]byte, error) { ran++; return []byte(reissueOutput), nil }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if ran != 0 {
		t.Errorf("reissue ran %d times for handle --revoke, want 0", ran)
	}
	if res := q.results["act-1"]; res.Status != "failed" {
		t.Errorf("action result = %+v, want failed", res)
	}
}
