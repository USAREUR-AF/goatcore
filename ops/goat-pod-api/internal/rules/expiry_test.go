package rules

import (
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/ledger"
)

func TestNextExpiry(t *testing.T) {
	g := gen(1, src(srcA, "cr_a", "org", nil), src(srcB, "cr_b", "named", at(5)))
	led := []ledger.Record{allowC(1, "lr_1", 3)}
	next := func(h int, l []ledger.Record) (int, bool) {
		n := at(h)
		u, ok := Compute(Inputs{Now: *n, Gen: g, Ledger: l}).NextExpiry(*n)
		return int(u.Sub(now).Hours()), ok
	}
	if h, ok := next(0, led); !ok || h != 3 {
		t.Fatalf("first end date %d %v", h, ok)
	}
	if h, ok := next(3, led); !ok || h != 5 {
		t.Fatalf("at an end date the next one counts: %d %v", h, ok)
	}
	if _, ok := next(5, led); ok {
		t.Fatal("an org rule has no end date")
	}
	ext := append(led, rec(2, ledger.ActionExtend, "cr_b", srcB, rfc(8), ledger.Local{}))
	if h, _ := next(3, ext); h != 8 {
		t.Fatalf("local extension: %d", h)
	}
	rev := append(led, rec(2, ledger.ActionRevoke, "", srcC, "", ledger.Local{}))
	if h, _ := next(0, rev); h != 5 {
		t.Fatalf("a revoked rule is not waited for: %d", h)
	}
}
