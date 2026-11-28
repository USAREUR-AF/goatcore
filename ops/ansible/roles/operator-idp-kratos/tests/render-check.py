#!/usr/bin/env python3
"""Render the operator IdP's templates and assert no operator can write their
own traits, while admin-driven enrolment still works.

WHY. traits.groups is the operator's authorization: Grafana maps the groups
claim to Grafana Admin, and the obs-traefik forwardauth passes X-User-Groups to
Prometheus, Alertmanager and Ollama. Kratos lets an identity edit its own traits
through the settings flow's `profile` method, and that method is ON unless the
config turns it off. Registration is the other way to choose your own traits,
and it is also ON by default. Traits must be written only by
ops/scripts/operator-idp-enroll.sh over the admin API (GHSA-2fjv-48gw-744g).

ENROLMENT. An enrolled identity has no credential. Its first session comes from
an admin-minted recovery code (POST /admin/recovery/code), so the recovery flow
and the code method must stay ON. code must not log in or register anyone:
passwordless_enabled and mfa_enabled stay false.

WHAT IT CHECKS. The defaults below are those of the pinned Kratos (v26.2.0,
read from the config schema embedded in the image; unchanged from v1.3.1). A method or flow whose
default is ON must be turned off EXPLICITLY; leaving the key out is a failure.
The enabled methods are exactly code and webauthn. The compose file passes the
secrets under the env names Kratos reads. The checks are proven RED against
mutated renders before their verdict on the real render is trusted.

    python3 ops/ansible/roles/operator-idp-kratos/tests/render-check.py

Exit 0 = pass, 1 = fail, 2 = jinja2/pyyaml unavailable (CI installs them).
"""
from __future__ import annotations

import copy
import sys
from pathlib import Path

try:
    import yaml
    from jinja2 import Environment, FileSystemLoader, StrictUndefined
except ImportError:
    print("ERROR: jinja2 and pyyaml required (pip install jinja2 pyyaml)", file=sys.stderr)
    sys.exit(2)

ROLE = Path(__file__).resolve().parent.parent
PINNED_IMAGE = "oryd/kratos:v26.2.0"

# Kratos v26.2.0 defaults for `enabled` (same as v1.3.1). If the pin moves, re-read these from the
# new image's embedded config schema before trusting this check.
METHOD_DEFAULT_ON = {"profile", "password", "code"}
# Methods that would let a holder log in without webauthn, or register, or
# write traits.
METHODS_OFF = ["profile", "password", "totp", "link", "lookup_secret", "passkey", "oidc"]
METHODS_ON = ["code", "webauthn"]
FLOW_DEFAULT_ON = {"registration"}
FLOWS_OFF = ["registration", "verification"]

VARS = dict(
    ansible_date_time={"iso8601": "1970-01-01T00:00:00Z"},
    operator_idp_kratos_webauthn_rp_id_prefix="ops",
    operator_idp_kratos_admin_host_port=4436,
    operator_idp_kratos_public_host_port=4435,
    operator_idp_kratos_image=PINNED_IMAGE,
    operator_idp_kratos_container_name="operator-idp-kratos",
    operator_idp_kratos_dsn="postgres://kratos:x@host.docker.internal:5436/kratos",
    operator_idp_kratos_compose_dir="/opt/operator-idp-kratos",
    operator_idp_kratos_config_subdir="config",
    site_id="zagros",
    site_dns_zone="goat.example",
)


def render(tpl: str) -> dict:
    env = Environment(loader=FileSystemLoader(str(ROLE / "templates")),
                      undefined=StrictUndefined, trim_blocks=True)
    return yaml.safe_load(env.get_template(tpl).render(**VARS))


def enabled(block, default):
    return (block or {}).get("enabled", default)


