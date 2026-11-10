#!/usr/bin/env python3
"""acl-evaluate.py — decide what a zenoh 1.9.0 ACL actually permits, without a
router.

WHY THIS EXISTS. Every guard in this role so far checks the SHAPE of the
rendered config (a rule id is present, a subject enumerates a CN). None of them
can answer the only question that matters when the universal read is narrowed:

    "if this peer declares a subscriber on <key>, does the router allow it?"

That question is decided by zenoh's key-expression INCLUSION test, which is not
string matching and not the intersection test people reach for by reflex:
`<anchor>/**` INTERSECTS `**` (they share keys) but does NOT INCLUDE it, so a
rule on `<anchor>/**` refuses a declared `**`. Getting that backwards is how a
narrowing either leaks the whole keyspace or silently blinds every reader
(a denial in zenoh is silent at both ends).

WHAT IT IMPLEMENTS. A faithful port of zenoh-keyexpr 1.9.0
`key_expr/include.rs` (`LTRIncluder`) plus the decision order of
`zenoh/net/routing/interceptor/authorization.rs` + `access_control.rs`:

  * a RULE applies to a declared key when the rule's key_expr INCLUDES it;
  * WITHIN one subject, a matching deny beats a matching allow;
  * ACROSS subjects, the first Allow wins (unset subject fields match
    everything, so a peer can match several subjects);
  * anything not allowed is denied (`default_permission: "deny"`).

The port is verified against the upstream crate's own test vectors in
`--self-test`, which is also where any future zenoh bump must be re-proven.

WHAT IT DOES NOT DO. It does not parse JSON5 with a real parser (no dependency
is available on the target hosts); it reads the rendered config with a narrow
extractor that understands exactly the shapes this role's template emits. If
the template grows a shape the extractor does not know, `--self-test` will not
catch it — the role's render tests will, because they assert on the decisions
this module returns for keys they name explicitly.

Usage:
  acl-evaluate.py --self-test
  acl-evaluate.py decide <config.json5> <subject-id> <message> <flow> <key>
  acl-evaluate.py impact <config.json5> <subject-id> <census.json>
        census.json = the router's own subscriber census, as returned by
        GET /@/*/router/subscriber/** — prints every LIVE declared key that
        the given subject would be REFUSED under this config. That is the
        pre-flight for narrowing a live goatnet: run it, read the list, and
        decide per name whether to scope the subscriber or grant it the
        fabric-wide read.
"""
import json
import re
import sys

# ---------------------------------------------------------------------------
# zenoh-keyexpr 1.9.0 key_expr/include.rs — LTRIncluder, ported chunk for chunk.
# ---------------------------------------------------------------------------
DOUBLE_WILD = "**"


def _has_direct_verbatim(expr: str) -> bool:
    """The FIRST chunk is verbatim (`@`-prefixed). Verbatim chunks are never
    matched by a wildcard — that is why `**` alone cannot reach `@/...` or
    `<key>/@adv/...`."""
    return expr.split("/", 1)[0].startswith("@")


def _has_verbatim(expr: str) -> bool:
    """ANY chunk is verbatim."""
    return any(c.startswith("@") for c in expr.split("/"))


def _chunk_includes(lchunk: str, rchunk: str) -> bool:
    """non_double_wild_chunk_includes: does the left chunk's set include the
    right chunk's set? `**` on the right is handled by the caller."""
    if lchunk == rchunk:
        return True
    if lchunk.startswith("@") or rchunk.startswith("@"):
        return False  # verbatim, and not identical
    if lchunk == "*":
        return True
    if "$" in lchunk:
        # `a$*b` — prefix/suffix/needle walk, exactly as upstream does it.
        parts = lchunk.split("$*")
        if not rchunk.startswith(parts[0]):
            return False
        rest = rchunk[len(parts[0]):]
        if not rest.endswith(parts[-1]):
            return False
        rest = rest[: len(rest) - len(parts[-1])] if parts[-1] else rest
        for needle in parts[1:-1]:
            idx = rest.find(needle)
            if idx < 0:
                return False
            rest = rest[idx + len(needle):]
        return True
    return False


