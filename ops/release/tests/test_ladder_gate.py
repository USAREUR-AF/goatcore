#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_ladder_gate.py — a ref reaches a higher rung only after the rung below
# runs it (ADR 1120, GB-373).
#
# ops/release/ladder_gate.py is called by deploy-component.sh before it touches a
# host and by ansible-preflight.sh before a direct play. It reads the ladder
# (ops/release/ladder.yaml), fleet-pins and the rendered pins of the lower rung,
# and the hotfix records. These tests build a synthetic repository (goatnets
# zagros_dev, zagros, zagros_prod) and drive the module directly.
#
# Plain python3 + asserts, the convention of test_readiness.py. Exit codes: 0 all
# pass, 1 one or more failed.

import importlib.util
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
RELEASE = os.path.normpath(os.path.join(HERE, ".."))
spec = importlib.util.spec_from_file_location("ladder_gate", os.path.join(RELEASE, "ladder_gate.py"))
lg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lg)

OLD = "ghcr.io/example/app@sha256:" + "a" * 64
NEW = "ghcr.io/example/app@sha256:" + "b" * 64
FAILURES = []


def check(name, cond, detail=""):
    if cond:
        print("ok   %s" % name)
    else:
        FAILURES.append(name)
        print("FAIL %s %s" % (name, detail))


def make_repo(dev=NEW, pre=NEW, prod=NEW, ladder=True, records=None, dev_absent=False):
    root = tempfile.mkdtemp(prefix="ladder-gate-")
    os.makedirs(os.path.join(root, "ops/release/records/app"))
    os.makedirs(os.path.join(root, "ops/pins"))
    for g, v in (("zagros_dev_hosts", dev), ("zagros_hosts", pre), ("zagros_prod_hosts", prod)):
        d = os.path.join(root, "ops/ansible/group_vars", g)
        os.makedirs(d)
        if v is not None and not (dev_absent and g == "zagros_dev_hosts"):
            with open(os.path.join(d, "pins.yml"), "w") as f:
                f.write('app_image: "%s"\n' % v)
    with open(os.path.join(root, "ops/pins/fleet-pins.yaml"), "w") as f:
        f.write(
            "goatnets:\n"
            "  zagros_dev: {members: [zagros_dev_hosts]}\n"
            "  zagros: {members: [zagros_hosts]}\n"
            "  zagros_prod: {members: [zagros_prod_hosts]}\n"
            "components:\n"
            "  app: {var: app_image, fleet: \"%s\", deploy_playbook: app-deploy.yml}\n" % OLD)
    with open(os.path.join(root, "ops/release/ladder.yaml"), "w") as f:
        if ladder:
            f.write("rungs:\n"
                    "  - {name: dev, goatnet: zagros_dev, status: live}\n"
                    "  - {name: preprod, goatnet: zagros, status: live}\n"
                    "  - {name: prod, goatnet: zagros_prod, status: live}\n"
                    "hops: []\n")
        else:
            f.write("rungs: []\nhops: []\n")
    for name, body in (records or {}).items():
        with open(os.path.join(root, "ops/release/records/app", name), "w") as f:
            f.write(body)
    return root


def decide(root, goatnet, ref=NEW, reason="", ack=""):
    return lg.decide(root, "app", goatnet, lg.digest_of(ref), skip_reason=reason, skip_ack=ack)


# 1. A prod deploy of a ref the preprod rung does not pin is refused, naming the rung and the command.
r = make_repo(pre=OLD)
d = decide(r, "zagros_prod")
check("prod refused when preprod pins another ref", d["decision"] == "refuse", d)
check("refusal carries GB-373", "GB-373" in d["message"], d["message"])
check("refusal names the lower rung and goatnet", "preprod" in d["message"] and "zagros" in d["message"], d["message"])
check("refusal names the command to run first",
      "deploy-component.sh app --goatnet zagros" in d["message"], d["message"])
shutil.rmtree(r)

# 2. Once preprod pins the same ref, the static gate passes and asks the caller to confirm convergence there.
r = make_repo()
d = decide(r, "zagros_prod")
check("prod allowed statically when preprod pins the ref", d["decision"] == "lower", d)
check("the lower goatnet to verify is preprod", d.get("lower_goatnet") == "zagros", d)
shutil.rmtree(r)

