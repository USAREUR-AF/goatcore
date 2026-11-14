#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check-netbird-service-identities.py — keep the NetBird service-identity
registry, the roles that read NetBird tokens, and every goatnet's group_vars in
agreement.

Registry: ops/netbird/service-identities.yaml. Design:
docs/design/netbird-service-identities.md.

WHY. A NetBird token a service reads used to be declared in three unconnected
places: a role's template, a hand-typed env-var name in each goatnet's
group_vars, and whichever one-off script minted it (if any). Standup minted only
one of five. zagros-dev therefore ran from standup until 2026-09-16 with four
identities missing; its portal deployed green and refused every machine
enrollment. Nothing in the tree could see that, because nothing held the list.

WHAT IT REFUSES
  1. a registry entry that is malformed: missing fields, a token_env without
     {SITE}, a required_when role that does not exist, not exactly one
     bootstrap identity;
  2. a registered consumer role that does not exist here or does not include
     ops/ansible/tasks/require-netbird-service-identity.yml for that identity;
     and an external consumer (a service whose role lives in another
     repository) that does have a role here, since then step 2 must see it;
  3. an unregistered consumer: a role whose templates render a NetBird token
     variable (a name segment NB or NETBIRD followed by _TOKEN, e.g.
     ENROLL_WATCH_NB_TOKEN, NOVA_NB_API_TOKEN) but that no identity lists;
  4. group_vars drift: in a group whose site_id is S, a quoted env-var name
     ending in a registered token suffix that is not the registry's name for S
     (zagros_dev_hosts reading ZAGROS_NB_API_TOKEN, say).

Usage:
  ops/ci/check-netbird-service-identities.py [repo-root]
  ops/ci/check-netbird-service-identities.py --self-test

