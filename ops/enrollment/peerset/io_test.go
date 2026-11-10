package peerset

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func fixedClock(t time.Time) func() {
	prev := now
	now = func() time.Time { return t }
	return func() { now = prev }
}

func TestLoad_MissingFileReturnsEmptyV2(t *testing.T) {
	ps, err := Load(filepath.Join(t.TempDir(), "absent.json"))
	if err != nil {
		t.Fatalf("Load missing: %v", err)
	}
	if ps == nil || ps.SchemaVersion != SchemaVersion {
		t.Fatalf("missing-file load did not produce empty v2 envelope: %+v", ps)
	}
	if len(ps.Peers) != 0 || len(ps.Relays) != 0 || len(ps.DeprecatedPeers) != 0 {
		t.Fatalf("empty envelope should have no rows; got %+v", ps)
	}
}

func TestLoad_EmptyFileReturnsEmptyV2(t *testing.T) {
	p := filepath.Join(t.TempDir(), "empty.json")
	if err := os.WriteFile(p, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	ps, err := Load(p)
	if err != nil {
		t.Fatalf("Load empty: %v", err)
	}
	if ps.SchemaVersion != SchemaVersion {
		t.Fatalf("schema_version = %d, want %d", ps.SchemaVersion, SchemaVersion)
	}
}

func TestLoad_V1AutoMigratesToV2(t *testing.T) {
	ps, err := Load("testdata/v1-flat.json")
	if err != nil {
		t.Fatalf("Load v1 fixture: %v", err)
	}
	if ps.SchemaVersion != SchemaVersion {
		t.Fatalf("post-migrate schema_version = %d, want %d", ps.SchemaVersion, SchemaVersion)
	}
	if len(ps.Peers) != 11 {
		t.Fatalf("post-migrate peer count = %d, want 11", len(ps.Peers))
	}
	for i, p := range ps.Peers {
		if p.Name == "" || p.CurrentPubkey == "" || p.WGCP0Address == "" {
			t.Fatalf("peer %d empty required field: %+v", i, p)
		}
	}
	// Specific peers (from CLAUDE.md canonical fleet) survived.
	wants := map[string]string{
		"lab-a":      "198.18.0.1",
		"prov-conus": "198.18.0.8",
		"op-laptop":  "198.18.0.10",
	}
	for name, addr := range wants {
		p, ok := ps.Lookup(name)
		if !ok {
			t.Fatalf("expected peer %q in migrated set", name)
		}
		if p.WGCP0Address != addr {
			t.Fatalf("%s wg_cp0_address = %s, want %s", name, p.WGCP0Address, addr)
		}
	}
}

func TestLoad_V1MalformedReturnsMigrationFailed(t *testing.T) {
	p := filepath.Join(t.TempDir(), "bad-v1.json")
	if err := os.WriteFile(p, []byte(`[{"name": "broken-no-pubkey", "address": "198.18.0.1"}]`), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := Load(p)
	if !errors.Is(err, ErrMigrationFailed) {
		t.Fatalf("err = %v, want ErrMigrationFailed", err)
	}
}

func TestLoad_V2RoundTripIsByteStable(t *testing.T) {
	t.Cleanup(fixedClock(time.Date(2026, 5, 14, 12, 0, 0, 0, time.UTC)))

	// Start from a freshly-migrated v1 file, save, reload, save again,
	// assert the two saved outputs are byte-identical. This is the
	// acceptance criterion for "byte-stable v2 output across two
	// Load → Save cycles."
	ps1, err := Load("testdata/v1-flat.json")
	if err != nil {
		t.Fatalf("first load: %v", err)
	}
	dir := t.TempDir()
	out1 := filepath.Join(dir, "first.json")
	if err := ps1.Save(out1); err != nil {
		t.Fatalf("first save: %v", err)
	}

	ps2, err := Load(out1)
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	out2 := filepath.Join(dir, "second.json")
	if err := ps2.Save(out2); err != nil {
		t.Fatalf("second save: %v", err)
	}

	b1, err := os.ReadFile(out1)
	if err != nil {
		t.Fatal(err)
	}
	b2, err := os.ReadFile(out2)
	if err != nil {
		t.Fatal(err)
	}
	if string(b1) != string(b2) {
		t.Fatalf("Load → Save round-trip not byte-stable:\nfirst:\n%s\nsecond:\n%s", b1, b2)
	}

	// Reload the saved v2 and re-verify the canonical fleet survived.
	final, err := Load(out2)
	if err != nil {
		t.Fatalf("final load: %v", err)
	}
	if len(final.Peers) != 11 {
		t.Fatalf("post round-trip peer count = %d, want 11", len(final.Peers))
	}
}

func TestSave_AtomicAndMode0644(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "envelope.json")
	ps := emptyV2()
	if err := ps.Save(target); err != nil {
		t.Fatalf("Save: %v", err)
	}
	info, err := os.Stat(target)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o644 {
		t.Fatalf("mode = %v, want 0644", info.Mode().Perm())
	}
	// No stray .tmp left behind.
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		if e.Name() == filepath.Base(target) {
			continue
		}
		t.Fatalf("unexpected sibling file %s after Save", e.Name())
	}
}

func TestLoad_V2WithSchemaVersionAboveSupportedFails(t *testing.T) {
	doc := PeerSet{SchemaVersion: SchemaVersion + 1, Metadata: Metadata{SchemaMinVersion: SchemaVersion + 1}}
	data, _ := json.Marshal(&doc)
	p := filepath.Join(t.TempDir(), "future.json")
	_ = os.WriteFile(p, data, 0o644)
	_, err := Load(p)
	if !errors.Is(err, ErrSchemaInvalid) {
		t.Fatalf("err = %v, want ErrSchemaInvalid", err)
	}
}

func TestLoad_V2MissingSchemaVersionFails(t *testing.T) {
	p := filepath.Join(t.TempDir(), "noversion.json")
	if err := os.WriteFile(p, []byte(`{"peers": []}`), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := Load(p)
	if !errors.Is(err, ErrSchemaInvalid) {
		t.Fatalf("err = %v, want ErrSchemaInvalid", err)
	}
}

func TestLoad_GarbageLeadingByteFails(t *testing.T) {
	p := filepath.Join(t.TempDir(), "garbage.json")
	if err := os.WriteFile(p, []byte("not-json"), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := Load(p)
	if !errors.Is(err, ErrSchemaInvalid) {
		t.Fatalf("err = %v, want ErrSchemaInvalid", err)
	}
}

func TestLoad_UnknownFieldRejected(t *testing.T) {
	p := filepath.Join(t.TempDir(), "with-unknown.json")
	body := `{"schema_version": 2, "peers": [], "relays": [], "deprecated_peers": [], "mystery_field": 1}`
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := Load(p)
	if !errors.Is(err, ErrSchemaInvalid) {
		t.Fatalf("err = %v, want ErrSchemaInvalid (unknown-field rejection)", err)
	}
}
