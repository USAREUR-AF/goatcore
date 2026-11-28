package acl

import (
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
)

const base = `{
  access_control: {
    rules: [
      { id: "pod-firstparty" },
      // @carriage-rules
    ],
    subjects: [
      { id: "pod-peer" },
      // @carriage-subjects
    ],
    policies: [
      { rules: ["pod-firstparty"], subjects: ["pod-peer"] },
      // @carriage-policies
    ],
  },
}`

const (
	a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
)

func sources() []generation.Source {
	until := time.Now().Add(time.Hour)
	return []generation.Source{
		{Slot: a, RuleID: "cr_a", Kind: "org", Principals: []string{a + ".g1"}, Write: generation.DefaultWrite(a)},
		{Slot: b, RuleID: "cr_b", Kind: "named", Until: &until, Principals: []string{"adapter.g1"}, Write: []string{b + "/**"}},
	}
}

func TestRenderEmptyKeepsBase(t *testing.T) {
	out, err := Render([]byte(base), nil, []string{"*/**"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if string(out) != base {
		t.Fatalf("empty rendering changed the base:\n%s", out)
	}
}

func TestRenderSources(t *testing.T) {
	out, err := Render([]byte(base), sources(), []string{"*/**"}, Options{
		HealthKey: "ops/pod/health/p", Reserved: []string{"pod-cn", "fabric-cn"},
	})
	if err != nil {
		t.Fatal(err)
	}
	s := string(out)
	for _, want := range []string{
		`{ id: "pod-firstparty" },`,
		`"id":"carriage-cn-1","cert_common_names":["` + a + `.g1"]`,
		`"id":"carriage-cn-2","cert_common_names":["adapter.g1"]`,
		`"id":"carriage-src-` + a + `-in","messages":["put","delete","declare_queryable","reply","liveliness_token"],"flows":["ingress"],"permission":"allow","key_exprs":["` + a + `/**","volatile/` + a + `/**","` + a + `/**/@adv/**"]`,
		`"rules":["carriage-src-` + a + `-in","carriage-src-` + a + `-query-out"],"subjects":["carriage-cn-1"]`,
		`"subjects":["fabric-peer"]`,
		`"id":"carriage-read-interest-in"`,
		`"key_exprs":["*/**"]`,
		`"id":"carriage-health-in"`,
	} {
		if !strings.Contains(s, want) {
			t.Errorf("rendering lacks %s", want)
		}
	}
	// A source's write rule names only its own slot.
	if strings.Contains(s, `"carriage-src-`+b+`-in","messages":["put","delete","declare_queryable","reply","liveliness_token"],"flows":["ingress"],"permission":"allow","key_exprs":["`+a) {
		t.Error("source b was granted a's slot")
	}
	// Indentation follows the marker line.
	if !strings.Contains(s, "\n      {\"id\":\"carriage-cn-1\"") {
		t.Error("entries not indented like the marker")
	}
}

func TestRenderRefusals(t *testing.T) {
	if _, err := Render([]byte(strings.Replace(base, MarkerPolicies, "", 1)), nil, nil, Options{}); err == nil ||
		!strings.Contains(err.Error(), "lacks the carriage markers") {
		t.Fatalf("missing marker: %v", err)
	}
	if _, err := Render([]byte(base), sources(), []string{"**"}, Options{}); err == nil {
		t.Fatal("bare ** read accepted")
	}
	if _, err := Render([]byte(base), sources(), nil, Options{Reserved: []string{"adapter.g1"}}); err == nil {
		t.Fatal("reserved CN accepted as a principal")
	}
	bad := sources()
	bad[1].Write = []string{a + "/**"}
	if _, err := Render([]byte(base), bad, nil, Options{}); err == nil {
		t.Fatal("write outside the source's slot accepted")
	}
}

func TestDeterministic(t *testing.T) {
	one, _ := Render([]byte(base), sources(), []string{"*/**"}, Options{})
	two, _ := Render([]byte(base), sources(), []string{"*/**"}, Options{})
	if string(one) != string(two) {
		t.Fatal("rendering is not deterministic")
	}
}
