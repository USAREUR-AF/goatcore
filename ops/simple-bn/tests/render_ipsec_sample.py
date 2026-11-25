#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render the role's swanctl connection from a sample declaration, for CI to load into a real charon.

    render_ipsec_sample.py psk|pubkey|responder <out.conf>

Uses test_ipsec.py's sample declarations (documentation addresses, a throwaway key). CI then runs
`swanctl --load-all` with charon up, which parses the file the way a boundary host will.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_ipsec as t  # noqa: E402


def main():
    kind, out = sys.argv[1], sys.argv[2]
    decl = {"psk": t.PSK_DECL, "pubkey": t.PUBKEY_DECL, "responder": t.RESPONDER_DECL}[kind]
    res = t.ipsec.simple_bn_ipsec(t.declaration(**decl), t.RESPONDER_ENV if kind == "responder" else t.ENV)
    if res["errors"]:
        sys.exit("; ".join(res["errors"]))
    text = t.Templates().render("swanctl-simple-bn.conf.j2", _sbn_ipsec=res)
    with open(out, "w") as f:
        f.write(text)
    os.chmod(out, 0o600)


if __name__ == "__main__":
    main()
