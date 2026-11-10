// SPDX-License-Identifier: Apache-2.0
//
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
//
// The single implementation for every Rust consumer: goat-cli's onboard mint +
// native-revoke selector, and microscope's spine join.

/// The literal that opens every operator-onboarded peer name.
pub const PREFIX: &str = "peer-";

/// Field separator, and the terminator that anchors a keyed match.
pub const SEP: &str = "-";

/// The human-readable grammar, for operator-facing text.
pub const FORM: &str = "peer-<handle>[-<org_short_tag>]-<site>-<date>[-<hostname>]";

/// The grammar's semver. A change to PREFIX, SEP, or field order renames the
/// whole fleet and is a MAJOR bump plus a migration.
pub const VERSION: &str = "1.1.0";

/// True when a name follows the operator-onboarded convention at all. A false answer
/// means "no principal key is encoded in this name", which is an untracked peer, NOT a
/// peer that failed to match a known principal.
pub fn is_peer_name(name: &str) -> bool {
    name.starts_with(PREFIX)
}

/// The keyed remainder — everything after the prefix. Absent when the name is not a
/// peer name at all, so a caller can never accidentally key off a non-conforming name's
/// leading characters.
pub fn rest(name: &str) -> Option<&str> {
    name.strip_prefix(PREFIX)
}

/// The LONGEST candidate key the name is keyed by, or none. A key matches only when the
/// remainder starts with the key followed by the separator, so `foo` never claims
/// `peer-foobar-…`, and longest-wins so a key that is a prefix of another cannot shadow
/// it. This is the one matcher behind both the handle-keyed (person) and prefix-keyed
/// (org) joins. Its limit is stated rather than papered over: the separator also occurs
/// INSIDE handles and tags, so `sam` does match `peer-sam-smith-…` whenever `sam-smith`
/// is not itself among the candidates. Pass the FULL candidate set and let longest-wins
/// disambiguate; for anything that acts on a principal, use `belongs_to`, which anchors
/// on the site too.
pub fn match_longest_key<'a>(name: &str, keys: &[&'a str]) -> Option<&'a str> {
    let rest = rest(name)?;
    let mut best: Option<&'a str> = None;
    let mut best_len = 0usize;
    for k in keys {
        if k.is_empty() || !rest.starts_with(&format!("{k}{SEP}")) {
            continue;
        }
        if best.is_none() || k.len() > best_len {
            best = Some(k);
            best_len = k.len();
        }
    }
    best
}

/// Compose the name an onboard assigns. The hostname field is deliberately absent:
/// NetBird appends it at enrollment. An `org_short_tag` EQUAL to the handle is omitted:
/// it carries no information, and on per-uuid goatnets the tag is FORCED to equal the
/// handle (it doubles as the Zenoh prefix), which minted names like
/// `peer-pat-sample-pat-sample-galaxy-2000-01-01`. It is the same degenerate
/// handle-as-org value that `org_trait_short_tag` already corrects for the Ory trait;
/// this closes the second consumer.
pub fn mint(handle: &str, org_short_tag: &str, site: &str, date: &str) -> String {
    let h = handle.trim();
    let tag = org_short_tag.trim();
    // A tag equal to the handle carries no information and is omitted.
    // On per-uuid goatnets `short_tag` is FORCED to equal the handle (it
    // doubles as the Zenoh prefix), which minted names like
    // `peer-user371x-user371x-zagros-...` — 23 of 111 peers
    // on such a goatnet. Same degenerate value the Ory org trait corrects.
    let tag = if tag == h { "" } else { tag };
    let tag_seg = if tag.is_empty() {
        String::new()
    } else {
        format!("{SEP}{tag}")
    };
    format!(
        "{PREFIX}{handle}{tag_seg}{SEP}{site}{SEP}{date}",
        handle = h,
        site = site.trim(),
        date = date.trim(),
    )
}

