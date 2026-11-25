#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render the role's swanctl connection from a sample declaration, for CI to load into a real charon.

    render_ipsec_sample.py psk|pubkey|responder|route <out.conf>

Uses test_ipsec.py's sample declarations (documentation addresses, a throwaway key). CI then runs
`swanctl --load-all` with charon up, which parses the file the way a boundary host will.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_ipsec as t  # noqa: E402


def main():
    kind, out = sys.argv[1], sys.argv[2]
    decl, env = {"psk": (t.PSK_DECL, t.ENV), "pubkey": (t.PUBKEY_DECL, t.ENV), "responder": (t.RESPONDER_DECL, t.RESPONDER_ENV),
                 "route": (t.ROUTE_DECL, t.ROUTE_ENV)}[kind]
    res = t.ipsec.simple_bn_ipsec(t.declaration(**decl), env)
    if res["errors"]:
        sys.exit("; ".join(res["errors"]))
    text = t.Templates().render("swanctl-simple-bn.conf.j2", _sbn_ipsec=res)
    with open(out, "w") as f:
        f.write(text)
    os.chmod(out, 0o600)


if __name__ == "__main__":
    main()
