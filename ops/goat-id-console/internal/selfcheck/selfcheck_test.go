package selfcheck

import "testing"

func TestSame_secondSpellingsOfOneMailbox(t *testing.T) {
	const base = "alice@example.org"
	for _, tc := range []struct{ name, other string }{
		{"case", "Alice@EXAMPLE.org"},
		{"surrounding space", "  alice@example.org "},
		{"plus tag", "alice+second@example.org"},
		{"plus tag and case", "ALICE+Admin2@Example.Org"},
		{"trailing dot on domain", "alice@example.org."},
		{"cyrillic a", "аlice@example.org"},
		{"cyrillic e and o in domain", "alice@еxamplе.оrg"},
		{"greek omicron", "alice@example.οrg"},
		{"fullwidth letters and at", "ａｌｉｃｅ＠example.org"},
		{"zero-width space", "ali\u200bce@example.org"},
		{"combining accent", "alíce@example.org"},
		{"precomposed accent", "alíce@example.org"},
		{"dotless i", "alıce@example.org"},
		{"capital sharp fold path", "ALICE@EXAMPLE.ORG"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if !Same(base, tc.other) {
				t.Errorf("Same(%q, %q) = false; EmailKey %q vs %q", base, tc.other, EmailKey(base), EmailKey(tc.other))
			}
		})
	}
}

func TestSame_differentPeopleStayDifferent(t *testing.T) {
	for _, tc := range []struct{ a, b string }{
		{"alice@example.org", "bob@example.org"},
		{"alice@example.org", "alice@example.com"},
		{"alice@example.org", "alicia@example.org"},
		{"alice.b@example.org", "aliceb@example.org"},
		{"alice", "bob"},
		{"", ""},
		{"alice@example.org", ""},
		// An address never equals a handle, even one spelled like its local part.
		{"alice@example.org", "alice"},
	} {
		if Same(tc.a, tc.b) {
			t.Errorf("Same(%q, %q) = true, want false", tc.a, tc.b)
		}
	}
}

func TestSame_handles(t *testing.T) {
	if !Same("Alice-Ops", "alice-ops") {
		t.Error("handle compare must fold case")
	}
	if !Same("alice-ops", "аlice-ops") {
		t.Error("handle compare must fold lookalikes")
	}
	if Same("alice+ops", "alice") {
		t.Error("a plus sign in a handle is part of the handle, not a tag")
	}
}

func TestAnySame(t *testing.T) {
	if !AnySame([]string{"x@example.org", "Alice"}, []string{"y@example.org", "alice"}) {
		t.Error("AnySame missed a matching handle")
	}
	if AnySame([]string{"x@example.org"}, []string{"y@example.org", "x"}) {
		t.Error("AnySame matched an address against a handle")
	}
}