/// Does this peer belong to exactly this principal on this goatnet? Anchored on the
/// FULL keyed prefix (handle, then the optional org tag, then the site), which is
/// strictly narrower than `match_longest_key`: an org-mate carrying the same shared tag
/// mid-name is never attributed here. This is the selector a revoke cut may use; the
/// loose matcher is not. It accepts BOTH the tagged and the untagged form when the tag
/// equals the handle — a MIGRATION REQUIREMENT, not leniency: peers minted before
/// v1.1.0 still carry the doubled segment, and a selector that accepted only the new
/// shape would silently skip them. Because this is what a revoke cut keys on, "silently
/// skip" means a credential that was supposed to be cut stays live. The dual accept
/// retires when no peer carries the doubled form.
pub fn belongs_to(name: &str, handle: &str, org_short_tag: &str, site: &str) -> bool {
    let (h, s, tag) = (handle.trim(), site.trim(), org_short_tag.trim());
    if h.is_empty() || s.is_empty() {
        return false;
    }
    if tag.is_empty() {
        return name.starts_with(&format!("{PREFIX}{h}{SEP}{s}{SEP}"));
    }
    // BOTH forms, and this is a migration requirement rather than
    // leniency. `mint` stopped emitting a tag equal to the handle, but
    // every peer minted BEFORE that still carries the doubled form. A
    // selector that accepted only the new shape would silently skip
    // those peers — and this is the function a REVOKE CUT uses, so
    // "silently skip" means a credential that was supposed to be cut
    // stays live. Accept the doubled form until no peer carries it.
    if name.starts_with(&format!("{PREFIX}{h}{SEP}{tag}{SEP}{s}{SEP}")) {
        return true;
    }
    tag == h && name.starts_with(&format!("{PREFIX}{h}{SEP}{s}{SEP}"))
}

#[cfg(test)]
// rustfmt would reflow these one-line cases and the byte-parity fence would then
// report drift the SoT did not cause; the generator owns this block's layout.
#[rustfmt::skip]
mod conformance_vectors {
    // The SoT's cross-language vectors — the IDENTICAL cases the Go
    // implementation's generated test runs, so the two are proven to agree.
    use super::*;

    #[test]
    fn is_peer_name_vectors() {
        assert!(is_peer_name("peer-maintainer-zagros-2026-06-10-mac"), "the canonical form");
        assert!(is_peer_name("peer-"), "prefix alone conforms; the remainder is simply empty");
        assert!(!is_peer_name("thor-a"), "an unmanaged peer carries no principal key");
        assert!(!is_peer_name("Peer-maintainer-zagros-2026-06-10"), "the prefix is case-sensitive");
        assert!(!is_peer_name(""), "empty is not a peer name");
    }

    #[test]
    fn rest_vectors() {
        assert_eq!(rest("peer-maintainer-zagros-2026-06-10-mac"), Some("maintainer-zagros-2026-06-10-mac"), "the keyed remainder");
        assert_eq!(rest("peer-"), Some(""), "an empty remainder is present-but-empty, not absent");
        assert_eq!(rest("thor-a"), None, "absent for a non-conforming name");
    }

    #[test]
    fn match_longest_key_vectors() {
        assert_eq!(match_longest_key("peer-aardwolf-nova-2026-05-21-laptop", &["aardwolf", "user52x"]), Some("aardwolf"), "the ordinary org-prefix join");
        assert_eq!(match_longest_key("peer-foobar-nova-2026-05-21-host", &["foo", "foobar"]), Some("foobar"), "longest wins — `foo` must not shadow `foobar`");
        assert_eq!(match_longest_key("peer-foobar-nova-2026-05-21-host", &["foo"]), None, "the separator anchor: `foo` does not match `foobar` at all");
        assert_eq!(match_longest_key("peer-maintainer-smith-zagros-2026-06-10-mac", &["maintainer", "maintainer-smith"]), Some("maintainer-smith"), "the org-mate mis-attribution class (hit live 2026-07-14) — the separator also occurs INSIDE a handle, so `maintainer-` does prefix `maintainer-smith-…`; longest-wins over the full candidate set is what keeps `maintainer` from claiming their peer");
        assert_eq!(match_longest_key("peer-maintainer-smith-zagros-2026-06-10-mac", &["maintainer"]), Some("maintainer"), "the stated limit of this matcher, pinned so nobody mistakes it for a cut selector: with the true owner ABSENT from the candidate set, a shorter handle does match. Anything that acts on a principal (a revoke cut) must use `belongs_to`, which anchors on handle AND site instead.");
        assert_eq!(match_longest_key("peer-maintainer-zagros-2026-06-10-mac", &["", "maintainer"]), Some("maintainer"), "an empty key never matches, and never wins by being 'longest'");
        assert_eq!(match_longest_key("thor-a", &["thor"]), None, "a non-conforming name is keyed by nothing, whatever it contains");
        assert_eq!(match_longest_key("peer-maintainer-zagros-2026-06-10-mac", &[]), None, "no candidates, no match — never a fabricated one");
    }

