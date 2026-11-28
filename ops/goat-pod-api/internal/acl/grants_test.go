package acl

import (
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
)

func TestRemoves(t *testing.T) {
	ren := func(read []string, src ...generation.Source) []byte {
		t.Helper()
		out, err := Render([]byte(base), src, read, Options{})
		if err != nil {
			t.Fatal(err)
		}
		return out
	}
	all, read := sources(), []string{"*/**"}
	first, second, both := ren(read, all[0]), ren(read, all[1]), ren(read, all...)
	narrowed := all[0]
	narrowed.Write = narrowed.Write[:1]
	rotated := all[0]
	rotated.Principals = []string{a + ".g2"}
	for name, c := range map[string]struct {
		live, next []byte
		want       bool
	}{
		"nothing to one":                 {[]byte(base), first, false},
		"same":                           {both, both, false},
		"second added":                   {first, both, false},
		"first added, entry ids shift":   {second, both, false},
		"second removed":                 {both, first, true},
		"first removed, entry ids shift": {both, second, true},
		"swap":                           {first, second, true},
		"write set narrowed":             {first, ren(read, narrowed), true},
		"certificate replaced":           {first, ren(read, rotated), true},
		"read set narrowed":              {first, ren([]string{a + "/**"}, all[0]), true},
		"all to none":                    {first, []byte(base), true},
		"live unreadable":                {[]byte(strings.Replace(string(first), `"rules":[`, `"rules":`, 1)), both, true},
	} {
		if got := Removes(c.live, c.next); got != c.want {
			t.Errorf("%s: removes = %v", name, got)
		}
	}
	if Carries([]byte(base)) || !Carries(first) {
		t.Error("Carries is wrong about the base or a carried source")
	}
}