def _includes_slice(left: str, right: str) -> bool:
    while True:
        lchunk, _, lrest = left.partition("/")
        lempty = lrest == ""
        if lchunk == DOUBLE_WILD:
            if (lempty and not _has_verbatim(right)) or (
                not lempty and _includes_slice(lrest, right)
            ):
                return True
            if _has_direct_verbatim(right):
                return False
            right = right.partition("/")[2]
            if right == "":
                return False
        else:
            rchunk, _, rrest = right.partition("/")
            if rchunk == "" or rchunk == DOUBLE_WILD or not _chunk_includes(lchunk, rchunk):
                return False
            if lempty:
                return rrest == ""
            left, right = lrest, rrest


def includes(left: str, right: str) -> bool:
    """True when the key set described by `left` INCLUDES the one described by
    `right` — i.e. when a rule with key_expr `left` applies to a declaration on
    `right`."""
    return left == right or _includes_slice(left, right)


# ---------------------------------------------------------------------------
# Canon form — the constraint that decides which rules can exist at all.
# ---------------------------------------------------------------------------
# A zenoh key expression must be in CANON FORM. `key_exprs` in the ACL config
# deserialize as OwnedKeyExpr, which rejects a non-canon string, so a router
# handed one DOES NOT START; a client rejects it the same way at declare time.
#
# This matters because the obvious rule for "any wildcard filter, but never the
# whole keyspace" — `**/*/**` — is NOT canon: `**/*` canonizes to `*/**`, and
# `*/**` refuses every `**`-leading filter. Proven against the crate itself in
# crates/goat-cli/src/cmd/flow.rs::everything_except_star_star_is_not_a_key_expression.
#
# Without this check an evaluator will happily "prove" a policy that cannot be
# deployed, which is worse than proving nothing. Port of zenoh-keyexpr 1.9.0
# key_expr/canon.rs, chunk-wise.
def _canon_chunk(chunk: str) -> str:
    while "$*$*" in chunk:
        chunk = chunk.replace("$*$*", "$*")
    return "*" if chunk == "$*" else chunk


def canon(expr: str) -> str:
    """The canonical form of `expr`."""
    out, pending_double = [], False
    for chunk in expr.split("/"):
        chunk = _canon_chunk(chunk)
        if chunk == DOUBLE_WILD:
            pending_double = True
            continue
        if chunk == "*" and pending_double:
            out.append("*")          # `**/*` -> `*/**`: the ** moves right
            continue
        if pending_double:
            out.append(DOUBLE_WILD)
            pending_double = False
        out.append(chunk)
    if pending_double:
        out.append(DOUBLE_WILD)
    return "/".join(out)


def validate(expr: str):
    """None when `expr` is a usable key expression, else why it is not."""
    if expr == "" or "//" in expr or expr.startswith("/") or expr.endswith("/"):
        return "empty chunks are forbidden, as well as leading and trailing slashes"
    c = canon(expr)
    if c != expr:
        return f"not in canon form: `{expr}` canonizes to `{c}`, so zenoh refuses it"
    return None


# ---------------------------------------------------------------------------
# The rendered ACL, read back.
# ---------------------------------------------------------------------------
_RULE_RE = re.compile(
    r'\{\s*(?:id:\s*"(?P<id>[^"]+)"\s*,)(?P<body>.*?)\}\s*,', re.S
)


def _strip_comments(text: str) -> str:
    return re.sub(r"^\s*//[^\n]*$", "", text, flags=re.M)


def _list_field(body: str, name: str):
    m = re.search(name + r"\s*:\s*\[(.*?)\]", body, re.S)
    if not m:
        return None
    return re.findall(r'"([^"]+)"', m.group(1))


