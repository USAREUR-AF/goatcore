package rules

import (
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/ledger"
)

var (
	now  = time.Date(2026, 11, 19, 12, 0, 0, 0, time.UTC)
	srcA = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	srcB = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	srcC = "cccccccccccccccccccccccccccccccc"
)

func at(h int) *time.Time { t := now.Add(time.Duration(h) * time.Hour); return &t }

func rfc(h int) string { return at(h).Format(time.RFC3339) }

func src(slot, id, kind string, until *time.Time) generation.Source {
	return generation.Source{Slot: slot, RuleID: id, Kind: kind, Until: until,
		Principals: []string{slot + ".g1"}, Write: generation.DefaultWrite(slot)}
}

func gen(n int64, s ...generation.Source) *generation.Generation {
	return &generation.Generation{Generation: n, IssuedAt: now.Add(-time.Hour), Sources: s}
}

func rec(seq int64, action, rule, slot, until string, local ledger.Local) ledger.Record {
	return ledger.Record{Entry: ledger.Entry{Seq: seq, Action: action, RuleID: rule, SourceSlot: slot, Until: until,
		Reason: "r"}, Local: local}
}

func allowC(seq int64, rule string, h int) ledger.Record {
	return rec(seq, ledger.ActionAllow, rule, srcC, rfc(h),
		ledger.Local{Principals: []string{srcC + ".g2"}, Write: generation.DefaultWrite(srcC)})
}

func slots(r Result) []string {
	var out []string
	for _, s := range r.Effective {
		out = append(out, s.Slot)
	}
	return out
}

func find(t *testing.T, r Result, id string) Rule {
	t.Helper()
	rule, ok := r.Find(id)
	if !ok {
		t.Fatalf("rule %s missing", id)
	}
	return rule
}

func TestCentralOnly(t *testing.T) {
	r := Compute(Inputs{Now: now, Gen: gen(1,
		src(srcA, "cr_a", "org", nil),
		src(srcB, "cr_b", "named", at(-1)), // expired
	)})
	if got := slots(r); len(got) != 1 || got[0] != srcA {
		t.Fatalf("effective %v", got)
	}
	if find(t, r, "cr_b").State != StateExpired || find(t, r, "cr_a").Ratification != nil {
		t.Fatal("central rule states wrong")
	}
}

func TestLocalAllowRevokeReactivate(t *testing.T) {
	g := gen(1, src(srcA, "cr_a", "org", nil))
	led := []ledger.Record{allowC(1, "lr_1", 24)}
	r := Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if got := slots(r); len(got) != 2 || *find(t, r, "lr_1").Ratification != RatPending {
		t.Fatalf("allow: effective %v", got)
	}
	// A revoke of a central source holds until lifted or decided.
	led = append(led, rec(2, ledger.ActionRevoke, "", srcA, "", ledger.Local{}))
	r = Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if got := slots(r); len(got) != 1 || got[0] != srcC {
		t.Fatalf("revoke central: effective %v", got)
	}
	if find(t, r, "cr_a").State != StateRevoked || len(r.Revocations) != 1 {
		t.Fatal("revocation not reported")
	}
	r = Compute(Inputs{Now: now.Add(500 * time.Hour), Gen: g, Ledger: led})
	if find(t, r, "cr_a").State != StateRevoked {
		t.Fatal("revoke ended by itself")
	}
	led = append(led, rec(3, ledger.ActionRevoke, "", srcC, "", ledger.Local{}))
	r = Compute(Inputs{Now: now.Add(40 * time.Hour), Gen: g, Ledger: led})
	if find(t, r, "lr_1").State != StateRevoked {
		t.Fatal("local rule not revoked")
	}
	led = append(led, rec(4, ledger.ActionReactivate, "lr_1", srcC, rfc(12), ledger.Local{}))
	r = Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if find(t, r, "lr_1").State != StateActive {
		t.Fatal("reactivate did not restore the local rule")
	}
	r = Compute(Inputs{Now: now.Add(13 * time.Hour), Gen: g, Ledger: led})
	if find(t, r, "lr_1").State != StateExpired {
		t.Fatal("local rule did not expire")
	}
}

