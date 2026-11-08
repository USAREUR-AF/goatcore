package compat

import "testing"

func TestSatlinkWord(t *testing.T) {
	for in, want := range map[string]string{
		"":              "satlink",
		"otherlink":     "otherlink",
		"Bad-Name":      "satlink",
		"x; drop table": "satlink",
	} {
		if got := satlinkWord(in); got != want {
			t.Errorf("satlinkWord(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestSatlinkTargetsFrom(t *testing.T) {
	got := satlinkTargetsFrom("a=192.0.2.1=r1, b=192.0.2.2=r2,broken")
	if len(got) != 2 || got[0] != (SatlinkTarget{"a", "192.0.2.1", "r1"}) || got[1].Name != "b" {
		t.Fatalf("parsed %+v", got)
	}
	if d := satlinkTargetsFrom(""); len(d) != len(SatlinkTargets) {
		t.Fatalf("empty must give the built-in targets, got %+v", d)
	}
}
