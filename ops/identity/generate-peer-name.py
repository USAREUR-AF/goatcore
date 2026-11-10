#!/usr/bin/env python3
"""ops/identity/generate-peer-name.py — the peer-name grammar generator.

Reads ops/identity/peer-name-grammar.yaml (the SoT) and emits the Go and Rust
implementations of the peer-name convention plus a table-driven conformance
test in each language, derived from the SoT's shared vectors. One-way: SoT ->
code, never code parsed back.

Sibling of generate.py (ADR 1067), deliberately a SEPARATE script over a
SEPARATE SoT file: grammar.yaml is a controlled-vocabulary SoT whose outputs
are JSON-Schema enum blocks, this is a string-composition rule whose outputs
are source files, and only one of the peer name's five fields (site) is an
identity-grammar dimension at all. Folding them would couple two unrelated
semver lines and give generate.py a second output type it has no shape for.

Usage:
  generate-peer-name.py --check          # fail if any target has drifted
  generate-peer-name.py --write          # (re)write every target
  generate-peer-name.py --list           # list targets
  generate-peer-name.py --emit <target>  # print one target to stdout

The build gate is ops/ci/check-peer-name-grammar-sync.sh, which runs --check
and additionally asserts the Ansible onboard playbook (which templates the same
grammar in Jinja and cannot be generated into) still constructs peer names in
the SoT's field order.
"""
import argparse
import json
import sys
import textwrap
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("FAIL: PyYAML required (pip install pyyaml)")

HERE = Path(__file__).resolve().parent  # ops/identity/
REPO = HERE.parent.parent               # repo root
SOT = HERE / "peer-name-grammar.yaml"
SOT_REL = "ops/identity/peer-name-grammar.yaml"
GEN_REL = "ops/identity/generate-peer-name.py"

TARGETS = {
    "go": "ops/goat-zenoh-portal/internal/peername/peername_gen.go",
    "go-vectors": "ops/goat-zenoh-portal/internal/peername/vectors_gen_test.go",
    "rust": "crates/panoscope-core/src/peername.rs",
}


def load():
    with open(SOT) as f:
        return yaml.safe_load(f)


# ---------------------------------------------------------------- helpers ---

def wrap(text, width, prefix):
    """Reflow a SoT prose string into comment lines of at most `width` columns."""
    return textwrap.wrap(
        " ".join(text.split()), width=width,
        initial_indent=prefix, subsequent_indent=prefix,
        break_long_words=False, break_on_hyphens=False,
    ) or [prefix.rstrip()]


def doc_lines(text, prefix, width=88):
    return "\n".join(wrap(text, width, prefix))


def rule_doc(g, name):
    for r in g["rules"]:
        if r["name"] == name:
            return r["doc"]
    raise KeyError(f"no rule {name} in the SoT")


def gostr(s):
    return json.dumps(s, ensure_ascii=False)


def ruststr(s):
    return json.dumps(s, ensure_ascii=False)


def header(g, comment, target_rel):
    """The identical provenance banner every generated file carries."""
    lines = [
        f"{comment} Code generated from {SOT_REL} by {GEN_REL}. DO NOT EDIT.",
        f"{comment}",
        f"{comment} Peer-name grammar v{g['version']} — {g['form']}",
        f"{comment}",
    ]
    lines += wrap(
        "Under setup-key enrollment NetBird leaves peer.user_id empty, so the peer NAME is "
        "the only link from a mesh peer to the principal that owns it "
        "(docs/design/peer-identity-resolution.md, Option B). Edit the SoT and re-run the "
        "generator; ops/ci/check-peer-name-grammar-sync.sh fails the build on drift.",
        88, comment + " ")
    lines.append(comment)
    lines += wrap(
        "A peer name is EVIDENCE about ownership, never a record of it — the recorded "
        "linkage is slots.identity_id, stamped at onboard. A consumer that presents "
        "ownership must say which of the two it used.",
        88, comment + " ")
    return "\n".join(lines)


# ------------------------------------------------------------------- Go -----