# 3. The lowest rung has no rung below it.
r = make_repo()
d = decide(r, "zagros_dev")
check("dev rung always passes", d["decision"] == "pass", d)
# preprod checks against dev
d = decide(r, "zagros")
check("preprod checks dev", d["decision"] == "lower" and d.get("lower_goatnet") == "zagros_dev", d)
shutil.rmtree(r)

# 4. --skip-rung without a reason, or without an ack, is refused; with both it passes and says so.
r = make_repo(pre=OLD)
d = decide(r, "zagros_prod", reason="", ack="operator")
check("skip without reason refused", d["decision"] == "refuse" and "--reason" in d["message"], d)
d = decide(r, "zagros_prod", reason="prod is down", ack="")
check("skip without ack refused", d["decision"] == "refuse" and "--ack" in d["message"], d)
d = decide(r, "zagros_prod", reason="prod is down", ack="operator")
check("skip with reason and ack passes", d["decision"] == "pass" and d.get("exception") == "skip-rung", d)
shutil.rmtree(r)

# 5. An open hotfix record for this component and ref is the recorded exception.
rec = ("component: app\nclass: hotfix\ncandidate_ref: %s\nstate: candidate\n"
       "reason: urgent\noperator_ack: operator\nopened: '2026-01-01'\n") % NEW
r = make_repo(pre=OLD, records={"20260101-hotfix-bbbbbbbbbbbb.yaml": rec})
d = decide(r, "zagros_prod")
check("hotfix record allows the prod deploy", d["decision"] == "pass" and d.get("exception") == "hotfix-record", d)
shutil.rmtree(r)
# an abandoned hotfix record does not
r = make_repo(pre=OLD, records={"x.yaml": rec.replace("state: candidate", "state: abandoned")})
d = decide(r, "zagros_prod")
check("abandoned hotfix record does not allow", d["decision"] == "refuse", d)
shutil.rmtree(r)

# 6. A rung where the component is not pinned is passed over; the next one down decides.
r = make_repo(pre=None, dev=NEW)
d = decide(r, "zagros_prod")
check("absent preprod falls to dev", d["decision"] == "lower" and d.get("lower_goatnet") == "zagros_dev", d)
r2 = make_repo(pre=None, dev=OLD)
d = decide(r2, "zagros_prod")
check("absent preprod, dev on another ref: refused", d["decision"] == "refuse", d)
shutil.rmtree(r); shutil.rmtree(r2)
# no lower rung runs it at all: refused fail-closed
r = make_repo(pre=None, dev=None)
d = decide(r, "zagros_prod")
check("no lower rung runs it: refused", d["decision"] == "refuse" and "--skip-rung" in d["message"], d)
shutil.rmtree(r)

# 7. No ladder declared, or a goatnet that is not on it: nothing to enforce.
r = make_repo(pre=OLD, ladder=False)
d = decide(r, "zagros_prod")
check("empty ladder passes", d["decision"] == "pass", d)
shutil.rmtree(r)

# 8. Play mode: a direct play maps to its component and goatnet, and an -e override of the image var is what is judged.
r = make_repo(pre=OLD)
res = lg.play_decisions(r, ["-i", "inv.ini", "app-deploy.yml", "-e", "target=zagros_prod_hosts"])
check("play mode refuses the prod play when preprod differs",
      any(x["decision"] == "refuse" for x in res), res)
res = lg.play_decisions(r, ["app-deploy.yml", "-e", "target=zagros_prod_hosts", "-e", "app_image=" + OLD])
check("play mode judges the -e override (OLD is on preprod)",
      res and all(x["decision"] != "refuse" for x in res), res)
res = lg.play_decisions(r, ["app-deploy.yml", "-e", json.dumps({"target": "zagros_prod_hosts", "app_image": NEW})])
check("play mode reads a JSON -e", any(x["decision"] == "refuse" for x in res), res)
res = lg.play_decisions(r, ["other-play.yml", "-e", "target=zagros_prod_hosts"])
check("a play that deploys no fleet component is not gated", res == [], res)
res = lg.play_decisions(r, ["app-deploy.yml", "-e", "target=zagros_prod_hosts", "--list-tasks"])
check("a listing run is not gated", res == [], res)
shutil.rmtree(r)

print("\n%d failure(s)" % len(FAILURES))
sys.exit(1 if FAILURES else 0)