func TestExtendAndReactivateCentral(t *testing.T) {
	prev := gen(1, src(srcA, "cr_a", "named", at(-2)), src(srcB, "cr_b", "named", at(2)))
	next := gen(2, src(srcB, "cr_b", "named", at(2)))
	archive := NextArchive(nil, prev, next, now.Add(-time.Hour), 183*24*time.Hour)
	if _, ok := archive["cr_a"]; !ok || len(archive) != 1 {
		t.Fatalf("archive %v", archive)
	}
	led := []ledger.Record{
		rec(1, ledger.ActionExtend, "cr_b", srcB, rfc(30), ledger.Local{}),
		rec(2, ledger.ActionReactivate, "cr_a", srcA, rfc(10), ledger.Local{}),
	}
	r := Compute(Inputs{Now: now.Add(5 * time.Hour), Gen: next, Archive: archive, Ledger: led})
	if got := slots(r); len(got) != 2 {
		t.Fatalf("effective %v", got)
	}
	rb := find(t, r, "cr_b")
	if rb.LocalUntil == nil || !rb.LocalUntil.Equal(*at(30)) || *rb.Ratification != RatPending {
		t.Fatalf("extension not shown: %+v", rb)
	}
	r = Compute(Inputs{Now: now, Gen: next, Archive: archive})
	if find(t, r, "cr_a").State != StateArchive {
		t.Fatal("archived rule not listed")
	}
}

func TestDecisions(t *testing.T) {
	led := []ledger.Record{
		allowC(1, "lr_1", 24),
		rec(2, ledger.ActionRevoke, "", srcA, "", ledger.Local{}),
		rec(3, ledger.ActionAllow, "lr_3", srcB, rfc(24),
			ledger.Local{Principals: []string{srcB + ".g1"}, Write: generation.DefaultWrite(srcB)}),
	}
	// Ratified allow lr_1 arrives as a central named rule; lapsed revoke lifts; lapsed allow drops.
	g := gen(2, src(srcA, "cr_a", "org", nil), src(srcC, "cr_c", "named", at(48)))
	g.LocalDecisions = []generation.Decision{
		{Seq: 1, LocalRuleID: "lr_1", State: "ratified"},
		{Seq: 2, State: "lapsed"},
		{Seq: 3, LocalRuleID: "lr_3", State: "lapsed"},
	}
	r := Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if got := slots(r); len(got) != 2 || got[0] != srcA || got[1] != srcC {
		t.Fatalf("effective after decisions %v", got)
	}
	l1, l3 := find(t, r, "lr_1"), find(t, r, "lr_3")
	if *l1.Ratification != RatRatified || l1.State != StateArchive || *l3.Ratification != RatLapsed {
		t.Fatalf("decisions not shown: %+v %+v", l1, l3)
	}
	if len(r.Revocations) != 0 {
		t.Fatal("lapsed revoke still in force")
	}
}

func TestMergeSameSlot(t *testing.T) {
	led := []ledger.Record{rec(1, ledger.ActionAllow, "lr_1", srcA, rfc(24),
		ledger.Local{Principals: []string{"adapter.g1"}, Write: []string{srcA + "/stream/**"}})}
	r := Compute(Inputs{Now: now, Gen: gen(1, src(srcA, "cr_a", "org", nil)), Ledger: led})
	if len(r.Effective) != 1 || len(r.Effective[0].Principals) != 2 || r.Effective[0].Until != nil {
		t.Fatalf("merge: %+v", r.Effective)
	}
}

