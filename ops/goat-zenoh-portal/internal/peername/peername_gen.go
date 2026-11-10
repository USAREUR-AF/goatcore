// Code generated from ops/identity/peer-name-grammar.yaml by ops/identity/generate-peer-name.py. DO NOT EDIT.
//
// Peer-name grammar v1.1.0 — peer-<handle>[-<org_short_tag>]-<site>-<date>[-<hostname>]
//
// Under setup-key enrollment NetBird leaves peer.user_id empty, so the peer NAME is the
// only link from a mesh peer to the principal that owns it
// (docs/design/peer-identity-resolution.md, Option B). Edit the SoT and re-run the
// generator; ops/ci/check-peer-name-grammar-sync.sh fails the build on drift.
//
// A peer name is EVIDENCE about ownership, never a record of it — the recorded linkage
// is slots.identity_id, stamped at onboard. A consumer that presents ownership must say
// which of the two it used.

// Package peername is the single implementation of the operator-onboarded
// NetBird peer-name grammar for every Go consumer in the portal: the vendor
// swimlane poller, the access-COP graph, and the /api/access/peer resolver each
// carried their own copy until the identity fragility audit.
package peername

import "strings"

const (
	// Prefix opens every operator-onboarded peer name.
	Prefix = "peer-"

	// Sep separates fields, and terminates a key so a shorter key can never
	// claim a longer one's peer.
	Sep = "-"

	// Form is the human-readable grammar, for operator-facing text.
	Form = "peer-<handle>[-<org_short_tag>]-<site>-<date>[-<hostname>]"

	// Version is the grammar's semver. A change to Prefix, Sep, or field order
	// renames the whole fleet and is a MAJOR bump plus a migration.
	Version = "1.1.0"
)

// IsPeerName — True when a name follows the operator-onboarded convention at all. A
// false answer means "no principal key is encoded in this name", which is an untracked
// peer, NOT a peer that failed to match a known principal.
func IsPeerName(name string) bool {
	return strings.HasPrefix(name, Prefix)
}

// Rest — The keyed remainder — everything after the prefix. Absent when the name is not
// a peer name at all, so a caller can never accidentally key off a non-conforming
// name's leading characters.
func Rest(name string) (string, bool) {
	if !IsPeerName(name) {
		return "", false
	}
	return name[len(Prefix):], true
}

// MatchLongestKey — The LONGEST candidate key the name is keyed by, or none. A key
// matches only when the remainder starts with the key followed by the separator, so
// `foo` never claims `peer-foobar-…`, and longest-wins so a key that is a prefix of
// another cannot shadow it. This is the one matcher behind both the handle-keyed
// (person) and prefix-keyed (org) joins. Its limit is stated rather than papered over:
// the separator also occurs INSIDE handles and tags, so `sam` does match
// `peer-sam-smith-…` whenever `sam-smith` is not itself among the candidates. Pass the
// FULL candidate set and let longest-wins disambiguate; for anything that acts on a
// principal, use `belongs_to`, which anchors on the site too.
func MatchLongestKey(name string, keys []string) string {
	rest, ok := Rest(name)
	if !ok {
		return ""
	}
	best := ""
	for _, k := range keys {
		if k == "" {
			continue
		}
		if strings.HasPrefix(rest, k+Sep) && len(k) > len(best) {
			best = k
		}
	}
	return best
}

// Mint — Compose the name an onboard assigns. The hostname field is deliberately
// absent: NetBird appends it at enrollment. An `org_short_tag` EQUAL to the handle is
// omitted: it carries no information, and on per-uuid goatnets the tag is FORCED to
// equal the handle (it doubles as the Zenoh prefix), which minted names like
// `peer-pat-sample-pat-sample-galaxy-2000-01-01`. It is the same degenerate
// handle-as-org value that `org_trait_short_tag` already corrects for the Ory trait;
// this closes the second consumer.
func Mint(handle, orgShortTag, site, date string) string {
	var b strings.Builder
	h := strings.TrimSpace(handle)
	b.WriteString(Prefix)
	b.WriteString(h)
	// A tag equal to the handle carries no information and is omitted.
	// On per-uuid goatnets short_tag is FORCED to equal the handle (it
	// doubles as the Zenoh prefix), which minted names like
	// peer-sam-lee-sam-lee-<site>-... for every person
	// on such a goatnet. Same degenerate value the Ory org trait corrects.
	if tag := strings.TrimSpace(orgShortTag); tag != "" && tag != h {
		b.WriteString(Sep)
		b.WriteString(tag)
	}
	b.WriteString(Sep)
	b.WriteString(strings.TrimSpace(site))
	b.WriteString(Sep)
	b.WriteString(strings.TrimSpace(date))
	return b.String()
}

// BelongsTo — Does this peer belong to exactly this principal on this goatnet? Anchored
// on the FULL keyed prefix (handle, then the optional org tag, then the site), which is
// strictly narrower than `match_longest_key`: an org-mate carrying the same shared tag
// mid-name is never attributed here. This is the selector a revoke cut may use; the
// loose matcher is not. It accepts BOTH the tagged and the untagged form when the tag
// equals the handle — a MIGRATION REQUIREMENT, not leniency: peers minted before v1.1.0
// still carry the doubled segment, and a selector that accepted only the new shape
// would silently skip them. Because this is what a revoke cut keys on, "silently skip"
// means a credential that was supposed to be cut stays live. The dual accept retires
// when no peer carries the doubled form.
func BelongsTo(name, handle, orgShortTag, site string) bool {
	h, s := strings.TrimSpace(handle), strings.TrimSpace(site)
	if h == "" || s == "" {
		return false
	}
	tag := strings.TrimSpace(orgShortTag)
	if tag == "" {
		return strings.HasPrefix(name, Prefix+h+Sep+s+Sep)
	}
	// BOTH forms, and this is a migration requirement rather than
	// leniency. Mint stopped emitting a tag equal to the handle, but every
	// peer minted BEFORE that still carries the doubled form. A selector
	// accepting only the new shape would silently skip those peers — and
	// this is the function a REVOKE CUT uses, so "silently skip" means a
	// credential that was supposed to be cut stays live. Accept the
	// doubled form until no peer carries it.
	if strings.HasPrefix(name, Prefix+h+Sep+tag+Sep+s+Sep) {
		return true
	}
	return tag == h && strings.HasPrefix(name, Prefix+h+Sep+s+Sep)
}
