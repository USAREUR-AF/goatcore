// Package selfcheck decides whether two names for a person could be the same
// person. The console's two-person rules (nobody approves their own request,
// nobody decides a request about themselves) used to compare login emails
// with strings.EqualFold, which a second spelling of the same mailbox walks
// straight past: Alice+x@example.org, a Cyrillic "а" in place of a Latin "a",
// a zero-width space. The stable answer is the Kratos identity id, and the
// callers compare that first; this package is what they compare when all
// they hold is an address or a handle.
//
// Every fold here OVER-matches on purpose. A false "same person" costs one
// click by a second admin; a false "different person" is a self-approval.
package selfcheck

import (
	"strings"
	"unicode"

	"golang.org/x/text/cases"
	"golang.org/x/text/runes"
	"golang.org/x/text/transform"
	"golang.org/x/text/unicode/norm"
)

// confusables maps characters that render like a Latin letter to that letter.
// It is the part of the Unicode confusables table (UTS #39) that matters for
// addresses: Cyrillic and Greek letters that are indistinguishable from Latin
// ones in an ordinary font, applied after case folding, so only lower-case
// forms are listed. Characters that NFKC already maps (fullwidth forms,
// mathematical letters, ℓ, ﬁ) are not repeated here.
var confusables = map[rune]rune{
	// Cyrillic
	'а': 'a', 'в': 'b', 'с': 'c', 'ԁ': 'd', 'е': 'e', 'ё': 'e', 'һ': 'h',
	'і': 'i', 'ї': 'i', 'ј': 'j', 'к': 'k', 'ӏ': 'l', 'м': 'm', 'н': 'h',
	'о': 'o', 'р': 'p', 'ԛ': 'q', 'ѕ': 's', 'т': 't',
	'ѵ': 'v', 'ԝ': 'w', 'х': 'x', 'у': 'y', 'ү': 'y',
	// Greek
	'α': 'a', 'β': 'b', 'ϲ': 'c', 'ε': 'e', 'η': 'n', 'ι': 'i', 'κ': 'k',
	'ν': 'v', 'ο': 'o', 'ρ': 'p', 'τ': 't', 'υ': 'u', 'χ': 'x', 'γ': 'y',
	'ω': 'w',
	// Latin letters that read as another Latin letter
	'ı': 'i', 'ȷ': 'j', 'ɡ': 'g', 'ɑ': 'a', 'ɩ': 'i', 'ʟ': 'l', 'ꞵ': 'b',
	'ø': 'o', 'đ': 'd', 'ħ': 'h', 'ł': 'l', 'ŧ': 't',
	// Punctuation that reads as a separator
	'·': '.', '․': '.', '‧': '.', '｡': '.', '։': ':',
}

// skeleton folds s to a lookalike-free, case-free form: compatibility
// decomposition, full case fold, combining marks and invisible format
// characters (zero-width space, joiners, bidi controls) removed, then the
// confusables mapped to the Latin letter they imitate.
func skeleton(s string) string {
	t := transform.Chain(
		norm.NFKC,
		cases.Fold(),
		norm.NFD,
		runes.Remove(runes.In(unicode.Mn)),
		runes.Remove(runes.In(unicode.Cf)),
		runes.Map(func(r rune) rune {
			if m, ok := confusables[r]; ok {
				return m
			}
			return r
		}),
		norm.NFC,
	)
	out, _, err := transform.String(t, s)
	if err != nil {
		// A transform error leaves no trustworthy fold; the plain lower-case
		// form is still a valid (weaker) key and never panics a gate.
		return strings.ToLower(s)
	}
	return out
}

// EmailKey is the form every spelling of one mailbox shares: lookalikes and
// case folded (skeleton), surrounding space and a trailing dot on the domain
// dropped, and a plus-address tag ("alice+second@") removed from the local
// part. A value with no usable "@" is returned as its skeleton, so a handle
// passed by mistake still compares sensibly. "" stays "".
func EmailKey(addr string) string {
	a := strings.TrimSpace(skeleton(strings.TrimSpace(addr)))
	at := strings.LastIndexByte(a, '@')
	if at <= 0 || at == len(a)-1 {
		return a
	}
	local, domain := a[:at], strings.TrimSuffix(a[at+1:], ".")
	if i := strings.IndexByte(local, '+'); i > 0 {
		local = local[:i]
	}
	return local + "@" + domain
}

// HandleKey is the comparison form of an onboarding or operator-key handle:
// the same lookalike and case fold as an address, with nothing else removed.
func HandleKey(h string) string {
	return strings.TrimSpace(skeleton(strings.TrimSpace(h)))
}

// IsEmail reports whether v is shaped like an address (has an "@" with text on
// both sides) rather than a handle.
func IsEmail(v string) bool {
	v = strings.TrimSpace(skeleton(strings.TrimSpace(v)))
	at := strings.LastIndexByte(v, '@')
	return at > 0 && at < len(v)-1
}

// Same reports whether a and b could name the same person: both addresses and
// equal under EmailKey, or both handles and equal under HandleKey. An empty
// value matches nothing, and an address never matches a handle (callers that
// hold both forms of a person compare each against its own kind).
func Same(a, b string) bool {
	a, b = strings.TrimSpace(a), strings.TrimSpace(b)
	if a == "" || b == "" {
		return false
	}
	ae, be := IsEmail(a), IsEmail(b)
	switch {
	case ae && be:
		return EmailKey(a) == EmailKey(b)
	case !ae && !be:
		return HandleKey(a) == HandleKey(b)
	}
	return false
}

// AnySame reports whether any value in a could name the same person as any
// value in b (Same, pairwise).
func AnySame(a, b []string) bool {
	for _, x := range a {
		for _, y := range b {
			if Same(x, y) {
				return true
			}
		}
	}
	return false
}
