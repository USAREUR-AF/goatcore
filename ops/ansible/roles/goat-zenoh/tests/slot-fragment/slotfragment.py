"""Python twin of the portal's slot-fragment renderer
(ops/goat-zenoh-portal/internal/acl/slotfragment.go, RenderSlotFragment), for
the role's render tests. golden.json5 pins the byte format both must produce;
internal/acl/slotfragment_golden_test.go checks the Go side against it.
"""
import hashlib
import re

_ANCHOR = re.compile(r"[0-9a-f]{32}")
_CN = re.compile(r"([0-9a-f]{32})(\.g[1-9][0-9]*)?")


def leaf_cn(anchor: str, gen: int) -> str:
    return anchor if gen <= 0 else f"{anchor}.g{gen}"


def render(bindings):
    """bindings: list of (anchor, cn_or_empty[, copub]) where copub is a list
    of (topic_base, live). Returns the fragment text."""
    body = []
    assigned = 0
    for b in bindings:
        anchor, cn = b[0], b[1]
        copub = len(b) > 2 and b[2]
        assert _ANCHOR.fullmatch(anchor), anchor
        if cn:
            m = _CN.fullmatch(cn)
            assert m and m.group(1) == anchor, cn
            cns = f'["{cn}"]'
            assigned += 1
        else:
            cns = f'["{anchor}.unassigned"]'  # zenoh 1.9 refuses an empty list
        body.append(f'      {{\n        id: "slot-{anchor}",\n        cert_common_names: {cns},\n      }},\n')
        for base, live in sorted(copub or [], key=lambda x: x[0]):
            key = hashlib.sha256(base.encode()).hexdigest()[:16]
            cp = cns if (cn and live) else f'["{anchor}.unassigned"]'
            body.append(f'      {{\n        id: "copub-{anchor}-{key}",\n        cert_common_names: {cp},\n      }},\n')
    text = "".join(body)
    digest = hashlib.sha256(text.encode()).hexdigest()
    head = (f"      // slot fragment sha256:{digest} ({assigned} of {len(bindings)} slots assigned; "
            f"rendered from the portal registry, GHSA-rr3p)\n")
    return head + text
