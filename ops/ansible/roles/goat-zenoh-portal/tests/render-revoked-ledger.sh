#!/usr/bin/env bash
# GHSA-2w7v: the operator-key revocation ledger the role renders must be on a
# persistent host bind mount of the portal container and outside the
# operator-key directory. Renders the role defaults, portal.env.j2 and
# docker-compose.yml.j2 with Jinja2 (unrelated variables left undefined) and
# checks PORTAL_OPERATOR_API_KEYS_REVOKED_LEDGER against the rendered mounts.
set -euo pipefail
ROLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "$ROLE_DIR" <<'PY'
import os, re, sys, yaml, jinja2
role = sys.argv[1]
class Lenient(dict):
    """Any Ansible filter or test this check does not model passes its input
    through; the ledger and mount lines use none of them."""
    def __init__(self, base, fallback):
        super().__init__(base)
        self.fallback = fallback
    def __contains__(self, k):
        return True
    def __getitem__(self, k):
        return dict.get(self, k, self.fallback)
    def get(self, k, d=None):
        return dict.get(self, k, self.fallback)
env = jinja2.Environment(undefined=jinja2.ChainableUndefined)
env.filters = Lenient(env.filters, lambda v, *a, **k: v)
env.tests = Lenient(env.tests, lambda v, *a, **k: bool(v))
env.globals["lookup"] = lambda *a, **k: ""
env.globals["query"] = lambda *a, **k: []
env.filters["bool"] = lambda v: str(v).lower() in ("true", "yes", "1")
env.filters["to_json"] = lambda v: str(v)
env.filters["dirname"] = os.path.dirname
env.filters["regex_replace"] = lambda v, p, r="": re.sub(p, r, str(v))
env.filters["ternary"] = lambda v, a, b: a if v else b
env.filters["to_nice_yaml"] = env.filters["to_yaml"] = lambda v, **k: str(v)
env.tests["match"] = lambda v, p: bool(re.match(p, str(v)))
env.tests["search"] = lambda v, p: bool(re.search(p, str(v)))
defaults = yaml.safe_load(open(os.path.join(role, "defaults/main.yml")))
vars_ = {}
for _ in range(5):  # resolve defaults that reference other defaults
    for k, v in defaults.items():
        if not isinstance(v, str):
            vars_[k] = v
            continue
        try:  # a default unrelated to the ledger may use an Ansible-only filter
            vars_[k] = env.from_string(v).render(**vars_)
        except Exception:
            vars_[k] = v
def render(name):
    return env.from_string(open(os.path.join(role, "templates", name)).read()).render(**vars_)
penv = render("portal.env.j2")
m = re.search(r"^PORTAL_OPERATOR_API_KEYS_REVOKED_LEDGER=(.+)$", penv, re.M)
assert m, "portal.env does not render PORTAL_OPERATOR_API_KEYS_REVOKED_LEDGER"
ledger = m.group(1).strip()
keydir = re.search(r"^PORTAL_OPERATOR_API_KEYS_DIR=(.+)$", penv, re.M).group(1).strip()
compose = render("docker-compose.yml.j2")
mounts = [x.split(":")[0] for x in re.findall(r'^\s*-\s*"([^"]+:[^"]+)"', compose, re.M)]
under = lambda p, d: (p.rstrip("/") + "/").startswith(d.rstrip("/") + "/")
host = [d for d in mounts if under(ledger, d) and d.startswith("/")]
assert ledger.startswith("/"), f"ledger path {ledger!r} is not absolute"
assert host, f"ledger {ledger} is on no host bind mount of the portal container ({mounts})"
assert not under(ledger, keydir), f"ledger {ledger} is inside the operator-key dir {keydir}"
print(f"goat-zenoh-portal: revocation ledger {ledger} is on host mount {host[0]}, outside {keydir}: OK")
PY
