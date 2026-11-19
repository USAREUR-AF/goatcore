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

func src(slot, id, kind string, until *time.Time) generation.Source {
	return generation.Source{Slot: slot, RuleID: id, Kind: kind, Until: until,
		Principals: []string{slot + ".g1"}, Write: generation.DefaultWrite(slot)}
}

func gen(issued time.Time, s ...generation.Source) *generation.Generation {
	return &generation.Generation{Generation: 1, IssuedAt: issued, Sources: s}
}

func slots(r Result) []string {
	var out []string
	for _, s := range r.Effective {
		out = append(out, s.Slot)
	}
	return out
}

func state(t *testing.T, r Result, id string) string {
	t.Helper()
	rule, ok := r.Find(id)
	if !ok {
		t.Fatalf("rule %s missing", id)
	}
	return rule.State
}

func TestCentralOnly(t *testing.T) {
	r := Compute(Inputs{Now: now, Gen: gen(now.Add(-time.Hour),
		src(srcA, "cr_a", "org", nil),
		src(srcB, "cr_b", "named", at(-1)), // expired
	)})
	if got := slots(r); len(got) != 1 || got[0] != srcA {
		t.Fatalf("effective %v", got)
	}
	if state(t, r, "cr_b") != StateExpired {
		t.Fatal("expired rule not marked")
	}
}

func TestLocalAllowRevokeReactivate(t *testing.T) {
	g := gen(now.Add(-time.Hour), src(srcA, "cr_a", "org", nil))
	led := []ledger.Entry{
		{Seq: 1, Action: ledger.ActionAllow, RuleID: "lr_1", Source: srcC, Principals: []string{srcC + ".g2"},
			Write: generation.DefaultWrite(srcC), Until: *at(24), Reason: "field"},
	}
	r := Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if got := slots(r); len(got) != 2 {
		t.Fatalf("allow: effective %v", got)
	}
	led = append(led, ledger.Entry{Seq: 2, Action: ledger.ActionRevoke, Source: srcA, Until: *at(4), Reason: "lost"})
	r = Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if got := slots(r); len(got) != 1 || got[0] != srcC {
		t.Fatalf("revoke central: effective %v", got)
	}
	if state(t, r, "cr_a") != StateRevoked || len(r.Revocations) != 1 {
		t.Fatal("revocation not reported")
	}
	// After the revoke's end, the central rule applies again.
	r = Compute(Inputs{Now: now.Add(5 * time.Hour), Gen: g, Ledger: led})
	if state(t, r, "cr_a") != StateActive {
		t.Fatal("revoke did not end")
	}
	// Revoking a local rule's source, then reactivating the rule.
	led = append(led,
		ledger.Entry{Seq: 3, Action: ledger.ActionRevoke, Source: srcC, Until: *at(48), Reason: "x"},
	)
	r = Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if state(t, r, "lr_1") != StateRevoked {
		t.Fatal("local rule not revoked")
	}
	led = append(led, ledger.Entry{Seq: 4, Action: ledger.ActionReactivate, RuleID: "lr_1", Source: srcC, Until: *at(12), Reason: "y"})
	r = Compute(Inputs{Now: now, Gen: g, Ledger: led})
	if state(t, r, "lr_1") != StateActive {
		t.Fatal("reactivate did not restore the local rule")
	}
	// Expiry.
	r = Compute(Inputs{Now: now.Add(13 * time.Hour), Gen: g, Ledger: led})
	if state(t, r, "lr_1") != StateExpired {
		t.Fatal("local rule did not expire")
	}
}

func TestExtendAndReactivateCentral(t *testing.T) {
	prev := gen(now.Add(-48*time.Hour), src(srcA, "cr_a", "named", at(-2)), src(srcB, "cr_b", "named", at(2)))
	next := gen(now.Add(-time.Hour), src(srcB, "cr_b", "named", at(2)))
	archive := NextArchive(nil, prev, next, now.Add(-time.Hour), 183*24*time.Hour)
	if _, ok := archive["cr_a"]; !ok || len(archive) != 1 {
		t.Fatalf("archive %v", archive)
	}
	led := []ledger.Entry{
		{Seq: 1, Action: ledger.ActionExtend, RuleID: "cr_b", Source: srcB, Until: *at(30), Reason: "longer"},
		{Seq: 2, Action: ledger.ActionReactivate, RuleID: "cr_a", Source: srcA, Until: *at(10), Reason: "back"},
	}
	r := Compute(Inputs{Now: now.Add(5 * time.Hour), Gen: next, Archive: archive, Ledger: led})
	if got := slots(r); len(got) != 2 {
		t.Fatalf("effective %v", got)
	}
	rb, _ := r.Find("cr_b")
	if rb.LocalUntil == nil || !rb.LocalUntil.Equal(*at(30)) || !rb.Pending {
		t.Fatalf("extension not shown: %+v", rb)
	}
	r = Compute(Inputs{Now: now, Gen: next, Archive: archive})
	if state(t, r, "cr_a") != StateArchive {
		t.Fatal("archived rule not listed")
	}
}

func TestSettledAfterAck(t *testing.T) {
	led := []ledger.Entry{
		{Seq: 1, Action: ledger.ActionAllow, RuleID: "lr_1", Source: srcC, Principals: []string{srcC + ".g1"},
			Write: generation.DefaultWrite(srcC), Until: *at(24), Reason: "field"},
	}
	ack := ledger.Ack{LastSeq: 1, At: now.Add(-30 * time.Minute), Generation: 1}
	// The generation held is the one the pod held at acknowledgment: the local rule still applies.
	r := Compute(Inputs{Now: now, Gen: gen(now.Add(-time.Hour)), Ledger: led, Ack: ack})
	if len(r.Effective) != 1 {
		t.Fatal("local rule dropped before the portal decided")
	}
	if rl, _ := r.Find("lr_1"); rl.Pending {
		t.Fatal("acknowledged rule still pending")
	}
	// A later generation decides: here the portal let it lapse.
	later := gen(now)
	later.Generation = 2
	r = Compute(Inputs{Now: now, Gen: later, Ledger: led, Ack: ack})
	if len(r.Effective) != 0 || state(t, r, "lr_1") != StateSettled {
		t.Fatal("lapsed local rule still carried after the portal decided")
	}
}

func TestMergeSameSlot(t *testing.T) {
	led := []ledger.Entry{
		{Seq: 1, Action: ledger.ActionAllow, RuleID: "lr_1", Source: srcA, Principals: []string{"adapter.g1"},
			Write: []string{srcA + "/stream/**"}, Until: *at(24), Reason: "adapter"},
	}
	r := Compute(Inputs{Now: now, Gen: gen(now.Add(-time.Hour), src(srcA, "cr_a", "org", nil)), Ledger: led})
	if len(r.Effective) != 1 || len(r.Effective[0].Principals) != 2 || r.Effective[0].Until != nil {
		t.Fatalf("merge: %+v", r.Effective)
	}
}