def emit_go(g):
    prefix, sep, form = g["prefix"], g["separator"], g["form"]
    out = [header(g, "//", TARGETS["go"]), ""]
    out += [
        "// Package peername is the single implementation of the operator-onboarded",
        "// NetBird peer-name grammar for every Go consumer in the portal: the vendor",
        "// swimlane poller, the access-COP graph, and the /api/access/peer resolver each",
        "// carried their own copy until the identity fragility audit.",
        "package peername",
        "",
        'import "strings"',
        "",
        "const (",
        f"\t// Prefix opens every operator-onboarded peer name.",
        f"\tPrefix = {gostr(prefix)}",
        "",
        "\t// Sep separates fields, and terminates a key so a shorter key can never",
        "\t// claim a longer one's peer.",
        f"\tSep = {gostr(sep)}",
        "",
        "\t// Form is the human-readable grammar, for operator-facing text.",
        f"\tForm = {gostr(form)}",
        "",
        "\t// Version is the grammar's semver. A change to Prefix, Sep, or field order",
        "\t// renames the whole fleet and is a MAJOR bump plus a migration.",
        f"\tVersion = {gostr(g['version'])}",
        ")",
        "",
    ]

    out += [doc_lines("IsPeerName — " + rule_doc(g, "is_peer_name"), "// ")]
    out += [
        "func IsPeerName(name string) bool {",
        "\treturn strings.HasPrefix(name, Prefix)",
        "}",
        "",
    ]

    out += [doc_lines("Rest — " + rule_doc(g, "rest"), "// ")]
    out += [
        "func Rest(name string) (string, bool) {",
        "\tif !IsPeerName(name) {",
        '\t\treturn "", false',
        "\t}",
        "\treturn name[len(Prefix):], true",
        "}",
        "",
    ]

    out += [doc_lines("MatchLongestKey — " + rule_doc(g, "match_longest_key"), "// ")]
    out += [
        "func MatchLongestKey(name string, keys []string) string {",
        "\trest, ok := Rest(name)",
        "\tif !ok {",
        '\t\treturn ""',
        "\t}",
        '\tbest := ""',
        "\tfor _, k := range keys {",
        '\t\tif k == "" {',
        "\t\t\tcontinue",
        "\t\t}",
        "\t\tif strings.HasPrefix(rest, k+Sep) && len(k) > len(best) {",
        "\t\t\tbest = k",
        "\t\t}",
        "\t}",
        "\treturn best",
        "}",
        "",
    ]

    out += [doc_lines("Mint — " + rule_doc(g, "mint"), "// ")]
    out += [
        "func Mint(handle, orgShortTag, site, date string) string {",
        "\tvar b strings.Builder",
        "\th := strings.TrimSpace(handle)",
        "\tb.WriteString(Prefix)",
        "\tb.WriteString(h)",
        "\t// A tag equal to the handle carries no information and is omitted.",
        "\t// On per-uuid goatnets short_tag is FORCED to equal the handle (it",
        "\t// doubles as the Zenoh prefix), which minted names like",
        "\t// peer-sam-lee-sam-lee-<site>-... for every person",
        "\t// on such a goatnet. Same degenerate value the Ory org trait corrects.",
        "\tif tag := strings.TrimSpace(orgShortTag); tag != \"\" && tag != h {",
        "\t\tb.WriteString(Sep)",
        "\t\tb.WriteString(tag)",
        "\t}",
        "\tb.WriteString(Sep)",
        "\tb.WriteString(strings.TrimSpace(site))",
        "\tb.WriteString(Sep)",
        "\tb.WriteString(strings.TrimSpace(date))",
        "\treturn b.String()",
        "}",
        "",
    ]

    out += [doc_lines("BelongsTo — " + rule_doc(g, "belongs_to"), "// ")]
    out += [
        "func BelongsTo(name, handle, orgShortTag, site string) bool {",
        "\th, s := strings.TrimSpace(handle), strings.TrimSpace(site)",
        '\tif h == "" || s == "" {',
        "\t\treturn false",
        "\t}",
        "\ttag := strings.TrimSpace(orgShortTag)",
        '\tif tag == "" {',
        "\t\treturn strings.HasPrefix(name, Prefix+h+Sep+s+Sep)",
        "\t}",
        "\t// BOTH forms, and this is a migration requirement rather than",
        "\t// leniency. Mint stopped emitting a tag equal to the handle, but every",
        "\t// peer minted BEFORE that still carries the doubled form. A selector",
        "\t// accepting only the new shape would silently skip those peers — and",
        "\t// this is the function a REVOKE CUT uses, so \"silently skip\" means a",
        "\t// credential that was supposed to be cut stays live. Accept the",
        "\t// doubled form until no peer carries it.",
        "\tif strings.HasPrefix(name, Prefix+h+Sep+tag+Sep+s+Sep) {",
        "\t\treturn true",
        "\t}",
        "\treturn tag == h && strings.HasPrefix(name, Prefix+h+Sep+s+Sep)",
        "}",
    ]
    return "\n".join(out) + "\n"