// A decision on a local revoke, either way, lifts its block from central rules only: what was
// granted locally to the source before the revoke stays cut off until a later local allow or
// reactivate (pod-carriage-and-provenance.md section 7.4).
func TestDecidedRevokeKeepsEarlierLocalGrantsCut(t *testing.T) {
	revoke := func(seq int64, slot string) ledger.Record {
		return rec(seq, ledger.ActionRevoke, "", slot, "", ledger.Local{})
	}
	central := src(srcC, "cr_c", "named", at(2))
	for _, state := range []string{RatLapsed, RatRatified} {
		decided := func(n int64, sources ...generation.Source) *generation.Generation {
			g := gen(2, sources...)
			g.LocalDecisions = []generation.Decision{{Seq: n, State: state}}
			return g
		}
		check := func(name string, g *generation.Generation, led []ledger.Record, h int, want bool) Result {
			t.Helper()
			r := Compute(Inputs{Now: *at(h), Gen: g, Ledger: led})
			if got := len(slots(r)) == 1; got != want {
				t.Fatalf("revoke %s, %s: carried %v", state, name, slots(r))
			}
			return r
		}
		led := []ledger.Record{allowC(1, "lr_1", 24), revoke(2, srcC)}
		r := check("earlier allow", decided(2), led, 0, false)
		if l := find(t, r, "lr_1"); l.State != StateRevoked || *l.Ratification != RatPending {
			t.Fatalf("revoke %s: the allow shows as %s", state, l.State)
		}
		check("earlier allow, then extended", decided(3), []ledger.Record{allowC(1, "lr_1", 4),
			rec(2, ledger.ActionExtend, "lr_1", srcC, rfc(24), ledger.Local{}), revoke(3, srcC)}, 0, false)
		check("earlier reactivate of a local rule", decided(4), []ledger.Record{allowC(1, "lr_1", 24), revoke(2, srcC),
			rec(3, ledger.ActionReactivate, "lr_1", srcC, rfc(24), ledger.Local{}), revoke(4, srcC)}, 0, false)
		check("extend after the revoke does not lift it", decided(2),
			append(led[:2:2], rec(3, ledger.ActionExtend, "lr_1", srcC, rfc(30), ledger.Local{})), 0, false)
		check("later allow", decided(2), append(led[:2:2], allowC(3, "lr_3", 24)), 0, true)
		check("later reactivate", decided(2),
			append(led[:2:2], rec(3, ledger.ActionReactivate, "lr_1", srcC, rfc(24), ledger.Local{})), 0, true)
		check("another source's revoke", decided(2), []ledger.Record{allowC(1, "lr_1", 24), revoke(2, srcB)}, 0, true)

		// Central rules are the portal's: the decided revoke does not block them...
		check("central rule", decided(2, central), led, 0, true)
		r = check("central and earlier allow", decided(2, central), []ledger.Record{
			rec(1, ledger.ActionAllow, "lr_1", srcC, rfc(24), ledger.Local{Principals: []string{"other.g1"}, Write: []string{srcC + "/x"}}),
			revoke(2, srcC)}, 0, true)
		if p := r.Effective[0].Principals; len(p) != 1 || p[0] != srcC+".g1" {
			t.Fatalf("revoke %s: the cut-off allow's certificate is carried: %v", state, p)
		}
		// ...but an earlier local extension or reactivation of one is cut off.
		check("earlier extension of a central rule", decided(2, central), []ledger.Record{
			rec(1, ledger.ActionExtend, "cr_c", srcC, rfc(24), ledger.Local{}), revoke(2, srcC)}, 5, false)
		check("another source's revoke and an extension", decided(2, central), []ledger.Record{
			rec(1, ledger.ActionExtend, "cr_c", srcC, rfc(24), ledger.Local{}), revoke(2, srcB)}, 5, true)
		check("later extension of a central rule", decided(1, central), []ledger.Record{revoke(1, srcC),
			rec(2, ledger.ActionExtend, "cr_c", srcC, rfc(24), ledger.Local{})}, 5, true)
		arch := Inputs{Now: now, Gen: decided(2), Archive: map[string]Archived{"cr_c": {Source: central, LastSeen: now}},
			Ledger: []ledger.Record{rec(1, ledger.ActionReactivate, "cr_c", srcC, rfc(24), ledger.Local{}), revoke(2, srcC)}}
		if got := slots(Compute(arch)); len(got) != 0 {
			t.Fatalf("revoke %s: an earlier reactivation is carried: %v", state, got)
		}
	}
}