Exit: 0 clean · 1 a violation (each listed) · 2 self-test failure
"""
import pathlib
import re
import shutil
import sys
import tempfile

import yaml

REQUIRED_FIELDS = ("purpose", "service_user", "token_env", "token_name",
                   "required_when", "onepassword", "consumers")
TOKEN_NAME = re.compile(r"^(?:[A-Z0-9]+_)*(?:NB|NETBIRD)(?:_[A-Z0-9]+)*_TOKEN$")
ENV_LINE = re.compile(r"^\s*([A-Z][A-Z0-9_]*)=", re.M)
INCLUDE = "require-netbird-service-identity.yml"


def site_upper(site):
    return site.upper().replace("-", "_")


def check(root):
    root = pathlib.Path(root)
    problems = []
    reg_path = root / "ops/netbird/service-identities.yaml"
    roles_dir = root / "ops/ansible/roles"
    spec = yaml.safe_load(reg_path.read_text()) or {}
    idents = spec.get("identities") or {}

    # 1 — registry shape
    if spec.get("schema") != "netbird-service-identities/v1":
        problems.append(f"{reg_path}: schema must be netbird-service-identities/v1")
    boots = [k for k, v in idents.items() if v.get("bootstrap")]
    if len(boots) != 1:
        problems.append(f"{reg_path}: exactly one identity must be bootstrap: true (found {boots})")
    consumer_of = {}
    external_of = {}
    for key, ident in idents.items():
        for f in REQUIRED_FIELDS:
            if f not in ident:
                problems.append(f"{reg_path}: identity {key} lacks {f}")
        te = str(ident.get("token_env", ""))
        if not te.startswith("{SITE}_") or not TOKEN_NAME.match(te.replace("{SITE}", "SITE")):
            problems.append(f"{reg_path}: identity {key} token_env {te!r} must be {{SITE}}_..._TOKEN "
                            f"with an NB or NETBIRD segment")
        rw = ident.get("required_when")
        if rw != "always":
            if not isinstance(rw, dict) or not rw.get("var") or not rw.get("role"):
                problems.append(f"{reg_path}: identity {key} required_when must be 'always' or {{var, role}}")
            elif not (roles_dir / rw["role"]).is_dir():
                problems.append(f"{reg_path}: identity {key} required_when role {rw['role']} does not exist")
        for role in ident.get("consumers") or []:
            consumer_of.setdefault(role, []).append(key)
        for role in ident.get("external_consumers") or []:
            external_of.setdefault(role, []).append(key)
            if (roles_dir / role).is_dir():
                problems.append(f"{reg_path}: external consumer {role} of '{key}' has a role in this repository; "
                                f"list it under consumers so its require task is checked")

    # 2 — every registered consumer includes the require task for its identity
    for role, keys in sorted(consumer_of.items()):
        rdir = roles_dir / role
        if not rdir.is_dir():
            problems.append(f"{reg_path}: consumer role {role} does not exist")
            continue
        tasks = "\n".join(p.read_text() for p in sorted((rdir / "tasks").rglob("*.yml")))
        for key in keys:
            pat = re.compile(re.escape(INCLUDE) + r"[\"']?\s*\n(?:\s+.*\n)*?\s+nsi_identity:\s*" + re.escape(key) + r"\b")
            if not pat.search(tasks):
                problems.append(f"ops/ansible/roles/{role}/tasks: consumer of '{key}' does not include "
                                f"ops/ansible/tasks/{INCLUDE} with nsi_identity: {key}")

    # 3 — no unregistered consumer
    for rdir in sorted(p for p in roles_dir.iterdir() if (p / "templates").is_dir()):
        names = set()
        for t in (rdir / "templates").rglob("*.j2"):
            names.update(n for n in ENV_LINE.findall(t.read_text(errors="replace")) if TOKEN_NAME.match(n))
        if names and rdir.name not in consumer_of and rdir.name not in external_of:
            problems.append(f"ops/ansible/roles/{rdir.name}: renders {', '.join(sorted(names))} but is not a "
                            f"consumer of any identity in ops/netbird/service-identities.yaml")

    # 4 — group_vars name drift
    suffixes = sorted(((str(i["token_env"])[len("{SITE}_"):], k) for k, i in idents.items()
                       if str(i.get("token_env", "")).startswith("{SITE}_")), key=lambda s: -len(s[0]))
    if suffixes:
        # Non-greedy prefix: the SHORTEST prefix wins, so the LONGEST registered
        # suffix is the one matched. With a greedy prefix ZAGROS_DEV_WARRANT_NB_API_TOKEN
        # matched the netbird-infra suffix NB_API_TOKEN and was reported as a
        # misnamed ZAGROS_DEV_NB_API_TOKEN.
        quoted = re.compile(r"['\"]([A-Z][A-Z0-9_]*?_(" + "|".join(re.escape(s) for s, _ in suffixes) + r"))['\"]")
        gv = root / "ops/ansible/group_vars"
        for gdir in sorted(p for p in gv.iterdir() if p.is_dir()) if gv.is_dir() else []:
            site = None
            for f in sorted(gdir.glob("*.yml")):
                data = yaml.safe_load(f.read_text()) or {}
                if isinstance(data, dict) and data.get("site_id"):
                    site = str(data["site_id"])
            if not site:
                continue
            for f in sorted(gdir.glob("*.yml")):
                for n, line in enumerate(f.read_text().splitlines(), 1):
                    if line.lstrip().startswith("#"):
                        continue
                    for name, suffix in quoted.findall(line):
                        want = f"{site_upper(site)}_{suffix}"
                        if name != want:
                            problems.append(f"{f.relative_to(root)}:{n}: {name} on site_id {site}; "
                                            f"the registry names it {want}")
    return problems


def self_test():
    real = pathlib.Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d)
        (root / "ops/netbird").mkdir(parents=True)
        shutil.copy(real / "ops/netbird/service-identities.yaml", root / "ops/netbird/")
        spec = yaml.safe_load((root / "ops/netbird/service-identities.yaml").read_text())
        roles = root / "ops/ansible/roles"
        for ident in spec["identities"].values():
            for role in ident["consumers"] + ([ident["required_when"]["role"]] if isinstance(ident["required_when"], dict) else []):
                (roles / role / "tasks").mkdir(parents=True, exist_ok=True)
        # every consumer wired correctly ...
        for key, ident in spec["identities"].items():
            for role in ident["consumers"]:
                t = roles / role / "tasks/main.yml"
                t.write_text((t.read_text() if t.exists() else "") +
                             f"- ansible.builtin.include_tasks: \"{{{{ role_path }}}}/../../tasks/{INCLUDE}\"\n"
                             f"  vars:\n    nsi_identity: {key}\n")
        # ... except: onboard-runner skips the include (2), an external consumer has a role
        # here (2), an unregistered role renders a token (3), and zagros_dev_hosts reads
        # zagros's name (4)
        (roles / "onboard-runner/tasks/main.yml").write_text("- debug: {msg: no gate}\n")
        external = next(r for i in spec["identities"].values() for r in i.get("external_consumers") or [])
        (roles / external / "tasks").mkdir(parents=True)
        (roles / "rogue/templates").mkdir(parents=True)
        (roles / "rogue/templates/env.j2").write_text("ROGUE_NB_TOKEN={{ x }}\nGITHUB_ONBOARD_TOKEN={{ y }}\n")
        g = root / "ops/ansible/group_vars/zagros_dev_hosts"
        g.mkdir(parents=True)
        (g / "vars.yml").write_text("site_id: zagros-dev\nnetvis_netbird_token: \"{{ lookup('env', 'ZAGROS_NB_API_TOKEN') }}\"\n"
                                    "portal_netbird_api_token_envvar: \"ZAGROS_DEV_PORTAL_NB_SVC_TOKEN\"\n"
                                    # a registered suffix that ENDS in another one (WARRANT_NB_API_TOKEN
                                    # ends in NB_API_TOKEN) is matched on the longer suffix and passes
                                    + ("warrant_membership_token_env: \"ZAGROS_DEV_WARRANT_NB_API_TOKEN\"\n"
                                       if "warrant-membership" in spec["identities"] else ""))
        got = check(root)
        want = ["roles/onboard-runner/tasks: consumer of 'netbird-infra'", "roles/rogue: renders ROGUE_NB_TOKEN",
                "ZAGROS_NB_API_TOKEN on site_id zagros-dev",
                f"external consumer {external} of 'netbird-infra' has a role in this repository"]
        missing = [w for w in want if not any(w in p for p in got)]
        extra = [p for p in got if not any(w in p for w in want)]
        if missing or extra:
            print("self-test FAILED")
            for m in missing:
                print("  not refused:", m)
            for e in extra:
                print("  unexpected:", e)
            return 2
    print("self-test ok: refused the ungated consumer, the external consumer with a role here, the "
          "unregistered role and the cross-goatnet name; "
          "passed GITHUB_ONBOARD_TOKEN, the correctly named portal variable and the "
          "longer-suffix warrant variable")
    return 0


def main(argv):
    if argv[1:2] == ["--self-test"]:
        return self_test()
    problems = check(argv[1] if len(argv) > 1 else ".")
    if problems:
        print("NetBird service identities are out of agreement:")
        for p in problems:
            print("  " + p)
        return 1
    print("ok: registry, consumer roles and group_vars agree")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