class Acl:
    """Rules, subjects and policies as the router would load them."""

    def __init__(self, text: str):
        text = _strip_comments(text)
        self.rules = {}      # id -> dict(messages, flows, permission, key_exprs)
        self.subjects = {}   # id -> dict(cert_common_names=[...] | None, link_protocols=...)
        self.policies = []   # list of (rule_ids, subject_ids)

        rules_blob = self._section(text, "rules")
        for m in _RULE_RE.finditer(rules_blob):
            body = m.group("body")
            perm = re.search(r'permission:\s*"(\w+)"', body)
            self.rules[m.group("id")] = {
                "messages": _list_field(body, "messages") or [],
                "flows": _list_field(body, "flows") or [],
                "permission": perm.group(1) if perm else "allow",
                "key_exprs": _list_field(body, "key_exprs") or [],
            }

        subj_blob = self._section(text, "subjects")
        for m in _RULE_RE.finditer(subj_blob):
            body = m.group("body")
            self.subjects[m.group("id")] = {
                "cert_common_names": _list_field(body, "cert_common_names"),
                "link_protocols": _list_field(body, "link_protocols"),
            }

        pol_blob = self._section(text, "policies")
        for m in re.finditer(
            r"\{\s*rules:\s*\[(?P<r>[^\]]*)\]\s*,\s*subjects:\s*\[(?P<s>[^\]]*)\]\s*,?\s*\}",
            pol_blob,
            re.S,
        ):
            self.policies.append(
                (re.findall(r'"([^"]+)"', m.group("r")), re.findall(r'"([^"]+)"', m.group("s")))
            )

    @staticmethod
    def _section(text: str, name: str) -> str:
        """The bracketed body of a top-level `<name>: [ ... ]` block."""
        i = text.index(name + ": [")
        j = i + len(name) + 2
        depth = 0
        for k in range(j, len(text)):
            if text[k] == "[":
                depth += 1
            elif text[k] == "]":
                depth -= 1
                if depth == 0:
                    return text[j: k + 1]
        raise ValueError("unterminated section: " + name)

    def unusable_key_exprs(self):
        """Every rule key_expr the router would refuse to load. A non-empty
        result means this config does not start — treat it as a hard failure,
        not a warning."""
        bad = []
        for rid, r in self.rules.items():
            for ke in r["key_exprs"]:
                why = validate(ke)
                if why:
                    bad.append((rid, ke, why))
        return bad

    def rules_for(self, subject_id):
        out = []
        for rule_ids, subject_ids in self.policies:
            if subject_id in subject_ids:
                out.extend(rule_ids)
        return out

    def decide(self, subject_id, message, flow, key):
        """`allow` / `deny` for ONE subject. Within a subject a matching deny
        beats a matching allow (authorization.rs builds a deny tree and a
        permit tree, and consults deny first)."""
        allowed = False
        for rid in self.rules_for(subject_id):
            r = self.rules.get(rid)
            if not r or message not in r["messages"] or flow not in r["flows"]:
                continue
            if not any(includes(ke, key) for ke in r["key_exprs"]):
                continue
            if r["permission"] == "deny":
                return "deny"
            allowed = True
        return "allow" if allowed else "deny"

    def subjects_for_cn(self, cn):
        """Every subject a peer presenting this CN over TLS matches. Unset
        fields match everything (a subject with only `link_protocols: ["tls"]`
        matches every mTLS peer)."""
        out = []
        for sid, s in self.subjects.items():
            if s["cert_common_names"] is not None and cn not in s["cert_common_names"]:
                continue
            if s["link_protocols"] is not None and "tls" not in s["link_protocols"]:
                continue
            out.append(sid)
        return out

    def decide_cn(self, cn, message, flow, key):
        """Across subjects the FIRST Allow wins (the decision loop in
        access_control.rs breaks on Allow), so a deny on one subject does not
        survive an allow on another."""
        for sid in self.subjects_for_cn(cn):
            if self.decide(sid, message, flow, key) == "allow":
                return "allow"
        return "deny"


def load(path) -> Acl:
    with open(path, encoding="utf-8") as fh:
        return Acl(fh.read())


