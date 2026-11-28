package acl

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

const (
	anchorA = "0123456789abcdef0123456789abcdef"
	anchorB = "fedcba9876543210fedcba9876543210"
	anchorC = "00000000000000000000000000000000"
)

// The fragment declares a subject for every pool slot, gives each assigned
// slot exactly its current CN, and gives a free (or quarantined) slot no CN.
func TestRenderSlotFragment(t *testing.T) {
	frag, digest, err := RenderSlotFragment([]SlotBinding{
		{Anchor: anchorA, CN: anchorA + ".g3"},
		{Anchor: anchorB},
		{Anchor: anchorC, CN: anchorC},
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"      {\n        id: \"slot-" + anchorA + "\",\n        cert_common_names: [\"" + anchorA + ".g3\"],\n      },\n",
		"      {\n        id: \"slot-" + anchorB + "\",\n        cert_common_names: [\"" + anchorB + ".unassigned\"],\n      },\n",
		"      {\n        id: \"slot-" + anchorC + "\",\n        cert_common_names: [\"" + anchorC + "\"],\n      },\n",
	} {
		if !strings.Contains(frag, want) {
			t.Errorf("fragment lacks\n%s\ngot\n%s", want, frag)
		}
	}
	if strings.Count(frag, `id: "slot-`) != 3 || strings.Count(frag, `id: "carry-`) != 3 || strings.Count(frag, `"`+anchorA+`.g3"`) != 1 {
		t.Errorf("one subject per slot (no co-publish targets) and one CN per assigned slot, got\n%s", frag)
	}
	withCopub, _, err := RenderSlotFragment([]SlotBinding{
		{Anchor: anchorA, CN: anchorA + ".g3", Copub: []CopubSubject{{Key: "1c342d414d73458d", Live: true}, {Key: "09e1cd2ed2bce389"}}},
		{Anchor: anchorB, Copub: []CopubSubject{{Key: "1c342d414d73458d", Live: true}}},
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"id: \"copub-" + anchorA + "-1c342d414d73458d\",\n        cert_common_names: [\"" + anchorA + ".g3\"],",
		"id: \"copub-" + anchorA + "-09e1cd2ed2bce389\",\n        cert_common_names: [\"" + anchorA + ".unassigned\"],",
		"id: \"copub-" + anchorB + "-1c342d414d73458d\",\n        cert_common_names: [\"" + anchorB + ".unassigned\"],",
	} {
		if !strings.Contains(withCopub, want) {
			t.Errorf("per-topic copub subject missing: %s\n%s", want, withCopub)
		}
	}
	if _, _, err := RenderSlotFragment([]SlotBinding{{Anchor: anchorA, Copub: []CopubSubject{{Key: "../x"}}}}); err == nil {
		t.Error("a non-hex co-publish key was accepted")
	}
	got, err := SlotFragmentDigest(frag)
	if err != nil || got != digest {
		t.Fatalf("SlotFragmentDigest = (%q, %v), want %q", got, err, digest)
	}
	if !strings.Contains(frag, "(2 of 3 slots assigned") {
		t.Errorf("digest line does not count assignments: %q", strings.SplitN(frag, "\n", 2)[0])
	}
	// Any edit to the body breaks the digest.
	tampered := strings.Replace(frag, `cert_common_names: ["`+anchorB+`.unassigned"],`, `cert_common_names: ["`+anchorB+`"],`, 1)
	if _, err := SlotFragmentDigest(tampered); err == nil {
		t.Fatal("a tampered fragment passed its digest check")
	}
}

// Anything that is not a slot anchor or a leaf CN of that same slot is
// refused, so registry content can never inject router config.
func TestRenderSlotFragment_refusesNonSlotInput(t *testing.T) {
	for _, bad := range [][]SlotBinding{
		{{Anchor: "not-an-anchor"}},
		{{Anchor: anchorA, CN: anchorB + ".g1"}},
		{{Anchor: anchorA, CN: anchorA + ".g0"}},
		{{Anchor: anchorA, CN: anchorA + `"], }, { id: "x`}},
		{{Anchor: anchorA, CN: "operator-trial-peer"}},
		{{Anchor: anchorA}, {Anchor: anchorA}},
	} {
		if _, _, err := RenderSlotFragment(bad); err == nil {
			t.Errorf("RenderSlotFragment(%+v) accepted", bad)
		}
	}
}

func TestSpliceSlotFragment(t *testing.T) {
	cfg := "subjects: [\n      { id: \"operator\" },\n" + SlotFragmentBegin + " (portal)\n      old\n" + SlotFragmentEnd + "\n    ],\n"
	out, err := SpliceSlotFragment(cfg, "      new\n")
	if err != nil {
		t.Fatal(err)
	}
	want := "subjects: [\n      { id: \"operator\" },\n" + SlotFragmentBegin + " (portal)\n      new\n" + SlotFragmentEnd + "\n    ],\n"
	if out != want {
		t.Fatalf("splice =\n%s\nwant\n%s", out, want)
	}
	if _, err := SpliceSlotFragment("{}", "x"); !errors.Is(err, ErrNoSlotMarkers) {
		t.Fatalf("no markers: err = %v", err)
	}
	if _, err := SpliceSlotFragment(cfg+cfg, "x"); err == nil {
		t.Fatal("two regions accepted")
	}
}

// ApplySlotFragment writes the fragment beside the config and into it, and
// leaves an unchanged config alone.
func TestApplySlotFragment(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "config.json5")
	seed := "{\n    subjects: [\n" + SlotFragmentBegin + "\n" + SlotFragmentEnd + "\n    ],\n}\n"
	if err := os.WriteFile(cfgPath, []byte(seed), 0o644); err != nil {
		t.Fatal(err)
	}
	r := &Renderer{ConfigPath: cfgPath, ReloadFake: true}
	res, err := r.ApplySlotFragment(context.Background(), []SlotBinding{{Anchor: anchorA, CN: anchorA + ".g1"}, {Anchor: anchorB}})
	if err != nil || !res.ConfigChanged {
		t.Fatalf("apply = (%+v, %v)", res, err)
	}
	live, _ := os.ReadFile(cfgPath)
	file, err := os.ReadFile(filepath.Join(dir, SlotFragmentFile))
	if err != nil {
		t.Fatalf("fragment file: %v", err)
	}
	if !strings.Contains(string(live), string(file)) || !strings.Contains(string(live), `["`+anchorA+`.g1"]`) {
		t.Fatalf("live config does not carry the fragment:\n%s", live)
	}
	res, err = r.ApplySlotFragment(context.Background(), []SlotBinding{{Anchor: anchorA, CN: anchorA + ".g1"}, {Anchor: anchorB}})
	if err != nil || res.ConfigChanged {
		t.Fatalf("re-apply of the same state = (%+v, %v), want unchanged", res, err)
	}
	// Revoke: the CN goes.
	if _, err := r.ApplySlotFragment(context.Background(), []SlotBinding{{Anchor: anchorA}, {Anchor: anchorB}}); err != nil {
		t.Fatal(err)
	}
	live, _ = os.ReadFile(cfgPath)
	if strings.Contains(string(live), `"`+anchorA+`"`) || strings.Contains(string(live), `"`+anchorA+`.g`) {
		t.Fatalf("revoked slot still has a leaf CN:\n%s", live)
	}

	// A config without markers still gets the fragment file, and says so.
	bare := filepath.Join(t.TempDir(), "config.json5")
	_ = os.WriteFile(bare, []byte("{}"), 0o644)
	r2 := &Renderer{ConfigPath: bare, ReloadFake: true}
	if _, err := r2.ApplySlotFragment(context.Background(), []SlotBinding{{Anchor: anchorA}}); !errors.Is(err, ErrNoSlotMarkers) {
		t.Fatalf("no markers: err = %v", err)
	}
	if _, err := os.Stat(filepath.Join(filepath.Dir(bare), SlotFragmentFile)); err != nil {
		t.Fatalf("fragment file not written without markers: %v", err)
	}
	if b, _ := os.ReadFile(bare); string(b) != "{}" {
		t.Fatal("a config without markers was rewritten")
	}
}

// GHSA-rr3p review r8 F4: the bindings are read while the renderer's lock is
// held, so a second render cannot run between a render's read and its write.
func TestApplySlotFragmentFrom_readsUnderTheLock(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(cfgPath, []byte(SlotFragmentBegin+"\n"+SlotFragmentEnd+"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := &Renderer{ConfigPath: cfgPath, ReloadFake: true}
	done := make(chan struct{})
	_, err := r.ApplySlotFragmentFrom(context.Background(), func(context.Context) ([]SlotBinding, error) {
		// A competing render started during this read must wait for this one.
		go func() {
			_, _ = r.ApplySlotFragment(context.Background(), []SlotBinding{{Anchor: anchorA}})
			close(done)
		}()
		select {
		case <-done:
			t.Error("a second render ran between this render's read and its write")
		case <-time.After(200 * time.Millisecond):
		}
		return []SlotBinding{{Anchor: anchorA, CN: anchorA + ".g1"}}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	<-done // the competing (later) render lands after, in order
	b, _ := os.ReadFile(cfgPath)
	if strings.Contains(string(b), anchorA+".g1") {
		t.Fatal("the render that started later did not land last")
	}
}

// ADR 1251 phase 4: carry-<anchor> names the other slots' leaves the core
// admits as writers of a slot; it never names the slot's own leaf and is the
// unassigned CN for a free slot or one nobody carries.
func TestRenderSlotFragment_carry(t *testing.T) {
	pod := anchorB + ".g2"
	frag, _, err := RenderSlotFragment([]SlotBinding{
		{Anchor: anchorA, CN: anchorA + ".g3", Carry: []string{pod}},
		{Anchor: anchorB, CN: pod},
		{Anchor: anchorC, Carry: []string{pod}}, // free: a carrier list is ignored
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"id: \"carry-" + anchorA + "\",\n        cert_common_names: [\"" + pod + "\"],",
		"id: \"carry-" + anchorB + "\",\n        cert_common_names: [\"" + anchorB + ".unassigned\"],",
		"id: \"carry-" + anchorC + "\",\n        cert_common_names: [\"" + anchorC + ".unassigned\"],",
		"1 carried",
	} {
		if !strings.Contains(frag, want) {
			t.Errorf("missing %q in\n%s", want, frag)
		}
	}
	for name, bad := range map[string][]string{
		"own CN":     {anchorA + ".g3"},
		"not a leaf": {"pod-1"},
		"injection":  {anchorB + `"], x: ["`},
		"unsorted":   {anchorB, anchorC},
		"repeated":   {anchorB, anchorB},
	} {
		if _, _, err := RenderSlotFragment([]SlotBinding{{Anchor: anchorA, CN: anchorA + ".g3", Carry: bad}}); err == nil {
			t.Errorf("%s carrier list accepted: %v", name, bad)
		}
	}
}

// A carrier's CN in carry-<anchor> does not make that carrier's own leaf read
// as loaded: only slot- and copub- subjects say that.
func TestLoadedLeafCNs_ignoresCarriers(t *testing.T) {
	dir := t.TempDir()
	frag, _, err := RenderSlotFragment([]SlotBinding{
		{Anchor: anchorA, CN: anchorA + ".g3", Carry: []string{anchorB + ".g7"}},
		{Anchor: anchorB, CN: anchorB + ".g2"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, SlotFragmentLoadedFile), []byte(frag), 0o644); err != nil {
		t.Fatal(err)
	}
	got, ok := (&Renderer{ConfigPath: filepath.Join(dir, "config.json5")}).LoadedLeafCNs()
	if !ok || !got[anchorA+".g3"] || !got[anchorB+".g2"] || got[anchorB+".g7"] {
		t.Fatalf("loaded CNs = %v (ok %v)", got, ok)
	}
}
