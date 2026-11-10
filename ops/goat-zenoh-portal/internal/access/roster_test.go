package access

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadRoster(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "roster.json")
	// A member with a vendor_tag (keys on the tag), one without (falls back to
	// the affiliation org id), and a system.
	body := `{
      "orgs": [
        {"id": "example-vendor", "kind": "vendor", "nation": null, "parent": null, "members": []},
        {"id": "nation-demo-forces", "kind": "organic", "nation": "EST", "parent": null, "members": []}
      ],
      "members": [
        {"handle": "sam-fowler", "type": "person", "org": "example-vendor", "vendor_tag": "example-org-sam", "status": "active"},
        {"handle": "nation-analyst", "type": "person", "org": "nation-demo-forces", "vendor_tag": null, "status": "active"},
        {"handle": "synth-1", "type": "system", "org": "goat", "vendor_tag": "goat-synth", "status": "revoked"}
      ]
    }`
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}

	got, err := LoadRoster(path)
	if err != nil {
		t.Fatalf("LoadRoster: %v", err)
	}
	if len(got) != 3 {
		t.Fatalf("want 3 entries, got %d", len(got))
	}

	// vendor_tag becomes the reconciliation key (Org), matching what a live
	// Kratos identity's traits.vendor.short_tag presents.
	if got[0].Org != "example-org-sam" {
		t.Errorf("member 0 Org = %q, want vendor_tag 'example-org-sam'", got[0].Org)
	}
	// No vendor_tag ⇒ fall back to the affiliation org id.
	if got[1].Org != "nation-demo-forces" {
		t.Errorf("member 1 Org = %q, want fallback 'nation-demo-forces'", got[1].Org)
	}
	if got[2].Type != "system" || got[2].Status != "revoked" {
		t.Errorf("member 2 = %+v, want system/revoked", got[2])
	}

	// OrgID always carries the true affiliation.id — the Phase D aff-claim
	// reconcile key — independent of the vendor_tag substitution in Org.
	if got[0].OrgID != "example-vendor" || got[1].OrgID != "nation-demo-forces" || got[2].OrgID != "goat" {
		t.Errorf("OrgIDs = %q/%q/%q, want example-vendor/nation-demo-forces/goat",
			got[0].OrgID, got[1].OrgID, got[2].OrgID)
	}

	// Affiliation is joined from the org block onto each member (the Go-side
	// join), independent of the vendor_tag reconciliation key.
	if got[0].Affiliation != "vendor" || got[0].Nation != "" {
		t.Errorf("member 0 affiliation = %q/%q, want vendor/<empty>", got[0].Affiliation, got[0].Nation)
	}
	if got[1].Affiliation != "organic" || got[1].Nation != "EST" {
		t.Errorf("member 1 affiliation = %q/%q, want organic/EST", got[1].Affiliation, got[1].Nation)
	}
	// A member whose org has no orgs[] entry carries no affiliation (honest
	// absence — never guessed).
	if got[2].Affiliation != "" || got[2].Nation != "" {
		t.Errorf("member 2 affiliation = %q/%q, want empty (org not in orgs[])", got[2].Affiliation, got[2].Nation)
	}
}

func TestLoadRosterErrors(t *testing.T) {
	if _, err := LoadRoster(filepath.Join(t.TempDir(), "missing.json")); err == nil {
		t.Error("expected error for missing file")
	}
	dir := t.TempDir()
	bad := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(bad, []byte("{not json"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := LoadRoster(bad); err == nil {
		t.Error("expected error for malformed JSON")
	}
}

func TestLoadRosterFreshness(t *testing.T) {
	dir := t.TempDir()

	// Stamped roster (generated_at + source_digest present, two members).
	stamped := filepath.Join(dir, "roster.json")
	body := `{
      "generated_at": "2026-07-01T15:31:40Z",
      "source_digest": "28de1d5f5b7e3c70e8119a47e084cf88",
      "orgs": [],
      "members": [
        {"handle": "a", "type": "person", "org": "x", "status": "active"},
        {"handle": "b", "type": "system", "org": "x", "status": "active"}
      ]
    }`
	if err := os.WriteFile(stamped, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	fr := LoadRosterFreshness(stamped)
	if fr == nil {
		t.Fatal("stamped roster: got nil freshness")
	}
	if fr.GeneratedAt != "2026-07-01T15:31:40Z" || fr.SourceDigest != "28de1d5f5b7e3c70e8119a47e084cf88" {
		t.Errorf("stamp not read: %+v", fr)
	}
	if fr.MemberCount != 2 {
		t.Errorf("MemberCount = %d, want 2", fr.MemberCount)
	}
	if fr.LoadedUnix == 0 {
		t.Errorf("LoadedUnix not set from mtime")
	}

	// Unstamped roster (predates gen-roster.py stamp): valid, but empty
	// provenance — the caller degrades to "age unknown", never a fabricated one.
	unstamped := filepath.Join(dir, "old.json")
	if err := os.WriteFile(unstamped, []byte(`{"orgs":[],"members":[{"handle":"a"}]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	fr = LoadRosterFreshness(unstamped)
	if fr == nil {
		t.Fatal("unstamped roster: got nil, want empty-provenance struct")
	}
	if fr.GeneratedAt != "" || fr.SourceDigest != "" {
		t.Errorf("unstamped should have empty provenance, got %+v", fr)
	}
	if fr.MemberCount != 1 {
		t.Errorf("MemberCount = %d, want 1", fr.MemberCount)
	}

	// Missing file: nil, not an error — freshness is advisory.
	if LoadRosterFreshness(filepath.Join(dir, "nope.json")) != nil {
		t.Error("missing file should yield nil freshness")
	}
}