# ---------------------------------------------------------------------------
# impact — what a live goatnet would lose
# ---------------------------------------------------------------------------
def census_keys(census_path):
    """Declared subscriber key-expressions with at least one CLIENT behind
    them, from the router's own admin census. Router-only rows are peer
    routers, which have their own subject and are not affected."""
    with open(census_path, encoding="utf-8") as fh:
        rows = json.load(fh)
    out = {}
    for row in rows:
        key = row["key"].split("/router/subscriber/", 1)[-1]
        clients = row.get("value", {}).get("clients") or []
        if clients:
            out.setdefault(key, set()).update(clients)
    return out


def impact(cfg_path, subject_id, census_path):
    acl = load(cfg_path)
    refused = []
    for key, clients in sorted(census_keys(census_path).items()):
        if acl.decide(subject_id, "declare_subscriber", "ingress", key) != "allow":
            refused.append((key, sorted(clients)))
    print(f"subject {subject_id}: {len(refused)} live subscription shape(s) REFUSED")
    for key, clients in refused:
        print(f"  {key}    clients: {', '.join(c[:8] + '…' for c in clients)}")
    print()
    print("Each line is a subscription that goes SILENTLY dead for any peer that")
    print("matches only this subject. Name the peer in zenoh_fabric_wide_read or")
    print("scope its subscriber before the router restarts.")
    return 0