def problems(cfg: dict) -> list[str]:
    errs = []
    ss = cfg.get("selfservice") or {}
    methods = ss.get("methods") or {}
    flows = ss.get("flows") or {}
    for m in METHODS_OFF:
        block = methods.get(m)
        on = enabled(block, m in METHOD_DEFAULT_ON)
        if on is not False:
            errs.append(f"selfservice.methods.{m}.enabled is {on!r}, must be false")
        elif m in METHOD_DEFAULT_ON and (block is None or "enabled" not in block):
            errs.append(f"selfservice.methods.{m}.enabled is absent (Kratos default ON)")
    on = sorted(m for m, b in methods.items() if enabled(b, m in METHOD_DEFAULT_ON) is True)
    for m in METHOD_DEFAULT_ON - set(methods):
        on.append(m)
    if sorted(set(on)) != METHODS_ON:
        errs.append(f"enabled methods are {sorted(set(on))}, must be exactly {METHODS_ON}")
    code = methods.get("code") or {}
    for k in ("passwordless_enabled", "mfa_enabled"):
        if code.get(k, False) is not False:
            errs.append(f"selfservice.methods.code.{k} is {code.get(k)!r}, must be false "
                        "(code would log in or register without webauthn)")
    for f in FLOWS_OFF:
        block = flows.get(f)
        v = enabled(block, f in FLOW_DEFAULT_ON)
        if v is not False:
            errs.append(f"selfservice.flows.{f}.enabled is {v!r}, must be false")
        elif f in FLOW_DEFAULT_ON and (block is None or "enabled" not in block):
            errs.append(f"selfservice.flows.{f}.enabled is absent (Kratos default ON)")
    rec = flows.get("recovery") or {}
    if rec.get("enabled") is not True:
        errs.append("selfservice.flows.recovery.enabled must be true (the enrolment path)")
    if rec.get("use", "code") != "code":
        errs.append(f"selfservice.flows.recovery.use is {rec.get('use')!r}, must be code (link is off)")
    if not rec.get("ui_url"):
        errs.append("selfservice.flows.recovery.ui_url is unset (Kratos falls back to ory.sh)")
    return errs


fails = 0


def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


def mutated(cfg, path, value):
    c = copy.deepcopy(cfg)
    node = c
    for k in path[:-1]:
        node = node.setdefault(k, {})
    if value is KeyError:
        node.pop(path[-1], None)
    else:
        node[path[-1]] = value
    return c


M = ["selfservice", "methods"]
F = ["selfservice", "flows"]
cfg = render("kratos.yaml.j2")

print("self-test (each mutation must be refused):")
for label, path, value in [
    ("profile enabled", M + ["profile", "enabled"], True),
    ("profile key removed (default ON)", M + ["profile"], KeyError),
    ("registration enabled", F + ["registration", "enabled"], True),
    ("registration key removed (default ON)", F + ["registration", "enabled"], KeyError),
    ("password key removed (default ON)", M + ["password"], KeyError),
    ("code passwordless_enabled", M + ["code", "passwordless_enabled"], True),
    ("code mfa_enabled", M + ["code", "mfa_enabled"], True),
    ("oidc enabled", M + ["oidc", "enabled"], True),
    ("passkey enabled", M + ["passkey", "enabled"], True),
    ("link enabled", M + ["link", "enabled"], True),
    ("totp enabled", M + ["totp", "enabled"], True),
    ("verification enabled", F + ["verification", "enabled"], True),
    ("recovery disabled (enrolment broken)", F + ["recovery", "enabled"], False),
    ("recovery use link", F + ["recovery", "use"], "link"),
    ("code disabled (enrolment broken)", M + ["code", "enabled"], False),
]:
    check(problems(mutated(cfg, path, value)) != [], f"refuses: {label}")

print("rendered kratos.yaml:")
errs = problems(cfg)
for e in errs:
    print("    " + e)
check(not errs, "no self-service flow or method lets an operator write traits; "
                "recovery+code enrol, webauthn is the only login method")

print("rendered docker-compose.yml:")
env = (render("docker-compose.yml.j2")["services"]["kratos"].get("environment") or {})
for k in ("SECRETS_COOKIE", "SECRETS_CIPHER"):
    check(k in env, f"kratos env passes {k} (the name Kratos reads)")
check(not any(k.startswith("KRATOS_SECRETS_") for k in env),
      "no KRATOS_SECRETS_* in the container env (Kratos ignores it and refuses to start)")

defaults = yaml.safe_load((ROLE / "defaults" / "main.yml").read_text())
check(str(defaults.get("operator_idp_kratos_image", "")).split("@")[0] == PINNED_IMAGE,
      f"image pin is {PINNED_IMAGE} (the version whose defaults this check encodes)")

print("render-check: " + ("PASS" if fails == 0 else f"FAIL ({fails})"))
sys.exit(1 if fails else 0)