def emit_go_vectors(g):
    v = g["vectors"]
    out = [header(g, "//", TARGETS["go-vectors"]), ""]
    out += [
        "// The SoT's cross-language conformance vectors. The IDENTICAL cases are",
        "// generated into the Rust implementation's test module, so Go and Rust are",
        "// proven to agree on behaviour rather than assumed to.",
        "package peername",
        "",
        'import "testing"',
        "",
        "func TestIsPeerNameVectors(t *testing.T) {",
        "\tcases := []struct {",
        "\t\tname string",
        "\t\twant bool",
        "\t\twhy  string",
        "\t}{",
    ]
    for c in v["is_peer_name"]:
        out.append(f"\t\t{{{gostr(c['name'])}, {str(c['want']).lower()}, {gostr(c['why'].strip())}}},")
    out += [
        "\t}",
        "\tfor _, c := range cases {",
        "\t\tif got := IsPeerName(c.name); got != c.want {",
        "\t\t\tt.Errorf(\"IsPeerName(%q) = %v, want %v — %s\", c.name, got, c.want, c.why)",
        "\t\t}",
        "\t}",
        "}",
        "",
        "func TestRestVectors(t *testing.T) {",
        "\tcases := []struct {",
        "\t\tname   string",
        "\t\twant   string",
        "\t\twantOK bool",
        "\t\twhy    string",
        "\t}{",
    ]
    for c in v["rest"]:
        want = c["want"]
        ok = want is not None
        out.append(f"\t\t{{{gostr(c['name'])}, {gostr(want or '')}, {str(ok).lower()}, {gostr(c['why'].strip())}}},")
    out += [
        "\t}",
        "\tfor _, c := range cases {",
        "\t\tgot, ok := Rest(c.name)",
        "\t\tif ok != c.wantOK || (ok && got != c.want) {",
        "\t\t\tt.Errorf(\"Rest(%q) = (%q, %v), want (%q, %v) — %s\", c.name, got, ok, c.want, c.wantOK, c.why)",
        "\t\t}",
        "\t}",
        "}",
        "",
        "func TestMatchLongestKeyVectors(t *testing.T) {",
        "\tcases := []struct {",
        "\t\tname string",
        "\t\tkeys []string",
        "\t\twant string",
        "\t\twhy  string",
        "\t}{",
    ]
    for c in v["match_longest_key"]:
        keys = ", ".join(gostr(k) for k in c["keys"])
        want = c["want"] or ""
        out.append(f"\t\t{{{gostr(c['name'])}, []string{{{keys}}}, {gostr(want)}, {gostr(c['why'].strip())}}},")
    out += [
        "\t}",
        "\tfor _, c := range cases {",
        "\t\tif got := MatchLongestKey(c.name, c.keys); got != c.want {",
        "\t\t\tt.Errorf(\"MatchLongestKey(%q, %v) = %q, want %q — %s\", c.name, c.keys, got, c.want, c.why)",
        "\t\t}",
        "\t}",
        "}",
        "",
        "func TestMintVectors(t *testing.T) {",
        "\tcases := []struct {",
        "\t\thandle, tag, site, date string",
        "\t\twant                    string",
        "\t\twhy                     string",
        "\t}{",
    ]
    for c in v["mint"]:
        out.append(
            f"\t\t{{{gostr(c['handle'])}, {gostr(c['org_short_tag'])}, {gostr(c['site'])}, "
            f"{gostr(c['date'])}, {gostr(c['want'])}, {gostr(c['why'].strip())}}},"
        )
    out += [
        "\t}",
        "\tfor _, c := range cases {",
        "\t\tif got := Mint(c.handle, c.tag, c.site, c.date); got != c.want {",
        "\t\t\tt.Errorf(\"Mint(%q, %q, %q, %q) = %q, want %q — %s\", c.handle, c.tag, c.site, c.date, got, c.want, c.why)",
        "\t\t}",
        "\t}",
        "}",
        "",
        "func TestBelongsToVectors(t *testing.T) {",
        "\tcases := []struct {",
        "\t\tname, handle, tag, site string",
        "\t\twant                    bool",
        "\t\twhy                     string",
        "\t}{",
    ]
    for c in v["belongs_to"]:
        out.append(
            f"\t\t{{{gostr(c['name'])}, {gostr(c['handle'])}, {gostr(c['org_short_tag'])}, "
            f"{gostr(c['site'])}, {str(c['want']).lower()}, {gostr(c['why'].strip())}}},"
        )
    out += [
        "\t}",
        "\tfor _, c := range cases {",
        "\t\tif got := BelongsTo(c.name, c.handle, c.tag, c.site); got != c.want {",
        "\t\t\tt.Errorf(\"BelongsTo(%q, %q, %q, %q) = %v, want %v — %s\", c.name, c.handle, c.tag, c.site, got, c.want, c.why)",
        "\t\t}",
        "\t}",
        "}",
    ]
    return "\n".join(out) + "\n"