# ---------------------------------------------------------------------------
# self-test — the upstream vectors plus the ones this change turns on
# ---------------------------------------------------------------------------
def self_test():
    fails = 0

    def ck(cond, msg):
        nonlocal fails
        print(("  PASS: " if cond else "  FAIL: ") + msg)
        if not cond:
            fails += 1

    print("== includes(): zenoh-keyexpr 1.9.0 vectors ==")
    # Straight from the crate's tests.rs / the semantics include.rs encodes.
    ck(includes("**", "**"), "** includes **")
    ck(includes("**", "a/b/c"), "** includes a concrete key")
    ck(includes("**", "*/**"), "** includes */**")
    ck(includes("*/**", "*/**"), "*/** includes itself (equality)")
    ck(not includes("*/**", "**"), "*/** does NOT include ** (a single chunk cannot cover zero)")
    ck(not includes("a/**", "**"), "a/** does NOT include **")
    ck(not includes("a/**", "*/**"), "a/** does NOT include */**")
    ck(not includes("a/**", "a$*/**"), "a/** does NOT include a$*/** (dollar-star chunk)")
    ck(includes("a/**", "a/b/c"), "a/** includes a/b/c")
    ck(includes("a/**", "a"), "a/** includes a itself")
    ck(includes("a/**", "a/**"), "a/** includes a/**")
    ck(includes("a$*/**", "ab/c"), "a$*/** includes ab/c")
    ck(includes("*", "x"), "* includes one concrete chunk")
    ck(not includes("*", "x/y"), "* does not include two chunks")
    ck(not includes("*", "**"), "* does not include **")
    # Verbatim (`@`) chunks — the reason the adminspace needs its own rule.
    ck(not includes("**", "@/x/router/subscriber/y"), "** never reaches the @ adminspace")
    ck(not includes("**", "a/@adv/pub/z"), "** never reaches an @adv key")
    ck(includes("**/@adv/**", "a/b/@adv/pub/z"), "**/@adv/** reaches an @adv key")
    ck(includes("a/**/@adv/**", "a/b/@adv/pub/z"), "a/**/@adv/** reaches a's @adv keys")
    ck(not includes("a/**/@adv/**", "b/c/@adv/pub/z"), "a/**/@adv/** does not reach b's")
    ck(includes("@/*/router/subscriber/**", "@/zid/router/subscriber/a/b"),
       "the census rule reaches the census leaves")

    print("== canon form: which rules can exist at all ==")
    ck(canon("**/*/**") == "*/**", "`**/*/**` canonizes to `*/**` — it is NOT a key expression")
    ck(validate("**/*/**") is not None, "so the evaluator refuses to reason about it")
    ck(validate("**/**") is not None, "`**/**` canonizes to `**`")
    ck(validate("**/$*/**") is not None, "`**/$*/**` canonizes to `*/**`")
    for good in ("**", "*/**", "**/basemap_vendor/**", "**/airtracks/v1/*", "*/geo/track/v1/*",
                 "*/*/*/health/v1", "ops/**", "abc/**/@adv/**", "a$*/**"):
        ck(validate(good) is None, f"`{good}` is a usable key expression")
    # The consequence, stated as a decision rather than left implicit: there is
    # no canonical rule that allows every wildcard filter and refuses `**`.
    # Any rule that includes a `**`-leading declaration must itself start with
    # `**`, and the only canonical such rules are `**` (which grants the whole
    # keyspace) or `**/<literal>/…` (which covers that one literal).
    ck(not includes("*/**", "**/basemap_vendor/**"),
       "`*/**` refuses a **-leading filter, so it cannot stand in for `**/*/**`")
    ck(includes("**/basemap_vendor/**", "**/basemap_vendor/**") and not includes("**/basemap_vendor/**", "**"),
       "an enumerated cross-tenant shape allows itself and still refuses `**`")

    print("== decide(): a hand-built ACL ==")
    cfg = '''
    rules: [
      { id: "wide", messages: ["declare_subscriber"], flows: ["ingress"],
        permission: "allow", key_exprs: ["**"] },
      { id: "scoped", messages: ["declare_subscriber"], flows: ["ingress"],
        permission: "allow", key_exprs: ["aaa/**", "ops/**"] },
      { id: "deny-audit", messages: ["declare_subscriber"], flows: ["ingress"],
        permission: "deny", key_exprs: ["ops/audit/operator/**"] },
    ],
    subjects: [
      { id: "peer", link_protocols: ["tls"] },
      { id: "named", cert_common_names: ["panoscope-trial-peer"] },
    ],
    policies: [
      { rules: ["scoped", "deny-audit"], subjects: ["peer"] },
      { rules: ["wide", "deny-audit"], subjects: ["named"] },
    ],
    '''
    acl = Acl(cfg)
    ck(acl.decide("peer", "declare_subscriber", "ingress", "aaa/x/**") == "allow",
       "scoped subject may declare its own namespace")
    ck(acl.decide("peer", "declare_subscriber", "ingress", "**") == "deny",
       "scoped subject is REFUSED **")
    ck(acl.decide("peer", "declare_subscriber", "ingress", "*/**") == "deny",
       "scoped subject is REFUSED */**")
    ck(acl.decide("named", "declare_subscriber", "ingress", "**") == "allow",
       "named subject may declare **")
    ck(acl.decide("named", "declare_subscriber", "ingress", "ops/audit/operator/**") == "deny",
       "a deny inside the subject beats that subject's own ** allow")
    ck(acl.decide("peer", "put", "egress", "aaa/x") == "deny",
       "a message the rule does not carry is denied")
    ck(acl.decide_cn("panoscope-trial-peer", "declare_subscriber", "ingress", "**") == "allow",
       "across subjects the first Allow wins (named rescues the tls-wide subject)")
    ck(acl.decide_cn("somebody-else", "declare_subscriber", "ingress", "**") == "deny",
       "an unnamed CN matching only the tls subject is refused **")

    print()
    print("ALL EVALUATOR SELF-TESTS PASSED" if fails == 0 else f"{fails} SELF-TEST(S) FAILED")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        sys.exit(self_test())
    if len(sys.argv) >= 6 and sys.argv[1] == "decide":
        _, _, cfg, sid, msg, flow, key = sys.argv[:7]
        print(load(cfg).decide(sid, msg, flow, key))
        sys.exit(0)
    if len(sys.argv) == 5 and sys.argv[1] == "impact":
        sys.exit(impact(sys.argv[2], sys.argv[3], sys.argv[4]))
    print(__doc__)
    sys.exit(2)
