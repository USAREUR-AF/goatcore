package generation

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/testpki"
)

const (
	pod = "0123456789abcdef0123456789abcdef"
	src = "fedcba9876543210fedcba9876543210"
)

func doc() map[string]any {
	return map[string]any{
		"schema":          Schema,
		"pod_slot":        pod,
		"generation":      7,
		"issued_at":       "2026-11-19T10:00:00Z",
		"local_authority": []any{map[string]any{"identity": "us-dod:1234567890", "cert_cn": "lead.g1"}},
		"read":            []any{"*/**"},
		"sources": []any{map[string]any{
			"slot": src, "rule_id": "cr_1", "kind": "org", "until": nil,
			"principals": []any{src + ".g1"},
			"write":      []any{src + "/**", "volatile/" + src + "/**", src + "/**/@adv/**"},
		}},
		"signer": "portal-signer",
	}
}

func TestVerifyRoundTrip(t *testing.T) {
	ca := testpki.NewCA(t, "ca")
	signer := ca.Leaf(t, "portal-signer")
	raw, err := Sign(doc(), signer.Key)
	if err != nil {
		t.Fatal(err)
	}
	g, err := Verify(raw, signer.Cert, pod)
	if err != nil {
		t.Fatal(err)
	}
	if g.Generation != 7 || len(g.Sources) != 1 || g.Sources[0].Principals[0] != src+".g1" {
		t.Fatalf("unexpected contents: %+v", g)
	}
	// Field order and whitespace do not matter: re-indent the signed bytes.
	var v any
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatal(err)
	}
	pretty, _ := json.MarshalIndent(v, "", "  ")
	if _, err := Verify(pretty, signer.Cert, pod); err != nil {
		t.Fatalf("re-indented document refused: %v", err)
	}
}

func TestVerifyRefusals(t *testing.T) {
	ca := testpki.NewCA(t, "ca")
	signer := ca.Leaf(t, "portal-signer")
	other := ca.Leaf(t, "portal-signer")
	wrongCN := ca.Leaf(t, "someone-else")

	signed := func(mut func(map[string]any)) []byte {
		d := doc()
		if mut != nil {
			mut(d)
		}
		raw, err := Sign(d, signer.Key)
		if err != nil {
			t.Fatal(err)
		}
		return raw
	}
	good := signed(nil)

	cases := []struct {
		name string
		raw  []byte
		cert *testpki.Leaf
		want string
	}{
		{"tampered", []byte(strings.Replace(string(good), `"generation":7`, `"generation":8`, 1)), signer, "signature"},
		{"other key", good, other, "signature"},
		{"signer CN", signed(func(d map[string]any) { d["signer"] = "someone-else" }), wrongCN, "signature"},
		{"named signer mismatch", signed(func(d map[string]any) { d["signer"] = "x" }), signer, "names signer"},
		{"other pod", signed(func(d map[string]any) { d["pod_slot"] = src }), signer, "for pod"},
		{"bare read", signed(func(d map[string]any) { d["read"] = []any{"**"} }), signer, "bare **"},
		{"write outside slot", signed(func(d map[string]any) {
			d["sources"].([]any)[0].(map[string]any)["write"] = []any{"other/**"}
		}), signer, "outside its own slot"},
		{"named without until", signed(func(d map[string]any) {
			d["sources"].([]any)[0].(map[string]any)["kind"] = "named"
		}), signer, "end date"},
		{"cn injection", signed(func(d map[string]any) {
			d["sources"].([]any)[0].(map[string]any)["principals"] = []any{`a"b`}
		}), signer, "malformed"},
		{"unsigned", []byte(`{"schema":"goat.carriage.generation/v1"}`), signer, "no signature"},
	}
	for _, c := range cases {
		if _, err := Verify(c.raw, c.cert.Cert, pod); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s: err = %v, want containing %q", c.name, err, c.want)
		}
	}
}

func TestWithinSlot(t *testing.T) {
	for expr, want := range map[string]bool{
		src + "/**":                true,
		src:                        true,
		"volatile/" + src + "/a":   true,
		src + "x/**":               false,
		"*/**":                     false,
		"volatile/" + src + "x/**": false,
	} {
		if WithinSlot(expr, src) != want {
			t.Errorf("WithinSlot(%q) != %v", expr, want)
		}
	}
}