# ----------------------------------------------------------------- Rust -----

def emit_rust(g):
    prefix, sep, form = g["prefix"], g["separator"], g["form"]
    v = g["vectors"]
    out = ["// SPDX-License-Identifier: Apache-2.0", "//"]
    out += header(g, "//", TARGETS["rust"]).split("\n")
    out += [
        "//",
        "// The single implementation for every Rust consumer: goat-cli's onboard mint +",
        "// native-revoke selector, and microscope's spine join.",
        "",
        f"/// The literal that opens every operator-onboarded peer name.",
        f"pub const PREFIX: &str = {ruststr(prefix)};",
        "",
        "/// Field separator, and the terminator that anchors a keyed match.",
        f"pub const SEP: &str = {ruststr(sep)};",
        "",
        "/// The human-readable grammar, for operator-facing text.",
        f"pub const FORM: &str = {ruststr(form)};",
        "",
        "/// The grammar's semver. A change to PREFIX, SEP, or field order renames the",
        "/// whole fleet and is a MAJOR bump plus a migration.",
        f"pub const VERSION: &str = {ruststr(g['version'])};",
        "",
    ]

    out += [doc_lines(rule_doc(g, "is_peer_name"), "/// ")]
    out += [
        "pub fn is_peer_name(name: &str) -> bool {",
        "    name.starts_with(PREFIX)",
        "}",
        "",
    ]

    out += [doc_lines(rule_doc(g, "rest"), "/// ")]
    out += [
        "pub fn rest(name: &str) -> Option<&str> {",
        "    name.strip_prefix(PREFIX)",
        "}",
        "",
    ]

    out += [doc_lines(rule_doc(g, "match_longest_key"), "/// ")]
    out += [
        "pub fn match_longest_key<'a>(name: &str, keys: &[&'a str]) -> Option<&'a str> {",
        "    let rest = rest(name)?;",
        "    let mut best: Option<&'a str> = None;",
        "    let mut best_len = 0usize;",
        "    for k in keys {",
        "        if k.is_empty() || !rest.starts_with(&format!(\"{k}{SEP}\")) {",
        "            continue;",
        "        }",
        "        if best.is_none() || k.len() > best_len {",
        "            best = Some(k);",
        "            best_len = k.len();",
        "        }",
        "    }",
        "    best",
        "}",
        "",
    ]

    out += [doc_lines(rule_doc(g, "mint"), "/// ")]
    out += [
        "pub fn mint(handle: &str, org_short_tag: &str, site: &str, date: &str) -> String {",
        "    let h = handle.trim();",
        "    let tag = org_short_tag.trim();",
        "    // A tag equal to the handle carries no information and is omitted.",
        "    // On per-uuid goatnets `short_tag` is FORCED to equal the handle (it",
        "    // doubles as the Zenoh prefix), which minted names like",
        "    // `peer-user371x-user371x-zagros-...` — 23 of 111 peers",
        "    // on such a goatnet. Same degenerate value the Ory org trait corrects.",
        "    let tag = if tag == h { \"\" } else { tag };",
        "    let tag_seg = if tag.is_empty() {",
        "        String::new()",
        "    } else {",
        "        format!(\"{SEP}{tag}\")",
        "    };",
        "    format!(",
        "        \"{PREFIX}{handle}{tag_seg}{SEP}{site}{SEP}{date}\",",
        "        handle = h,",
        "        site = site.trim(),",
        "        date = date.trim(),",
        "    )",
        "}",
        "",
    ]

    out += [doc_lines(rule_doc(g, "belongs_to"), "/// ")]
    out += [
        "pub fn belongs_to(name: &str, handle: &str, org_short_tag: &str, site: &str) -> bool {",
        "    let (h, s, tag) = (handle.trim(), site.trim(), org_short_tag.trim());",
        "    if h.is_empty() || s.is_empty() {",
        "        return false;",
        "    }",
        "    if tag.is_empty() {",
        "        return name.starts_with(&format!(\"{PREFIX}{h}{SEP}{s}{SEP}\"));",
        "    }",
        "    // BOTH forms, and this is a migration requirement rather than",
        "    // leniency. `mint` stopped emitting a tag equal to the handle, but",
        "    // every peer minted BEFORE that still carries the doubled form. A",
        "    // selector that accepted only the new shape would silently skip",
        "    // those peers — and this is the function a REVOKE CUT uses, so",
        "    // \"silently skip\" means a credential that was supposed to be cut",
        "    // stays live. Accept the doubled form until no peer carries it.",
        "    if name.starts_with(&format!(\"{PREFIX}{h}{SEP}{tag}{SEP}{s}{SEP}\")) {",
        "        return true;",
        "    }",
        "    tag == h && name.starts_with(&format!(\"{PREFIX}{h}{SEP}{s}{SEP}\"))",
        "}",
        "",
        "#[cfg(test)]",
        "// rustfmt would reflow these one-line cases and the byte-parity fence would then",
        "// report drift the SoT did not cause; the generator owns this block's layout.",
        "#[rustfmt::skip]",
        "mod conformance_vectors {",
        "    // The SoT's cross-language vectors — the IDENTICAL cases the Go",
        "    // implementation's generated test runs, so the two are proven to agree.",
        "    use super::*;",
        "",
        "    #[test]",
        "    fn is_peer_name_vectors() {",
    ]
    for c in v["is_peer_name"]:
        neg = "" if c["want"] else "!"
        out.append(
            f"        assert!({neg}is_peer_name({ruststr(c['name'])}), {ruststr(c['why'].strip())});"
        )
    out += ["    }", "", "    #[test]", "    fn rest_vectors() {"]
    for c in v["rest"]:
        want = "None" if c["want"] is None else f"Some({ruststr(c['want'])})"
        out.append(f"        assert_eq!(rest({ruststr(c['name'])}), {want}, {ruststr(c['why'].strip())});")
    out += ["    }", "", "    #[test]", "    fn match_longest_key_vectors() {"]
    for c in v["match_longest_key"]:
        keys = ", ".join(ruststr(k) for k in c["keys"])
        want = "None" if c["want"] is None else f"Some({ruststr(c['want'])})"
        out.append(
            f"        assert_eq!(match_longest_key({ruststr(c['name'])}, &[{keys}]), {want}, "
            f"{ruststr(c['why'].strip())});"
        )
    out += ["    }", "", "    #[test]", "    fn mint_vectors() {"]
    for c in v["mint"]:
        out.append(
            f"        assert_eq!(mint({ruststr(c['handle'])}, {ruststr(c['org_short_tag'])}, "
            f"{ruststr(c['site'])}, {ruststr(c['date'])}), {ruststr(c['want'])}, "
            f"{ruststr(c['why'].strip())});"
        )
    out += ["    }", "", "    #[test]", "    fn belongs_to_vectors() {"]
    for c in v["belongs_to"]:
        neg = "" if c["want"] else "!"
        out.append(
            f"        assert!({neg}belongs_to({ruststr(c['name'])}, {ruststr(c['handle'])}, "
            f"{ruststr(c['org_short_tag'])}, {ruststr(c['site'])}), "
            f"{ruststr(c['why'].strip())});"
        )
    out += ["    }", "}"]
    return "\n".join(out) + "\n"