    #[test]
    fn mint_vectors() {
        assert_eq!(mint("maintainer", "", "nova", "2026-05-17"), "peer-maintainer-nova-2026-05-17", "unaffiliated principal — the org segment is absent, not empty");
        assert_eq!(mint("maintainer", "acme-corp", "nova", "2026-05-17"), "peer-maintainer-acme-corp-nova-2026-05-17", "org-affiliated principal");
        assert_eq!(mint("maintainer", "  acme  ", "zagros", "2026-06-10"), "peer-maintainer-acme-zagros-2026-06-10", "a manifest's stray whitespace never reaches the wire name");
        assert_eq!(mint("user371x", "user371x", "zagros", "2026-06-16"), "peer-user371x-zagros-2026-06-16", "v1.1.0 — a tag EQUAL to the handle is omitted. This exact input minted peer-user371x-user371x-zagros-2026-06-16 on ZAGROS; the tag is forced to equal the handle on per-uuid goatnets and carries nothing.");
        assert_eq!(mint("maintainer", "  maintainer  ", "zagros", "2026-06-10"), "peer-maintainer-zagros-2026-06-10", "the equality is tested AFTER trimming, or whitespace defeats the rule");
    }

    #[test]
    fn belongs_to_vectors() {
        assert!(belongs_to("peer-maintainer-zagros-2026-06-10-mac", "maintainer", "", "zagros"), "untagged form");
        assert!(belongs_to("peer-maintainer-acme-zagros-2026-06-10-mac", "maintainer", "acme", "zagros"), "tagged form");
        assert!(!belongs_to("peer-maintainer-acme-zagros-2026-06-10-mac", "maintainer", "", "zagros"), "a manifest with no tag must not claim the tagged peer");
        assert!(belongs_to("peer-user371x-user371x-zagros-2026-06-16-JL-2.local", "user371x", "user371x", "zagros"), "MIGRATION — a peer minted BEFORE v1.1.0 carries the doubled segment and must still be claimed. This is the revoke selector: a miss here leaves a credential live that was supposed to be cut.");
        assert!(belongs_to("peer-user371x-zagros-2026-06-16-JL-2.local", "user371x", "user371x", "zagros"), "the same principal minted AFTER v1.1.0 — both forms, one selector");
        assert!(!belongs_to("peer-maintainer-smith-zagros-2026-06-10-mac", "maintainer", "maintainer", "zagros"), "the dual accept must not widen: `maintainer`+tag `maintainer` never claims the DIFFERENT principal `maintainer-smith`, whose name merely starts the same way");
        assert!(!belongs_to("peer-maintainer-smith-zagros-2026-06-10-mac", "maintainer", "", "zagros"), "the org-mate mis-attribution class, on the revoke selector");
        assert!(!belongs_to("peer-maintainer-zagros-2026-06-10-mac", "maintainer", "", "nova"), "site-scoped: a revoke on one goatnet never cuts a peer on another");
        assert!(!belongs_to("peer-maintainer-acme-zagros-2026-06-10-mac", "maintainer", "beta", "zagros"), "the wrong tag is not a match");
    }
}