EMITTERS = {"go": emit_go, "go-vectors": emit_go_vectors, "rust": emit_rust}


# ------------------------------------------------------------------ main ----

def check(g):
    ok = True
    for target, rel in TARGETS.items():
        path = REPO / rel
        want = EMITTERS[target](g)
        if not path.exists():
            print(f"FAIL {target}: missing generated file {rel}")
            ok = False
            continue
        have = path.read_text()
        if have != want:
            print(f"FAIL {target}: {rel} has drifted from {SOT_REL}")
            import difflib
            for line in list(difflib.unified_diff(
                    have.splitlines(), want.splitlines(),
                    fromfile=f"{rel} (checked in)", tofile=f"{rel} (from SoT)", lineterm=""))[:60]:
                print("  " + line)
            ok = False
    if ok:
        print(f"OK: every peer-name consumer reproduces {SOT_REL} v{g['version']} byte-for-byte.")
    return ok


def write(g):
    for target, rel in TARGETS.items():
        path = REPO / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(EMITTERS[target](g))
        print(f"wrote {rel}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    grp = ap.add_mutually_exclusive_group(required=True)
    grp.add_argument("--check", action="store_true")
    grp.add_argument("--write", action="store_true")
    grp.add_argument("--list", action="store_true")
    grp.add_argument("--emit", metavar="TARGET", choices=sorted(TARGETS))
    args = ap.parse_args()

    g = load()
    if args.list:
        for t, rel in TARGETS.items():
            print(f"{t}\t{rel}")
        return 0
    if args.emit:
        sys.stdout.write(EMITTERS[args.emit](g))
        return 0
    if args.write:
        write(g)
        return 0
    return 0 if check(g) else 1


if __name__ == "__main__":
    sys.exit(main())
