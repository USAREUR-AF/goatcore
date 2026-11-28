#!/usr/bin/env python3
"""The federation e2e harness, checked without Docker (CI: check-federation-e2e-harness.sh).

    python3 ops/federation-e2e/tests/test_harness.py

1. The catalog: unique scenario ids, every unit it names exists as units/<unit>.sh, every
   scenario has a pass condition, runs_where is ci or local, every check names a step.
2. The outcome rules of lib/verdict.py on synthetic records: a passing step is PASS, a failing
   one FAIL; a unit that stopped makes its later checks BLOCKED; a unit not run is SKIP; a
   mark carries its outcome and reason; a required BLOCKED or SKIP never exits 0; a scenario
   whose required checks are all NOT APPLICABLE is NOT APPLICABLE; the result validates
   against result.schema.json.
3. lib/handoff.py on the two-driver contract's own hand-off object: it reads a stop's hand-off
   and an edge run's way back, refuses a confirm code that is not the result's, fills only
   whole-element tokens, and finds each command quoted in the person's text.
4. lib/srctree.sh on a worktree that has just fetched a commit no ref holds, its objects
   loose: the clone build() makes is checked out at that commit and holds every object of it,
   so the commit stays readable after the source repository drops those loose objects; and a
   clone an earlier run left without a checkout is made again, not reused. The same from a
   shallow source repository, where git ignores --shared and --local: the clone gets no
   alternates and only the objects the source's refs reach. A sha that is not 40 lowercase
   hex (an empty one names the directory of every cached tree) is refused before anything is
   removed.
5. lib/srctree.sh's src_clone_ref, the clone the preview, the reverts and the upgrade unit
   check out a ref in: it owns its objects, so it keeps a commit after the source drops that
   commit's loose objects; it checks out a tag; a shallow source, whose history the merges
   and reverts read, is refused before anything is removed. No script of the harness makes a
   --shared clone.
6. lib/samples.py, the reader of the key-rotation scenario (federation-key-rotation.md §6):
   its recorder keeps each event's key under the flow's prefix and skips everything else; its check passes a step
   where every number arrived on time, fails a missing number and a refused release, fails
   a late number in a step with no gap, and with --gap passes a step whose numbers all
   arrived late, naming the gap, and fails one with a number lost, naming the gap's length.
   Every check units/rotation.sh marks for a refused key (rot_checks) is a check of the
   key-rotation scenario, so a refusal can never mark a check the catalog does not hold.
7. lib/hostfw.sh's edge values, rendered by roles/goat-host-firewall and read by
   lib/fwdeclared.py: the probe sessions' REST ports (PL_PORT to PL_PORT+2) are admitted
   from the docker host and from the ops host, the driver's address when FE2E_DRIVER is
   workspace (a driver on the edge reaches them over loopback), and not from an address the
   values do not name.
8. lib/joinsite.py, the judge of the join-site-plan scenario: a service declared on the node
   that a plan command builds there, or that the plan names, passes; one built only on
   another node, one neither built nor named (SILENT), and one both built and named (BOTH)
   fail; a model that places nothing on the node fails.
9. lib/joinsite_lab.py, the join-site-hosts lab: layout on the tree's fixture leaves two sites
   (alpha's node named mgmt-alpha-01, gamma's gamma-core-1) at the given addresses, with beta
   and delta gone and alpha's signal federation declared off; reach passes a plan whose
   targets are the lab's, and fails one that names the fixture's documentation address, a
   limit outside the lab, or no command at all.
10. The units that stage a probe launcher's folder (units/bilateral.sh, units/onecmd.sh,
    units/nohand-loop.sh) leave its private keys 0600 and open only its certificates to
    0644: no unit makes the whole folder world-readable.
11. lib/burst.py, the lane-pacing burst (ADR 1244), on two receiving-edge logs recorded in the
   shape the lab's edge router writes (tests/fixtures/edge-burst-*.log): `edge` counts a
   burst key as crossed only from its ingress allow on the partner link, never from the
   core link or another run's prefix, counts each downsampling drop of a burst key, gives
   the smallest gap between crossings, and counts the samples that crossed out of put order;
   `judge` passes a paced burst only when every sample crossed in put order and was stored
   with no drop at the edge, no queue_full drop at the sender and no gap under 1/hz, fails
   an unpaced one naming the drops, fails a reordered one naming the first sample out of
   order, and in the overflow case
   passes only when the sender counted what the edge never saw and warned; `metric` sums the
   republisher's queue_full series; `hz` reads the lane's rule from the edge config.
12. live-host.sh `images` and live.sh `--images-present`: an image already held at its digest
    is used as it is and never pulled, one not held is pulled, and the zenoh image is held to
    its pinned digest either way; with `--images-present`, live.sh needs no token and logs no
    host in, and without it, a missing token is still refused. FED_LIVE_PORT_BASE_PASTURE and
    _MEADOW move the laptop's forwards. prereqs keeps a Docker with its compose plugin that the
    host already runs, and installs Ubuntu's docker.io only where there is none.
    clone turns auto gc off in the repository it removes, between its init and its commit.
Exit 0 when all hold; prints one line per failure otherwise.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FAIL = []


def read_json(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def read_yaml(path):
    with open(path, encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def write_text(path, text):
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def check(cond, what):
    if not cond:
        FAIL.append(what)


def catalog_checks():
    cat = read_yaml(os.path.join(ROOT, "scenarios", "catalog.yaml"))
    ids = [s["id"] for s in cat["scenarios"]]
    check(len(ids) == len(set(ids)), "catalog: duplicate scenario ids")
    for s in cat["scenarios"]:
        check(bool((s.get("pass_condition") or "").strip()), f"catalog: {s['id']} has no pass condition")
        check(s.get("runs_where", "local") in ("ci", "local"), f"catalog: {s['id']} runs_where is not ci or local")
        units = s.get("unit")
        units = list(units.values()) if isinstance(units, dict) else ([units] if units else [])
        if not s.get("not_built"):
            check(bool(units), f"catalog: {s['id']} is built but names no unit")
        for u in units:
            check(os.path.exists(os.path.join(ROOT, "units", f"{u}.sh")), f"catalog: {s['id']} names unit {u}, no units/{u}.sh")
        for c in s.get("checks", []):
            check(bool(c.get("step")), f"catalog: {s['id']}.{c.get('id')} names no step")


def backout_edge_check():
    """The back-out removes the peering's entry from the edge's host firewall and,
    with no peering left, unloads the baseline (roles/goat-host-firewall/tasks/
    absent.yml, docs/design/peering-host-firewall.md "Taking a peering out"), so
    back-out-to-before holds the edge to its snapshot like the core: the edge check
    is required, and the scenario records no known failure."""
    cat = read_yaml(os.path.join(ROOT, "scenarios", "catalog.yaml"))
    sc = next((s for s in cat["scenarios"] if s["id"] == "back-out-to-before"), None)
    check(sc is not None, "catalog: no back-out-to-before scenario")
    if sc is None:
        return
    check(not sc.get("known_failure"), "catalog: back-out-to-before still records a known failure")
    by = {c["id"]: c for c in sc.get("checks", [])}
    for cid in ("edge-as-before", "core-as-before"):
        check(cid in by, f"catalog: back-out-to-before has no {cid} check")
        check(by.get(cid, {}).get("required", True) is not False, f"catalog: back-out-to-before's {cid} check is not required")


def write(d, name, rows):
    with open(os.path.join(d, name), "w") as fh:
        for r in rows:
            fh.write(json.dumps(r) + "\n")


def step(n, name, status):
    return {"n": n, "name": name, "status": status, "command": ["true"], "exit": 0 if status == "pass" else 1,
            "expected_exit": 0, "duration_s": 0.1, "started": "2026-01-01T00:00:00Z", "log": f"{n:02d}-{name}.log"}


def verdict_checks():
    cat = {"schema": "fed-e2e-catalog/1", "params": {"A": "xaa"}, "scenarios": [
        {"id": "all-pass", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "a", "step": "s-{A}-a"}]},
        {"id": "one-fails", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "a", "step": "s-{A}-a"}, {"id": "b", "step": "s-b"}]},
        {"id": "stopped", "title": "t", "unit": "u2", "pass_condition": "p", "checks": [{"id": "c", "step": "s-c"}]},
        {"id": "not-run", "title": "t", "unit": "u3", "pass_condition": "p", "checks": [{"id": "d", "step": "s-d"}]},
        {"id": "excluded", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "e", "step": "s-e"}]},
        {"id": "optional-fail", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "a", "step": "s-{A}-a"}, {"id": "f", "step": "s-b", "required": False}]},
        {"id": "mark-beats-pass", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "g", "step": "s-xaa-a"}]},
        {"id": "fail-beats-mark", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "h", "step": "s-b"}]},
    ]}
    with tempfile.TemporaryDirectory() as t:
        cp = os.path.join(t, "catalog.yaml")
        write_text(cp, yaml.safe_dump(cat))
        u1, u2 = os.path.join(t, "u1"), os.path.join(t, "u2")
        os.makedirs(u1); os.makedirs(u2)
        write(u1, "report.jsonl", [step(1, "s-xaa-a", "pass"), step(2, "s-b", "fail")])
        write(u1, "marks.jsonl", [{"check": "excluded.e", "outcome": "NOT APPLICABLE", "reason": "agreement offers no such flow"},
                                  {"check": "mark-beats-pass.g", "outcome": "NOT APPLICABLE", "reason": "nothing to exercise on this host"},
                                  {"check": "fail-beats-mark.h", "outcome": "BLOCKED", "reason": "an excuse"}])
        write_text(os.path.join(u2, "stopped.json"), json.dumps({"reason": "probe did not come up"}))
        out = os.path.join(t, "r.json")
        p = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1, "--record", u2,
                            "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out],
                           capture_output=True, text=True)
        check(os.path.exists(out), f"verdict wrote no result: {p.stderr[-300:]}")
        if not os.path.exists(out):
            return
        r = read_json(out)
        by = {s["id"]: s for s in r["scenarios"]}
        want = {"all-pass": "PASS", "one-fails": "FAIL", "stopped": "BLOCKED", "not-run": "SKIP", "excluded": "NOT APPLICABLE", "optional-fail": "PASS",
                "mark-beats-pass": "NOT APPLICABLE", "fail-beats-mark": "FAIL"}
        for k, v in want.items():
            check(by[k]["outcome"] == v, f"verdict: {k} is {by[k]['outcome']}, want {v}")
        check(by["excluded"]["checks"][0]["reason"] == "agreement offers no such flow", "verdict: a mark lost its reason")
        check(p.returncode == 1 and r["verdict"]["exit"] == 1, "verdict: a required FAIL, BLOCKED or SKIP exited 0")
        # Only passing and excluded scenarios: exit 0.
        p2 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1, "--only", "all-pass,excluded,optional-fail",
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out], capture_output=True, text=True)
        check(p2.returncode == 0, "verdict: PASS and NOT APPLICABLE only did not exit 0")
        # A ref that did not build: every scenario BLOCKED with the reason, and a non-zero exit.
        p4 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1,
                             "--blocked-all", "the tools did not build", "--harness-fp", "abc123",
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".4"], capture_output=True, text=True)
        r4 = read_json(out + ".4") if os.path.exists(out + ".4") else {"scenarios": []}
        check(p4.returncode == 1 and r4["scenarios"] and all(x["outcome"] == "BLOCKED" for x in r4["scenarios"]), "verdict: --blocked-all did not make every scenario BLOCKED")
        check(r4.get("harness", {}).get("fingerprint") == "abc123", "verdict: the harness fingerprint is not in the result")
        check("directory" not in r4, "verdict: a result on the pinned directory carries a directory field")
        # A directory built from a saltlick commit is named in the result.
        p5 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1,
                             "--directory-ref", "origin/example", "--directory-sha", "1" * 40,
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".5"], capture_output=True, text=True)
        r5 = read_json(out + ".5") if os.path.exists(out + ".5") else {}
        check(r5.get("directory") == {"ref": "origin/example", "sha": "1" * 40}, "verdict: --directory-ref is not in the result: " + p5.stderr[-300:])
        # A mark may not say PASS or FAIL.
        write(u2, "marks.jsonl", [{"check": "stopped.c", "outcome": "PASS", "reason": "trust me"}])
        p3 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u2,
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".3"], capture_output=True, text=True)
        check(p3.returncode != 0 and "may say only" in p3.stderr + p3.stdout, "verdict: a PASS mark was accepted")
        try:
            import jsonschema
            jsonschema.validate(read_json(out), read_json(os.path.join(ROOT, "result.schema.json")))
        except ImportError:
            FAIL.append("jsonschema is not installed")


def handoff_checks():
    """lib/handoff.py on the contract's own hand-off object (federation-two-drivers.md)."""
    h = os.path.join(ROOT, "lib", "handoff.py")
    run = lambda *a: subprocess.run([sys.executable, h, *a], capture_output=True, text=True)
    with tempfile.TemporaryDirectory() as d:
        stop = {"ok": True, "handoff": {"phase": 3, "code": "GB-NNN", "run_on": "edge",
                "command": ["goatbuilder", "federate", "xab", "--side", "edge", "--from", "3", "--to", "3",
                            "--expect-commit", "c" * 40],
                "commit": "c" * 40, "carry": [], "result": "handoff/3/edge-half.json",
                "import": ["goatbuilder", "federate", "import", "xab", "3", "{result}", "--sha256", "{code}"],
                "then": ["goatbuilder", "federate", "xab", "--from", "3"]}}
        write_text(os.path.join(d, "stop.json"), json.dumps(stop))
        r = run("next", os.path.join(d, "stop.json"))
        check(r.returncode == 0 and json.loads(r.stdout)["run_on"] == "edge", f"handoff next reads the stop: {r.stderr.strip()}")
        nocommit = json.loads(json.dumps(stop)); del nocommit["handoff"]["commit"]
        write_text(os.path.join(d, "nocommit.json"), json.dumps(nocommit))
        r = run("next", os.path.join(d, "nocommit.json"))
        check(r.returncode == 2, "handoff next refuses an edge hand-off that names no commit")
        badcarry = json.loads(json.dumps(stop)); badcarry["handoff"]["carry"] = [{"file": "agreement.json", "sha256": "a" * 64}]
        write_text(os.path.join(d, "badcarry.json"), json.dumps(badcarry))
        r = run("next", os.path.join(d, "badcarry.json"))
        check(r.returncode == 2, "handoff next refuses a carry entry without in: checkout|work")
        okcarry = json.loads(json.dumps(stop)); okcarry["handoff"]["carry"] = [{"file": "agreement.json", "in": "work", "sha256": "a" * 64}]
        write_text(os.path.join(d, "okcarry.json"), json.dumps(okcarry))
        r = run("next", os.path.join(d, "okcarry.json"))
        check(r.returncode == 0, f"handoff next reads a work carry entry: {r.stderr.strip()}")
        write_text(os.path.join(d, "none.json"), json.dumps({"ok": True}))
        r = run("next", os.path.join(d, "none.json"))
        check(r.returncode == 0 and json.loads(r.stdout)["run_on"] is None, "handoff next: a report without a hand-off is none")
        sha = "ab12" * 16
        code = " ".join(sha[:32][i:i + 4] for i in range(0, 32, 4))
        way = {"handoff": {"phase": 3, "code": "GB-NNN", "run_on": "controller", "command": None,
               "result_path": "/var/lib/goat-federation/xab/handoff/3/edge-half.json", "result_sha256": sha,
               "confirm_code": code, "import": ["goatbuilder", "federate", "import", "xab", "3", "{result}", "--sha256", code],
               "then": ["goatbuilder", "federate", "xab", "--from", "3"]}}
        write_text(os.path.join(d, "way.json"), json.dumps(way))
        r = run("wayback", os.path.join(d, "way.json"))
        check(r.returncode == 0 and json.loads(r.stdout)["confirm_code"] == code, f"handoff wayback reads the edge's code: {r.stderr.strip()}")
        way["handoff"]["confirm_code"] = "0000 " * 8
        write_text(os.path.join(d, "bad.json"), json.dumps(way))
        r = run("wayback", os.path.join(d, "bad.json"))
        check(r.returncode == 2, "handoff wayback refuses a confirm code that is not the result's")
        r = run("fill", json.dumps(stop["handoff"]["import"]), "--result", "/tmp/r.json", "--code", code)
        check(r.returncode == 0 and json.loads(r.stdout)[5:] == ["/tmp/r.json", "--sha256", code], f"handoff fill replaces whole-element tokens: {r.stderr.strip()}")
        r = run("fill", json.dumps(["x", "--file={result}"]), "--result", "/tmp/r.json")
        check(r.returncode == 2, "handoff fill refuses a token inside an element")
        write_text(os.path.join(d, "text.txt"), "continue on the edge driver:\n  goatbuilder federate xab --side edge --from 3 --to 3 --expect-commit " + "c" * 40 + "\n")
        r = run("text-holds", os.path.join(d, "text.txt"), json.dumps(stop["handoff"]["command"]))
        check(r.returncode == 0, f"handoff text-holds finds the quoted argv: {r.stderr.strip()}")
        r = run("text-holds", os.path.join(d, "text.txt"), json.dumps(stop["handoff"]["then"]))
        check(r.returncode == 2, "handoff text-holds refuses a text that lacks the argv")


def fwdeclared_checks():
    """lib/fwdeclared.py: the ports a goat-host-firewall rule set admits from one source."""
    ruleset = """table inet goat_host_firewall
delete table inet goat_host_firewall
table inet goat_host_firewall {
  chain input {
    type filter hook input priority filter; policy drop;
    iifname "lo" accept comment "loopback"
    udp sport 67 udp dport 68 accept comment "dhcp-client"
    iifname "br-*" accept comment "container-bridges"
    ip saddr { 198.51.100.10/32 } tcp dport { 22 } accept comment "ssh-management"
    ip saddr { 192.0.2.10 } udp dport { 51820 } accept comment "wg-fed0/tunnel"
    iifname "wg-fed0" ip saddr { 169.254.10.0 } tcp dport { 7448 } accept comment "wg-fed0/edge-router"
    tcp dport { 7447 } accept comment "fabric-tls"
    counter drop comment "default-deny"
  }
  chain published_new {
    ip saddr { 203.0.113.5/32 } meta l4proto tcp ct original proto-dst { 9999 } accept comment "other"
  }
}
"""
    with tempfile.TemporaryDirectory() as d:
        f = os.path.join(d, "ruleset.nft")
        write_text(f, ruleset)

        def run(saddr, iif):
            p = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "fwdeclared.py"), "--ruleset", f, "--saddr", saddr, "--iif", iif],
                               capture_output=True, text=True)
            return json.loads(p.stdout) if p.returncode == 0 else None

        r = run("198.51.100.10", "eth0")
        check(r and r["tcp"] == [22, 7447] and r["udp"] == [], f"fwdeclared: management source got {r and (r['tcp'], r['udp'])}")
        r = run("169.254.10.0", "wg-fed0")
        check(r and r["tcp"] == [7447, 7448], f"fwdeclared: the far link address on the tunnel got {r and r['tcp']}")
        r = run("169.254.10.0", "eth0")
        check(r and r["tcp"] == [7447], "fwdeclared: a rule bound to the tunnel interface admitted another interface")
        r = run("192.0.2.10", "eth1")
        check(r and r["udp"] == [51820] and r["by_port"].get("udp/51820") == ["wg-fed0/tunnel"], "fwdeclared: the tunnel port from the partner endpoint")
        r = run("203.0.113.5", "eth0")
        check(r and r["tcp"] == [7447], "fwdeclared: a rule outside chain input or a fixed expression was counted")
        write_text(f, "table inet other {}\n")
        check(run("198.51.100.10", "eth0") is None, "fwdeclared: a file without the goat_host_firewall table was read as a rule set")


def hostfw_checks():
    """lib/hostfw.sh: the edge's other-tcp override admits a side's own driver to its probe ports."""
    import re
    with open(os.path.join(ROOT, "lib", "engine.sh"), encoding="utf-8") as fh:
        engine = fh.read()
    helpers = []
    for name in ("party_pos", "octet", "core_ip", "edge_ip", "ops_ip", "partners", "probe_port"):
        m = re.search(r"^%s\(\) \{.*\}.*$" % name, engine, re.M)
        check(m is not None, f"hostfw: engine.sh defines no one-line {name}()")
        if m:
            helpers.append(m.group(0))
    m = re.search(r"^pidx\(\) \{.*?^\}$", engine, re.M | re.S)
    check(m is not None, "hostfw: engine.sh defines no pidx()")
    if not m or len(helpers) != 7:
        return
    helpers.append(m.group(0))
    render = os.path.join(os.path.dirname(ROOT), "ansible", "roles", "goat-host-firewall", "files",
                          "goat-host-firewall-render.py")
    with tempfile.TemporaryDirectory() as d:
        edge, core = os.path.join(d, "edge.yml"), os.path.join(d, "core.yml")
        script = "\n".join(['NB=10.77; PARTIES="xaa xab"; P_xaa=xab; P_xab=xaa'] + helpers +
                           ['. "%s"' % os.path.join(ROOT, "lib", "hostfw.sh"),
                            'fw_profile_vars xaa 0 "$1" "$2"',
                            'echo "$PL_PORT $(ops_ip xaa) $NB.$(octet xaa).1"'])
        p = subprocess.run(["bash", "-c", script, "hostfw", edge, core], capture_output=True, text=True)
        check(p.returncode == 0, f"hostfw: fw_profile_vars failed: {p.stderr.strip()}")
        if p.returncode != 0:
            return
        pl, ops, gw = p.stdout.split()
        pl = int(pl)
        v = read_yaml(edge)
        inp = {"schema": "goat-host-firewall-input/v1", "profile": "edge",
               "host": {"ssh_port": 22, "management_sources": v["goat_host_firewall_management_sources"],
                        "mesh_interface": "wt0", "mesh_udp_ports": []},
               "peerings": [{"name": "wg-fed0", "transport": "wireguard", "listen_port": 51820,
                             "peer_addr": "10.77.2.20", "tunnel_interface": "wg-fed0",
                             "remote_link_ip": "169.254.10.1", "edge_port": 7448, "extra_tcp_ports": [],
                             "via_forwarder": False}]}
        write_text(os.path.join(d, "in.json"), json.dumps(inp))
        write_text(os.path.join(d, "ov.json"), json.dumps(v["goat_host_firewall_override"]))
        r = subprocess.run([sys.executable, render, "--input", os.path.join(d, "in.json"),
                            "--override", os.path.join(d, "ov.json")], capture_output=True, text=True)
        check(r.returncode == 0, f"hostfw: the role refused the edge values: {r.stderr.strip()}")
        if r.returncode != 0:
            return
        rules = os.path.join(d, "edge.nft")
        write_text(rules, r.stdout)

        def tcp(saddr):
            q = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "fwdeclared.py"), "--ruleset", rules,
                                "--saddr", saddr, "--iif", "eth0"], capture_output=True, text=True)
            return set(json.loads(q.stdout)["tcp"]) if q.returncode == 0 else set()

        want = {pl, pl + 1, pl + 2}
        check(want <= tcp(gw), f"hostfw: the docker host {gw} is not admitted to the probe ports {sorted(want)}")
        check(want <= tcp(ops), f"hostfw: the ops host {ops}, a workspace driver, is not admitted to its own "
                                f"probe ports {sorted(want)} (got {sorted(tcp(ops))})")
        check(not (want & tcp("10.77.1.99")), "hostfw: an address the values do not name reaches the probe ports")
def launcher_key_mode_checks():
    """A launcher folder's private keys stay 0600; only certificates are opened to 0644."""
    import re
    for unit in ("bilateral.sh", "onecmd.sh", "nohand-loop.sh"):
        text = open(os.path.join(ROOT, "units", unit), encoding="utf-8").read()
        check(not re.search(r'chmod 0644 "\$pl"/\*(\s|;|$)', text, re.M),
              f"launcher keys: units/{unit} makes the whole launcher folder 0644, private keys included")
        check(re.search(r'chmod 0600 "\$pl"/\*-key\.pem', text) is not None,
              f"launcher keys: units/{unit} does not leave the launcher folder's private keys 0600")


def srctree_checks():
    """src_tree: the source tree of the commit under test holds every object of it."""
    with tempfile.TemporaryDirectory() as d:
        env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                   GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.org", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.org")
        git = lambda *a, cwd=d: subprocess.run(["git", *a], cwd=cwd, env=env, capture_output=True, text=True)
        git("init", "-q", "-b", "main", "upstream")
        up = os.path.join(d, "upstream")
        git("commit", "-q", "--allow-empty", "-m", "one", cwd=up)
        git("clone", "-q", "--bare", "upstream", "main.git")
        common = os.path.join(d, "main.git")
        git("worktree", "add", "-q", os.path.join(d, "wt"), "main", cwd=common)
        write_text(os.path.join(up, "f"), "the commit under test\n")
        git("add", "f", cwd=up)
        git("commit", "-q", "-m", "two", cwd=up)
        sha = git("rev-parse", "HEAD", cwd=up).stdout.strip()
        # Fetched into the worktree by sha alone: its objects are loose, and no ref holds them.
        git("fetch", "-q", up, sha, cwd=os.path.join(d, "wt"))
        loose = [os.path.join(common, "objects", o[:2], o[2:]) for o in
                 git("rev-list", "--objects", sha, "--not", "main", cwd=common).stdout.split()[::2] if len(o) == 40]
        check(loose and all(os.path.exists(p) for p in loose), f"srctree: the fetched commit's objects are not loose in the source ({loose})")

        def tree(dest):
            return subprocess.run(["bash", "-c", '. "$1"; src_tree "$2" "$3" "$4"', "_", os.path.join(ROOT, "lib", "srctree.sh"), common, sha, dest],
                                  env=env, capture_output=True, text=True)

        def holds(dest):
            head = git("rev-parse", "-q", "--verify", "HEAD", cwd=dest).stdout.strip() if os.path.isdir(dest) else ""
            readable = os.path.isdir(dest) and git("cat-file", "-e", f"{sha}^{{tree}}", cwd=dest).returncode == 0
            return head == sha and readable and os.path.exists(os.path.join(dest, "f"))

        dest = os.path.join(d, "src", sha)
        p = tree(dest)
        check(p.returncode == 0 and holds(dest), f"srctree: the clone is not checked out at the fetched commit: {p.stderr.strip()}")
        # The source drops the commit's loose objects, as git prune or gc may: the clone still has them.
        for o in loose:
            os.remove(o)
        check(os.path.isdir(dest) and git("cat-file", "-e", f"{sha}^{{tree}}", cwd=dest).returncode == 0,
              "srctree: the clone lost the commit's tree when the source dropped its loose objects (it borrows objects it does not hold)")

        # A shallow source: git clones it by transfer, with no alternates, and sends only what
        # its refs reach, so a commit fetched by sha alone is not in the clone unless fetched.
        git("clone", "-q", "--bare", "--depth", "1", "file://" + up, "shallow.git")
        sh_common = os.path.join(d, "shallow.git")
        check(os.path.exists(os.path.join(sh_common, "shallow")), "srctree: the shallow source is not shallow")
        git("worktree", "add", "-q", os.path.join(d, "wt-shallow"), "main", cwd=sh_common)
        write_text(os.path.join(up, "g"), "a later commit under test\n")
        git("add", "g", cwd=up)
        git("commit", "-q", "-m", "three", cwd=up)
        sha3 = git("rev-parse", "HEAD", cwd=up).stdout.strip()
        git("fetch", "-q", up, sha3, cwd=os.path.join(d, "wt-shallow"))
        dest3 = os.path.join(d, "src", sha3)
        p = subprocess.run(["bash", "-c", '. "$1"; src_tree "$2" "$3" "$4"', "_", os.path.join(ROOT, "lib", "srctree.sh"), sh_common, sha3, dest3],
                           env=env, capture_output=True, text=True)
        head3 = git("rev-parse", "-q", "--verify", "HEAD", cwd=dest3).stdout.strip() if os.path.isdir(dest3) else ""
        check(p.returncode == 0 and head3 == sha3 and os.path.exists(os.path.join(dest3, "g")),
              f"srctree: from a shallow source, the clone could not check out a commit fetched by sha: {p.stderr.strip()}")

        # A sha that is not 40 lowercase hex is refused before anything is removed: with an
        # empty one, DEST would be the directory of every cached tree.
        sib = os.path.join(d, "src", "f" * 40)
        os.makedirs(sib, exist_ok=True)
        write_text(os.path.join(sib, "kept"), "a sibling tree\n")
        for bad in ("", "abc123", sha.upper()):
            dest_bad = os.path.join(d, "src", bad)
            p = subprocess.run(["bash", "-c", '. "$1"; src_tree "$2" "$3" "$4"', "_", os.path.join(ROOT, "lib", "srctree.sh"), common, bad, dest_bad],
                               env=env, capture_output=True, text=True)
            check(p.returncode != 0 and "not a full sha" in p.stderr, f"srctree: the sha {bad!r} was not refused: rc {p.returncode} {p.stderr.strip()}")
            check(os.path.exists(os.path.join(sib, "kept")), f"srctree: the sha {bad!r} removed a sibling tree")
            if not os.path.exists(os.path.join(sib, "kept")):
                os.makedirs(sib, exist_ok=True)
                write_text(os.path.join(sib, "kept"), "a sibling tree\n")

        # A clone an earlier run left with no checkout is made again, not reused.
        git("fetch", "-q", up, sha, cwd=os.path.join(d, "wt"))
        broken = os.path.join(d, "src", "broken")
        git("clone", "-q", "--no-checkout", common, broken)
        p = tree(broken)
        check(p.returncode == 0 and holds(broken), f"srctree: an earlier clone with no checkout was reused: {p.stderr.strip()}")


def srcclone_checks():
    """src_clone_ref: a clone at a ref that owns its objects, never from a shallow source."""
    with tempfile.TemporaryDirectory() as d:
        env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                   GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.org", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.org")
        git = lambda *a, cwd=d: subprocess.run(["git", *a], cwd=cwd, env=env, capture_output=True, text=True)
        lib = os.path.join(ROOT, "lib", "srctree.sh")

        def clone(src, ref, dest):
            return subprocess.run(["bash", "-c", '. "$1"; src_clone_ref "$2" "$3" "$4"', "_", lib, src, ref, dest],
                                  env=env, capture_output=True, text=True)

        git("init", "-q", "-b", "main", "src")
        src = os.path.join(d, "src")
        git("commit", "-q", "--allow-empty", "-m", "one", cwd=src)
        git("tag", "v1", cwd=src)
        write_text(os.path.join(src, "f"), "a commit a ref names\n")
        git("add", "f", cwd=src)
        git("commit", "-q", "-m", "two", cwd=src)
        sha = git("rev-parse", "HEAD", cwd=src).stdout.strip()
        loose = [os.path.join(src, ".git", "objects", o[:2], o[2:]) for o in
                 git("rev-list", "--objects", sha, "--not", "v1", cwd=src).stdout.split()[::2] if len(o) == 40]
        check(loose and all(os.path.exists(p) for p in loose), f"srcclone: the commit's objects are not loose in the source ({loose})")
        dest = os.path.join(d, "preview")
        p = clone(src, "main", dest)
        head = git("rev-parse", "-q", "--verify", "HEAD", cwd=dest).stdout.strip() if os.path.isdir(dest) else ""
        check(p.returncode == 0 and head == sha, f"srcclone: the clone is not checked out at the ref: rc {p.returncode} {p.stderr.strip()}")
        # A tag, as the upgrade unit checks out a release.
        tagged = os.path.join(d, "release")
        p = clone(src, "v1", tagged)
        check(p.returncode == 0 and git("describe", "--tags", "--exact-match", cwd=tagged).stdout.strip() == "v1",
              f"srcclone: the clone is not checked out at the tag: {p.stderr.strip()}")
        # The source drops the commit's loose objects, as git prune or gc may: the clone keeps them.
        for o in loose:
            os.remove(o)
        check(os.path.isdir(dest) and git("cat-file", "-e", f"{sha}^{{tree}}", cwd=dest).returncode == 0
              and git("log", "-q", "--format=%H", "-n", "2", cwd=dest).returncode == 0,
              "srcclone: the clone lost a commit when the source dropped its loose objects (it borrows objects it does not hold)")
        # A shallow source: its history is cut, so a merge or a revert there would read too
        # little. Refused, and the destination left as it was.
        git("clone", "-q", "--depth", "1", "file://" + src, "shallow")
        shallow = os.path.join(d, "shallow")
        kept = os.path.join(d, "kept")
        os.makedirs(kept)
        write_text(os.path.join(kept, "marker"), "an earlier clone\n")
        p = clone(shallow, "HEAD", kept)
        check(p.returncode != 0 and "shallow" in p.stderr, f"srcclone: a shallow source was not refused: rc {p.returncode} {p.stderr.strip()}")
        check(os.path.exists(os.path.join(kept, "marker")), "srcclone: a refused clone removed its destination")
        # An empty destination is refused before anything is removed.
        p = clone(src, "main", "")
        check(p.returncode != 0, "srcclone: an empty destination was not refused")
    # No script of the harness borrows objects from its source any more.
    shared = []
    for base, _, files in os.walk(ROOT):
        for name in files:
            if name.endswith(".sh") or name in ("env", "run.sh"):
                path = os.path.join(base, name)
                with open(path, encoding="utf-8", errors="replace") as fh:
                    for n, line in enumerate(fh, 1):
                        if "clone" in line and "--shared" in line and not line.lstrip().startswith("#"):
                            shared.append(f"{os.path.relpath(path, ROOT)}:{n}")
    check(not shared, f"srcclone: these make a --shared clone, which reads its objects from the source: {shared}")


def samples_checks():
    """lib/samples.py: the numbered samples a reader recorded, judged per step (§6)."""
    lib = os.path.join(ROOT, "lib", "samples.py")
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "arrived")
        stream = ("event: PUT\ndata: {\"key\": \"from/xaa/all/fla2b100n1\", \"value\": \"x\"}\n\n"
                  ": keep-alive\ndata: not json\ndata: {\"value\": \"no key\"}\n"
                  "data: {\"key\": \"from/xaa/all/fla2b101n2\"}\n"
                  "data: {\"key\": \"from/xaa/all/round-trip-n7\"}\n")
        p = subprocess.run([sys.executable, lib, "record", out, "--prefix", "fla2b"], input=stream, capture_output=True, text=True)
        with open(out, encoding="utf-8") as fh:
            rows = [r.split("\t") for r in fh.read().splitlines()]
        check(p.returncode == 0 and [r[1] for r in rows] == ["fla2b100n1", "fla2b101n2"],
              f"samples record: kept {[r[1:] for r in rows]}, rc {p.returncode}")

        def flow(sent, arrived):
            fs, fa = os.path.join(d, "sent"), os.path.join(d, "arr")
            write_text(fs, "".join(f"{t}\tfla2b{int(t)}n{n}\t{code}\n" for t, n, code in sent))
            write_text(fa, "".join(f"{t}\tfla2b{int(t)}n{n}\n" for t, n in arrived))
            return fs, fa

        def run(sent, arrived, *extra):
            fs, fa = flow(sent, arrived)
            p = subprocess.run([sys.executable, lib, "check", "--sent", fs, "--arrived", fa, "--from", "100", "--to", "110", *extra],
                               capture_output=True, text=True)
            return p.returncode, p.stdout.strip().splitlines()[-1] if p.stdout.strip() else p.stderr

        sent = [(100 + i, i + 1, "200") for i in range(10)]
        on_time = [(t + 0.2, n) for t, n, _ in sent]
        rc, last = run(sent, on_time)
        check(rc == 0 and "every number arrived on time" in last, f"samples: an on-time step is not a PASS: {rc} {last}")
        rc, last = run(sent, [a for a in on_time if a[1] != 4])
        check(rc == 1 and "1 number(s) never reached the reader" in last, f"samples: a missing number is not a FAIL: {rc} {last}")
        rc, last = run([(t, n, "503" if n == 2 else c) for t, n, c in sent], on_time)
        check(rc == 1 and "did not accept 1 release" in last, f"samples: a refused release is not a FAIL: {rc} {last}")
        late = [(t + (12 if 4 <= n <= 6 else 0.2), n) for t, n, _ in sent]
        rc, last = run(sent, late)
        check(rc == 1 and "late, in a step that has no gap" in last, f"samples: a late number in a step with no gap is not a FAIL: {rc} {last}")
        rc, last = run(sent, late, "--gap")
        check(rc == 0 and "re-delivered after a gap of 4.0 s" in last, f"samples --gap: late but complete is not a PASS naming the gap: {rc} {last}")
        rc, last = run(sent, [a for a in late if a[1] != 5], "--gap")
        check(rc == 1 and "1 number(s) lost outside a rotation's gap" in last, f"samples --gap: a number lost in a run too short for a gap is not a FAIL: {rc} {last}")

        # A transport rotation's gap loses what was released while the tunnel was down
        # (measured, §6): the numbers of each run at least GAP_MIN_S long are recorded
        # with the run's length and count, and pass; a loss outside such a run fails.
        def run_window(sent, arrived, t0, t1, *extra):
            fs, fa = flow(sent, arrived)
            p = subprocess.run([sys.executable, lib, "check", "--sent", fs, "--arrived", fa, "--from", str(t0), "--to", str(t1), *extra],
                               capture_output=True, text=True)
            return p.returncode, (p.stdout.strip().splitlines() or [p.stderr])[-1], p.stdout

        long_sent = [(100 + i, i + 1, "200") for i in range(80)]
        two_gaps = [(t + 0.2, n) for t, n, _ in long_sent if not (11 <= n <= 30 or 51 <= n <= 70)]
        rots = ("--rotation", "105", "135", "--rotation", "145", "175")
        rc, last, out = run_window(long_sent, two_gaps, 100, 180, "--gap", *rots)
        check(rc == 0 and "20 number(s) lost in a gap of 21.0 s" in last and last.count("lost in a gap") == 2,
              f"samples --gap: two rotations' gaps of lost numbers are not a PASS naming each: {rc} {last}")
        rep = json.loads((out.strip().splitlines() or ["{}"])[0])
        check([(g["first"], g["last"], g["lost"]) for g in rep.get("gaps", [])] == [(11, 30, 20), (51, 70, 20)] and rep.get("lost_outside") == [],
              f"samples --gap: the report does not list each gap: {rep}")
        stray = [a for a in two_gaps if a[1] != 76]
        rc, last, out = run_window(long_sent, stray, 100, 180, "--gap", *rots)
        check(rc == 1 and "1 number(s) lost outside a rotation's gap" in last and json.loads((out.strip().splitlines() or ["{}"])[0]).get("lost_outside") == [76],
              f"samples --gap: a loss outside the gaps is not a FAIL naming it: {rc} {last}")
        rc, last = run([(t + 50, n, c) for t, n, c in sent], [])
        check(rc == 1 and "no sample was released inside the window" in last, f"samples: an empty window is not a FAIL: {rc} {last}")

        # A gap counts only when the flow recovers after it, it lies inside a rotation's
        # window (one gap per rotated side) and it is no longer than GAP_MAX_S. 120
        # samples at one a second; one rotation window, 140 to 170.
        many = [(100 + i, i + 1, "200") for i in range(120)]
        def keep(drop):
            return [(t + 0.2, n) for t, n, _ in many if not drop(n)]
        one_rot = ("--rotation", "140", "170")
        rc, last, out = run_window(many, keep(lambda n: 46 <= n <= 65), 100, 220, "--gap", *one_rot)
        check(rc == 0 and "20 number(s) lost in a gap of 21.0 s" in last,
              f"samples --gap: a 20 s gap inside the rotation's window, recovered, is not a PASS: {rc} {last}")
        rc, last, out = run_window(many, keep(lambda n: n >= 51), 100, 220, "--gap", *one_rot)
        check(rc == 1 and "not followed by an on-time number" in last,
              f"samples --gap: a flow that never recovers is not a FAIL: {rc} {last}")
        rc, last, out = run_window(many, keep(lambda n: 11 <= n <= 110), 100, 220, "--gap", "--rotation", "100", "220")
        check(rc == 1 and "longer than" in last,
              f"samples --gap: a 100 s outage is not a FAIL: {rc} {last}")
        rc, last, out = run_window(many, keep(lambda n: 46 <= n <= 51 or 61 <= n <= 66), 100, 220, "--gap", *one_rot)
        check(rc == 1 and "more lossy runs than rotated sides" in last,
              f"samples --gap: two 6 s losses in one rotation (flapping) are not a FAIL: {rc} {last}")
        rc, last, out = run_window(many, [], 100, 220, "--gap", *one_rot)
        check(rc == 1 and "lost outside a rotation's gap" in last,
              f"samples --gap: a flow where nothing arrives is not a FAIL: {rc} {last}")
        rc, last, out = run_window(many, keep(lambda n: 16 <= n <= 35), 100, 220, "--gap", *one_rot)
        check(rc == 1 and "outside every rotation's window" in last,
              f"samples --gap: a gap outside the rotation's window is not a FAIL: {rc} {last}")
    # Each rotation records the rotating side's material before and after, and the
    # partner's. A side's own snapshot keeps its name across both sides' rotations:
    # the second rotation once overwrote the first side's before-file with the key it
    # rotated to, and old-retired-transport then read the new key as the old one.
    snap = ('RECORD=/r; eval "$(sed -n \'/^rot_snap()/,/^}/p\' units/rotation.sh)"; '
            'rot_snap transport xaa xaa before; rot_snap transport xab xaa before; '
            'rot_snap transport xab xab before; rot_snap transport xaa xab after')
    p = subprocess.run(["bash", "-c", snap], cwd=ROOT, capture_output=True, text=True)
    names = p.stdout.split()
    check(p.returncode == 0 and len(names) == 4 and len(set(names)) == 4
          and names[0] == "/r/rot-transport-xaa-before.txt" and names[2] == "/r/rot-transport-xab-before.txt",
          f"rotation: rot_snap does not keep each side's own snapshot apart from the partner's: {names} {p.stderr.strip()}")
    unit = open(os.path.join(ROOT, "units", "rotation.sh"), encoding="utf-8").read()
    body = re.sub(r"(?ms)^rot_snap\(\).*?^}\n", "", unit)
    raw = [l.strip() for l in body.splitlines() if re.search(r"-(before|after)\.txt", l)]
    check(not raw, f"rotation: a snapshot path is spelled outside rot_snap: {raw}")
    # The partner's step after a transport rotation stops at phase 10's hand-off (drive_x
    # 5 14), so it must go on from phase 11, where the partner replaces our peer key;
    # without it the tunnel stays down. It stops at 13: the unit's own acceptance step
    # covers 14, and phase 14 through drive has no acceptance surfaces.
    one = re.search(r"(?ms)^rot_one\(\).*?^}\n", unit)
    body1 = one.group(0) if one else ""
    step = body1.find("drive_x '$them' '$s' '$AVENUE' 5 14")
    back = body1.find('rec "rot-tunnel-back-$k-$s"')
    on = body1.find("drive '$them' '$s' '$AVENUE' 11 13", step if step >= 0 else 0)
    check(step >= 0 and back > step and step < on < back,
          "rotation: the partner's step does not go on from phase 11 (drive $them $s $AVENUE 11 13) before the tunnel check")
    # The partner's step from phase 11 runs through rec.py, which starts its command as
    # a program: a shell function named there is not found and the step never starts.
    # The line is run here through the real rec.py, with `drive` a stub exported the
    # way lib/engine.sh's export_helpers exports it.
    line = next((l.strip() for l in unit.splitlines() if 'rec "rot-partner-step-from-11-' in l), "")
    with tempfile.TemporaryDirectory() as d:
        script = (f'REC={os.path.join(os.path.dirname(ROOT), "e2e", "rec.py")!r}; export E2E_RECORD_DIR={d!r}; FAILS=0\n'
                  'rec() { python3 "$REC" "$@" || FAILS=$((FAILS + 1)); }\n'
                  'drive() { echo "drive $*" > "$E2E_RECORD_DIR/drive.called"; }; export -f drive\n'
                  'k=transport s=xaa them=xab AVENUE=wg\n'
                  f'{line}\n'
                  'echo "FAILS=$FAILS"\n')
        p = subprocess.run(["bash", "-c", script], cwd=ROOT, capture_output=True, text=True)
        called = os.path.join(d, "drive.called")
        got = open(called).read().strip() if os.path.exists(called) else ""
        check(line and "FAILS=0" in p.stdout and got == "drive xab xaa wg 11 13",
              f"rotation: the partner's step from phase 11 does not start through rec.py: {line!r} {got!r} {p.stdout.strip()} {p.stderr.strip()[-300:]}")
    # The samples check is given each rotated side's window: from the side's rotation to
    # the end of its partner's step, recorded around rot_one.
    check(re.search(r"side-start\\t%s\\n' \"\$\(date [^)]*\)\" \"\$s\"", unit) and re.search(r"side-end\\t%s\\n' \"\$\(date [^)]*\)\" \"\$s\"", unit) and "--rotation" in unit,
          "rotation: each side's rotation window is not recorded and passed to samples.py (--rotation)")
    cat = read_yaml(os.path.join(ROOT, "scenarios", "catalog.yaml"))
    ids = {c["id"] for sc in cat["scenarios"] if sc["id"] == "key-rotation" for c in sc.get("checks", [])}
    script = ('eval "$(sed -n \'/^rot_checks()/,/^}/p\' units/rotation.sh)"; '
              'for k in leaves carrier transport signing; do for o in first second; do rot_checks $k $o; done; done')
    p = subprocess.run(["bash", "-c", script], cwd=ROOT, capture_output=True, text=True)
    marked = set(p.stdout.split())
    check(p.returncode == 0 and len(marked) > 40, f"rotation: rot_checks printed {len(marked)} checks, rc {p.returncode}: {p.stderr.strip()}")
    check(marked <= ids, f"rotation: rot_checks names checks the catalog does not hold: {sorted(marked - ids)}")


def burst_checks():
    """lib/burst.py: what the receiving edge did with a burst on one lane (ADR 1244)."""
    lib = os.path.join(ROOT, "lib", "burst.py")
    fx = os.path.join(HERE, "fixtures")
    pre = "from/xaa/all/fe2ebst7/"

    def edge(name):
        p = subprocess.run([sys.executable, lib, "edge", "--log", os.path.join(fx, name), "--key-prefix", pre],
                           capture_output=True, text=True)
        try:
            return p.returncode, json.loads(p.stdout)
        except ValueError:
            return p.returncode, {"stderr": p.stderr.strip()}

    rc, paced = edge("edge-burst-paced.log")
    check(rc == 0 and (paced.get("crossed"), paced.get("dropped")) == (10, 0),
          f"burst edge: the paced log is not 10 crossed, 0 dropped: {rc} {paced}")
    check(paced.get("min_gap_ms") == 11.0, f"burst edge: the paced log's smallest gap is not 11.0 ms: {paced.get('min_gap_ms')}")
    rc, unpaced = edge("edge-burst-unpaced.log")
    check(rc == 0 and (unpaced.get("crossed"), unpaced.get("dropped")) == (2, 8),
          f"burst edge: the unpaced log is not 2 crossed (another run's prefix and the core link left out), 8 dropped: {rc} {unpaced}")
    check(unpaced.get("dropped_keys", [None])[0] == pre + "b2" and unpaced.get("info_drop_lines") == 1,
          f"burst edge: the unpaced log's drops are not named: {unpaced}")
    check((paced.get("out_of_order"), unpaced.get("out_of_order")) == (0, 0),
          f"burst edge: an in-order log counts samples out of order: {paced.get('out_of_order')} {unpaced.get('out_of_order')}")
    rc, reordered = edge("edge-burst-reordered.log")
    check(rc == 0 and (reordered.get("crossed"), reordered.get("out_of_order"), reordered.get("first_out_of_order"))
          == (5, 1, pre + "b3"), f"burst edge: b3 crossing after b4 is not one sample out of order, named: {rc} {reordered}")

    with tempfile.TemporaryDirectory() as d:
        def sent(n, code="200"):
            f = os.path.join(d, "sent")
            write_text(f, "".join(f"{100 + i / 1000}\trelease/all/fe2ebst7/b{i + 1}\t{code}\n" for i in range(n)))
            return f

        def judge(e, *extra, n=10):
            fe = os.path.join(d, "edge.json")
            write_text(fe, json.dumps(e))
            p = subprocess.run([sys.executable, lib, "judge", "--sent", sent(n), "--edge", fe, "--hz", "100", *extra],
                               capture_output=True, text=True)
            out = p.stdout.strip().splitlines()
            return p.returncode, out[-1] if out else p.stderr.strip()

        q0 = ["--queue-full-before", "0", "--queue-full-after", "0"]
        rc, last = judge(paced, "--stored", "10", *q0)
        check(rc == 0 and last.startswith("burst: PASS"), f"burst judge: a paced burst is not a PASS: {rc} {last}")
        rc, last = judge(unpaced, "--stored", "2", *q0)
        check(rc == 1 and "8 dropped at the receiving edge" in last and pre + "b2" in last,
              f"burst judge: an unpaced burst is not a FAIL naming the drops: {rc} {last}")
        rc, last = judge(paced, "--stored", "9", *q0)
        check(rc == 1 and "9 of 10 stored" in last, f"burst judge: a sample not stored is not a FAIL: {rc} {last}")
        rc, last = judge(paced, "--stored", "10", "--queue-full-before", "3", "--queue-full-after", "5")
        check(rc == 1 and "2 queue_full" in last, f"burst judge: a queue_full drop in a paced burst is not a FAIL: {rc} {last}")
        close = dict(paced, min_gap_ms=6.0)
        rc, last = judge(close, "--stored", "10", *q0)
        check(rc == 1 and "6.0 ms" in last, f"burst judge: a gap under 1/hz is not a FAIL naming it: {rc} {last}")
        ov = dict(paced, crossed=4, dropped=0)
        over = ["--stored", "4", "--queue-full-before", "0", "--queue-full-after", "6", "--overflow"]
        rc, last = judge(ov, *over, "--warned", "1")
        check(rc == 0 and last.startswith("burst: PASS") and "6 queue_full" in last,
              f"burst judge --overflow: counted and warned is not a PASS: {rc} {last}")
        rc, last = judge(ov, *over, "--warned", "0")
        check(rc == 1 and "no queue-full warning" in last, f"burst judge --overflow: no warning is not a FAIL: {rc} {last}")
        rc, last = judge(ov, "--stored", "4", "--queue-full-before", "0", "--queue-full-after", "5", "--overflow", "--warned", "1")
        check(rc == 1 and "1 sample(s) unaccounted" in last, f"burst judge --overflow: a silent loss is not a FAIL: {rc} {last}")
        rc, last = judge(reordered, "--stored", "5", *q0, n=5)
        check(rc == 1 and "1 crossed out of put order" in last and pre + "b3" in last,
              f"burst judge: a reordered burst is not a FAIL naming the sample: {rc} {last}")
        rc, last = judge(dict(ov, out_of_order=1, first_out_of_order=pre + "b9"), *over, "--warned", "1")
        check(rc == 1 and "out of put order" in last, f"burst judge --overflow: a reordered overflow is not a FAIL: {rc} {last}")
        rc, last = judge(paced, "--stored", "10", *q0, n=0)
        check(rc == 1 and "nothing was sent" in last, f"burst judge: an empty burst is not a FAIL: {rc} {last}")

    prom = ('# HELP goat_fed_republisher_dropped_total x\n'
            'goat_fed_republisher_dropped_total{prefix="release/all/",reason="queue_full"} 3\n'
            'goat_fed_republisher_dropped_total{prefix="release/xab/",reason="queue_full"} 2\n'
            'goat_fed_republisher_dropped_total{prefix="release/all/",reason="edge_down"} 7\n'
            'goat_fed_republisher_source_info_dropped_total{reason="queue_full"} 9\n')
    p = subprocess.run([sys.executable, lib, "metric"], input=prom, capture_output=True, text=True)
    check(p.returncode == 0 and p.stdout.strip() == "5", f"burst metric: queue_full is not summed to 5: {p.returncode} {p.stdout!r}")
    p = subprocess.run([sys.executable, lib, "metric"], input="", capture_output=True, text=True)
    check(p.returncode == 0 and p.stdout.strip() == "0", f"burst metric: no series is not 0: {p.stdout!r}")
    with tempfile.TemporaryDirectory() as d:
        cfg = os.path.join(d, "config.json5")
        write_text(cfg, 'downsampling: [ { id: "partner-ingress-rate", rules: [\n'
                        '  { key_expr: "from/xab/**", freq: 50 },\n  { key_expr: "from/xaa/**", freq: 100 },\n] } ]\n')
        p = subprocess.run([sys.executable, lib, "hz", "--config", cfg, "--key", "from/xaa/**"], capture_output=True, text=True)
        check(p.returncode == 0 and p.stdout.strip() == "100", f"burst hz: the lane's rule is not read: {p.returncode} {p.stdout!r}")
        p = subprocess.run([sys.executable, lib, "hz", "--config", cfg, "--key", "from/xac/**"], capture_output=True, text=True)
        check(p.returncode == 1, f"burst hz: a lane with no rule is not an error: {p.returncode} {p.stdout!r}")

def drive_rotate_checks():
    """units/rotation.sh copies lib/engine.sh's drive() as drive_rotate with only its
    goatbuilder command line swapped. The copy is refused when the swap finds no
    line, or when it changes any line but that one."""
    engine = os.path.join(ROOT, "lib", "engine.sh")
    real = subprocess.run(["sed", "-n", "/^drive()/,/^}/p", engine], capture_output=True, text=True).stdout
    check("--from \"$from\" --to \"$to\"" in real, "drive_rotate: lib/engine.sh's drive() was not read")
    line = '    goatbuilder) docker exec "${env[@]}" -w /opt/goat "$h" goatbuilder -C /opt/goat --yes federate "$them" --as "$s" --from "$from" --to "$to" ;;'
    twice = real.replace(line, line + "\n" + line.replace("goatbuilder)", "again)"), 1)
    none = real.replace('--from "$from" --to "$to" ;;', '--from "$from" ;;', 1)
    def run(drive):
        script = ('die() { echo "DIE $*"; exit 3; }; '
                  'eval "$(sed -n \'/^make_drive_rotate()/,/^}/p\' units/rotation.sh)"; '
                  'eval "$DRIVE"; make_drive_rotate; declare -f drive_rotate | grep -c "federate rotate"')
        p = subprocess.run(["bash", "-c", script], cwd=ROOT, capture_output=True, text=True,
                           env={**os.environ, "DRIVE": drive})
        return p.returncode, (p.stdout + p.stderr).strip()
    rc, out = run(real)
    check(rc == 0 and out.endswith("1"), f"drive_rotate: the copy of today's drive() is refused: {rc} {out}")
    rc, out = run(twice)
    check(rc == 3 and "DIE" in out and "diff lines" in out, f"drive_rotate: a copy that changes a second line is not refused: {rc} {out}")
    rc, out = run(none)
    check(rc == 3 and "no longer has the command line" in out, f"drive_rotate: a drive() with no line to swap is not refused: {rc} {out}")


def gbrelease_checks():
    """--goatbuilder-version: the released asset, checked against its .sha256, on the release's own ref."""
    sha = "a" * 40
    with tempfile.TemporaryDirectory() as d:
        home = os.path.join(d, "fe2e")
        asset = os.path.join(d, "served-goatbuilder")
        write_text(asset, "the published goatbuilder\n")
        good = subprocess.run(["sha256sum", asset], capture_output=True, text=True).stdout.split()[0]
        called = os.path.join(d, "release-tool-called")
        # release_tool and release_tag are env's own, cut out of it; curl serves the asset and a sum.
        prelude = ('set -u; FE2E_HOME="$1"; lock() { :; }; unlock() { :; }; '
                   'eval "$(sed -n \'/^release_tag()/p; /^release_tool()/,/^}/p\' env)"; '
                   'curl() { local o=""; while [ $# -gt 1 ]; do [ "$1" = -o ] && o="$2"; shift; done; '
                   'if [ -n "$o" ]; then cp "$ASSET" "$o"; else echo "$SUM  goatbuilder-linux-amd64"; fi; }; '
                   'crate_version() { echo "$CRATE_V"; }; '
                   '. lib/gbrelease.sh; ')

        def run(body, gb_version, crate_v="0.7.1", served_sum=good, spy=False):
            e = {**os.environ, "ASSET": asset, "SUM": served_sum, "CRATE_V": crate_v, "GB_VERSION": gb_version}
            pre = prelude
            if spy:  # records that the release was fetched at all
                pre += f'eval "real_$(declare -f release_tool)"; release_tool() {{ : > {called}; real_release_tool "$@"; }}; '
            p = subprocess.run(["bash", "-c", pre + body, "_", home], cwd=ROOT, capture_output=True, text=True, env=e)
            return p.returncode, p.stdout.strip(), p.stderr.strip()

        rc, out, err = run(f'gb_under_test {sha}; gb_tools "$(gb_under_test {sha})" | wc -l', "")
        check(rc == 0 and out.splitlines() == [f"{home}/bin/{sha}/goatbuilder", "0"],
              f"gbrelease: without --goatbuilder-version a unit does not run the ref's own build, or tools.txt changes: {rc} {out} {err}")

        rc, out, err = run(f'p="$(gb_under_test {sha})" && echo "$p" && gb_tools "$p"', "0.7.1")
        rel = os.path.join(home, "release", "goatbuilder-0.7.1", "goatbuilder")
        check(rc == 0 and out.splitlines() == [rel, "GOATBUILDER_RELEASE=0.7.1", f"GOATBUILDER_SHA256={good}"],
              f"gbrelease: the release asset is not the goatbuilder under test, or tools.txt does not name it: {rc} {out} {err}")

        rc, out, err = run(f'gb_under_test {sha}', "0.7.2", crate_v="0.7.2", served_sum="0" * 64)
        check(rc != 0 and "not published as a release asset" in err and not os.path.exists(os.path.join(home, "release", "goatbuilder-0.7.2", "goatbuilder")),
              f"gbrelease: an asset that does not match its .sha256 was used: {rc} {out} {err}")

        rc, out, err = run(f'gb_under_test {sha}', "0.7.1", crate_v="0.7.0", spy=True)
        check(rc != 0 and "is goatbuilder 0.7.0, not 0.7.1" in err and not os.path.exists(called),
              f"gbrelease: a ref that is not the release was run with the released goatbuilder: {rc} {out} {err}")

    def fp(gb_version):
        p = subprocess.run(["bash", "-c", 'eval "$(sed -n \'/^fingerprint()/,/^}/p\' env)"; fingerprint . all'],
                           cwd=ROOT, capture_output=True, text=True, env={**os.environ, "GB_VERSION": gb_version})
        return p.stdout.strip()
    built, r1, r2 = fp(""), fp("0.7.1"), fp("0.7.2")
    check(len(built) == 12 and len({built, r1, r2}) == 3,
          f"gbrelease: the fingerprint does not tell a released goatbuilder from a build, or one release from another: {built} {r1} {r2}")


def joinsite_checks():
    def run(services, checks):
        with tempfile.TemporaryDirectory() as t:
            m, pl = os.path.join(t, "model.json"), os.path.join(t, "plan.json")
            write_text(m, json.dumps({"services": services}))
            write_text(pl, json.dumps({"checks": checks}))
            p = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "joinsite.py"), m, pl, "n1"],
                               capture_output=True, text=True)
            return p.returncode, p.stdout
    on = lambda: {"present": True, "hosts": ["n1"]}
    ory = {"name": "cmd", "detail": "cd x && ansible-playbook ory-deploy.yml --limit n1 -e a=b"}
    named = {"name": "not-run", "detail": "portal \u2014 declared, not yet built at this site (gate row I1, I4)"}
    rc, out = run({"ory": on(), "portal": on(), "minio": {"present": False, "hosts": ["n1"]}}, [ory, named])
    check(rc == 0 and "built   ory  by ory-deploy.yml" in out and "named   portal" in out and "minio" not in out,
          f"joinsite: a built and a named service do not pass: {rc} {out}")
    rc, out = run({"ory": on()}, [{"name": "cmd", "detail": "ansible-playbook ory-deploy.yml --limit n2"}])
    check(rc == 1 and "SILENT  ory" in out, f"joinsite: a service built on another node passes: {rc} {out}")
    rc, out = run({"portal": on(), "grafana": on()}, [named])
    check(rc == 1 and "SILENT  grafana" in out, f"joinsite: a silent service passes: {rc} {out}")
    rc, out = run({"ory": on()}, [ory, {"name": "not-run", "detail": "ory \u2014 declared at n1 (gate row I3)"}])
    check(rc == 1 and "BOTH    ory" in out, f"joinsite: a service built and named passes: {rc} {out}")
    rc, out = run({"ory": {"present": True, "hosts": ["n2"]}}, [ory])
    check(rc == 1, f"joinsite: a model that places nothing on the node passes: {rc} {out}")


def joinsite_lab_checks():
    lab = os.path.join(ROOT, "lib", "joinsite_lab.py")
    fixture = os.path.join(ROOT, "..", "goatbuilder", "fixtures", "join-node-third-site")
    with tempfile.TemporaryDirectory() as t:
        dest = os.path.join(t, "tree")
        p = subprocess.run([sys.executable, lab, "layout", fixture, dest, "10.77.6.10", "10.77.6.20"],
                           capture_output=True, text=True)
        check(p.returncode == 0, f"joinsite_lab: layout failed: {p.stderr}")
        with open(os.path.join(dest, "ops/ansible/inventory/mgmt-hosts.ini"), encoding="utf-8") as fh:
            inv = fh.read()
        check("mgmt-alpha-01 ansible_host=10.77.6.10" in inv and "gamma-core-1 ansible_host=10.77.6.20" in inv
              and "beta" not in inv and "delta" not in inv, f"joinsite_lab: the lab inventory is not two sites: {inv}")
        gv = os.path.join(dest, "ops/ansible/group_vars")
        check(sorted(os.listdir(gv)) == ["alpha_hosts", "gamma_hosts"], f"joinsite_lab: group_vars left: {os.listdir(gv)}")
        with open(os.path.join(gv, "alpha_hosts", "vars.yml"), encoding="utf-8") as fh:
            alpha = fh.read()
        check("signal_federation_proxy_enabled" not in alpha and "federation_peer" not in alpha
              and "signal_federation_absent_reason:" in alpha and "goatnet_founder: true" in alpha,
              f"joinsite_lab: alpha's signal federation is not declared off: {alpha}")
        with open(os.path.join(dest, "ops/ansible/inventory/mgmt-hosts-mesh.ini"), encoding="utf-8") as fh:
            mesh = fh.read()
        check("mgmt-alpha-01 " in mesh and "alpha-core-1" not in mesh and "beta" not in mesh,
              f"joinsite_lab: the mesh inventory is not the lab's: {mesh}")

        def reach(cmds, timeout=None):
            plan = os.path.join(t, "plan.json")
            write_text(plan, json.dumps({"checks": [{"name": "cmd", "detail": c} for c in cmds]}))
            r = subprocess.run([sys.executable, lab, "reach", plan, "mgmt-alpha-01=10.77.6.10", "gamma-core-1=10.77.6.20",
                                "alpha_hosts", "gamma_hosts"], capture_output=True, text=True, timeout=timeout)
            return r.returncode, r.stdout
        good = ["ansible-playbook x.yml -l 'mgmt-alpha-01*'", "ssh -o BatchMode=yes goat@10.77.6.20 true",
                "ansible-playbook y.yml --limit gamma-core-1", "ansible-playbook z.yml -e target=alpha_hosts"]
        rc, out = reach(good)
        check(rc == 0 and " 0 reach outside the lab" in out, f"joinsite_lab: a lab plan does not pass: {rc} {out}")
        rc, out = reach(good + ["ssh goat@192.0.2.30 true"])
        check(rc == 1 and "the fixture's address 192.0.2.30" in out, f"joinsite_lab: the fixture's address passes: {rc} {out}")
        rc, out = reach(good + ["ansible-playbook x.yml -l 'mgmt-beta-01*'"])
        check(rc == 1 and "outside the lab, mgmt-beta-01*" in out, f"joinsite_lab: a foreign limit passes: {rc} {out}")
        rc, out = reach([])
        check(rc == 1, f"joinsite_lab: an empty plan passes: {rc} {out}")
        rc, out = reach(good + ["ssh -o BatchMode=yes -p 22 goat@203.0.113.5 true"])
        check(rc == 1 and "outside the lab, 203.0.113.5" in out, f"joinsite_lab: a foreign ssh target passes: {rc} {out}")
        rc, out = reach(good + ["bash -c 'ssh -o BatchMode=yes goat@10.77.6.10 true'"])
        check(rc == 0, f"joinsite_lab: an ssh to the lab inside a quoted command fails: {rc} {out}")
        # A long run of options with no target is read in linear time, not by backtracking.
        try:
            rc, out = reach(good + ["ssh" + " -!" * 40 + " x"], timeout=20)
            check(rc == 0, f"joinsite_lab: an ssh with no user@host target fails: {rc} {out}")
        except subprocess.TimeoutExpired:
            check(False, "joinsite_lab: reach did not finish within 20 s on a long ssh option run")


def refused_marks_checks():
    """A refused rotation marks BLOCKED only what the refusal stops: the refused side's own
    checks and the key's checks, never the other side's, which runs its own rotate and
    records its own result. Marks outrank passing steps, so marking the other side would
    hide its real results."""
    script = ('eval "$(sed -n \'/^rot_checks()/,/^}/p\' units/rotation.sh)"; '
              'eval "$(sed -n \'/^rot_refused_marks()/,/^}/p\' units/rotation.sh)"; '
              'rot_refused_marks "$K" "$O"')
    def run(k, o):
        p = subprocess.run(["bash", "-c", script], cwd=ROOT, capture_output=True, text=True,
                           env={**os.environ, "K": k, "O": o})
        rows = [l.split("\t", 1) for l in p.stdout.splitlines() if l.strip()]
        return p.returncode, {c: why for c, why in rows}
    rc, m = run("carrier", "second")
    check(rc == 0 and m, f"refused marks: nothing printed for a refused second side: rc {rc}")
    check(not any(c.endswith("-first") for c in m), f"refused marks: a second side's refusal marks the first side's checks: {sorted(c for c in m if c.endswith('-first'))}")
    check("rotate-carrier-second-ran" in m and "samples-carrier" in m and "old-retired-carrier" in m,
          f"refused marks: a second side's refusal leaves its own or the key's checks unmarked: {sorted(m)}")
    rc, m = run("transport", "first")
    check("rotate-transport-first-ran" in m and "samples-transport" in m,
          f"refused marks: a first side's refusal leaves its own or the key's checks unmarked: {sorted(m)}")
    check(not any(c.endswith("-second") or c.endswith("-second-ran") for c in m),
          f"refused marks: a first side's refusal marks the second side, which still runs its own rotate: {sorted(c for c in m if 'second' in c)}")
    # The second side still attempts its own rotate after the first side's refusal (a refusal
    # changes nothing), so one run shows each side's own refusal.
    src = open(os.path.join(ROOT, "units", "rotation.sh"), encoding="utf-8").read()
    body = src.split("rot_one() {", 1)[1].split("\n}\n", 1)[0] if "rot_one() {" in src else ""
    check(body and 'case " $ROT_REFUSED " in *" $k "*) return 0' not in body.split("drive_rotate", 1)[0],
          "refused marks: rot_one still skips the second side's rotate once the first side was refused")


def refused_changed_nothing_checks():
    """A refused rotation's proof that it changed nothing counts in the verdict: the catalog
    has refused-changed-nothing-<key>-<side> for every key, on the unit's own step, and a
    rotation that runs marks it NOT APPLICABLE, so a key that rotates is never BLOCKED by it."""
    cat = read_yaml(os.path.join(ROOT, "scenarios", "catalog.yaml"))
    sc = [s for s in cat["scenarios"] if s["id"] == "key-rotation"]
    checks = {c["id"]: c.get("step") for c in (sc[0]["checks"] if sc else [])}
    for k in ("leaves", "carrier", "transport", "signing"):
        for o, side in (("first", "{A}"), ("second", "{B}")):
            cid = f"refused-changed-nothing-{k}-{o}"
            check(checks.get(cid) == f"rot-refused-{k}-{side}-changed-nothing",
                  f"refused changed nothing: key-rotation has no check {cid} on step rot-refused-{k}-{side}-changed-nothing (got {checks.get(cid)!r})")
    src = open(os.path.join(ROOT, "units", "rotation.sh"), encoding="utf-8").read()
    body = src.split("rot_one() {", 1)[1].split("\n}\n", 1)[0] if "rot_one() {" in src else ""
    after = body.split("local f0=", 1)[-1] if "local f0=" in body else ""
    ran = body.split("local f0=", 1)[0].rsplit("return 0\n  fi", 1)[-1] + after.split('rec "rot-rotate-$k-$s-ran"', 1)[0]
    check('mark "key-rotation.refused-changed-nothing-$k-$ord" "NOT APPLICABLE"' in ran,
          "refused changed nothing: a rotation that runs does not mark its refused-changed-nothing check NOT APPLICABLE before it records rotate-<key>-ran")


def rotate_refusal_checks():
    """units/rotation.sh reads a key the goatbuilder under test refuses to rotate
    from the refusal's code, and marks that key's checks BLOCKED instead of
    failing the unit. Every code `rotate` refuses with before anything runs
    (each goat_errors::coded in crates/goatbuilder/src/federate/rotation.rs) must
    be one it reads: a hand-declared forwarder's GB-312, for one."""
    unit = open(os.path.join(ROOT, "units", "rotation.sh")).read()
    m = re.search(r"why=\"\$\(grep -m1 -oE '([^']+)' \"\$out\"", unit)
    check(m is not None, "rotate refusals: units/rotation.sh has no refusal grep")
    if m is None:
        return
    pattern = m.group(1)
    src = open(os.path.join(ROOT, "..", "..", "crates", "goatbuilder", "src", "federate", "rotation.rs")).read()
    # A run's own fault is not a refusal of the key, and the unit fails on it:
    # a file it could not write (GB-254), a rotation id file that is not one
    # (GB-321), a state the code rules out (GB-322).
    faults = {"GB-254", "GB-321", "GB-322"}
    codes = sorted(set(re.findall(r'goat_errors::coded\(\s*"(GB-\d+)"', src)) - faults)
    check(len(codes) >= 3, f"rotate refusals: rotation.rs names too few codes: {codes}")
    for code in codes:
        sample = f"`rotate xab carrier`: peering xab on zagros declares something. Nothing ran ({code}). Do this instead\n"
        p = subprocess.run(["grep", "-m1", "-oE", pattern], input=sample, capture_output=True, text=True)
        check(p.returncode == 0 and code in p.stdout, f"rotate refusals: units/rotation.sh does not read {code} as a refusal (pattern {pattern})")


def live_images_checks():
    """live-host.sh images against a docker that holds only the images named in HELD."""
    zen = "ghcr.io/usareur-af/zenoh-pqc@sha256:" + "1" * 64
    spire = "ghcr.io/spiffe/spire-server:1.0@sha256:" + "2" * 64
    rp = "ghcr.io/example-org/goat-fed-republisher@sha256:" + "3" * 64
    with tempfile.TemporaryDirectory() as d:
        log = os.path.join(d, "docker.log")
        os.makedirs(os.path.join(d, "bin"))
        write_text(os.path.join(d, "bin", "docker"),
                   "#!/bin/sh\necho \"$*\" >> %s\n"
                   "case \"$1 $2\" in \"image inspect\") case \" $HELD \" in *\" $3 \"*) exit 0 ;; esac; exit 1 ;; esac\n"
                   "[ \"$1\" = pull ] && [ \"$2\" = -q ] && exit 0\nexit 2\n" % log)
        os.chmod(os.path.join(d, "bin", "docker"), 0o700)
        env = dict(os.environ, PATH=os.path.join(d, "bin") + ":" + os.environ["PATH"], HELD=zen,
                   FED_ZENOH_IMAGE=zen, FED_SPIRE_IMAGE=spire, FED_REPUBLISHER_IMAGE=rp)
        r = subprocess.run(["bash", os.path.join(ROOT, "live-host.sh"), "images"], env=env, capture_output=True, text=True)
        calls = ""
        if os.path.exists(log):
            with open(log, encoding="utf-8") as fh:
                calls = fh.read()
        check(r.returncode == 0 and "present: " + zen in r.stdout and "images ok" in r.stdout, f"live images: a held image is not used as it is: {r.returncode} {r.stdout}{r.stderr}")
        check("pull -q " + zen not in calls, f"live images: a held image was pulled: {calls}")
        check("pull -q " + spire in calls and "pull -q " + rp in calls and "pulled: " + spire in r.stdout, f"live images: an image not held was not pulled: {calls}")
        env["FED_ZENOH_IMAGE"] = "ghcr.io/usareur-af/zenoh-pqc:latest"
        r = subprocess.run(["bash", os.path.join(ROOT, "live-host.sh"), "images"], env=env, capture_output=True, text=True)
        check(r.returncode != 0 and "digest-pinned" in r.stderr, f"live images: an image by tag is not refused: {r.returncode} {r.stderr}")
    with tempfile.TemporaryDirectory() as d:
        for held in (True, False):
            b = os.path.join(d, str(held))
            os.makedirs(b)
            write_text(os.path.join(b, "docker"), "#!/bin/sh\n[ \"$1 $2\" = \"compose version\" ] && exit %d\nexit 0\n" % (0 if held else 1))
            os.chmod(os.path.join(b, "docker"), 0o700)
            r = subprocess.run(["bash", os.path.join(ROOT, "live-host.sh"), "packages"], env=dict(os.environ, PATH=b + ":/usr/bin:/bin"), capture_output=True, text=True)
            words = set(r.stdout.split())
            ubuntu_docker = {"docker.io", "docker-compose-v2"}
            check(r.returncode == 0 and "wireguard-tools" in words and (ubuntu_docker.isdisjoint(words) if held else ubuntu_docker <= words),
                  f"live packages: docker {'held' if held else 'absent'}: {r.returncode} {r.stdout}{r.stderr}")
    base = ["bash", os.path.join(ROOT, "live.sh"), "--dry-run", "--scenario", "1", "--pasture-host", "192.0.2.11", "--meadow-host", "192.0.2.12"]
    env = {k: v for k, v in os.environ.items() if k != "GHCR_READ_TOKEN"}
    with tempfile.TemporaryDirectory() as d:
        r = subprocess.run(base + ["--images-present", "--record", d], env=env, capture_output=True, text=True)
        out = r.stdout + r.stderr
        check(r.returncode == 0 and "docker login" not in out and "logout" not in out, f"live --images-present: a dry run logs a host in or fails: {r.returncode} {out[-600:]}")
        r = subprocess.run(base + ["--images-present", "--record", d], env=dict(env, FED_LIVE_PORT_BASE_PASTURE="38", FED_LIVE_PORT_BASE_MEADOW="48"), capture_output=True, text=True)
        check(r.returncode == 0 and "-L 38000,38101-38103 to pasture; 48000,48101-48103 to meadow" in r.stderr, f"live: the forwards' ports do not move with FED_LIVE_PORT_BASE_*: {r.returncode} {r.stderr[-600:]}")
    gb = os.path.join(tempfile.gettempdir(), "no-such-binary")
    r = subprocess.run(["bash", os.path.join(ROOT, "live.sh"), "--goat", gb, "--goatbuilder", gb, "--pasture-host", "192.0.2.11", "--meadow-host", "192.0.2.12"], env=env, capture_output=True, text=True)
    check(r.returncode != 0 and "GHCR_READ_TOKEN" in r.stderr, f"live: without --images-present a missing token is not refused: {r.returncode} {r.stderr}")


def live_clone_gc_checks():
    """live-host.sh clone turns auto gc off in the repository it removes, before the commit that would start one."""
    src = open(os.path.join(ROOT, "live-host.sh"), encoding="utf-8").read()
    body = src.split("\nclone)\n", 1)[1].split("\n  ;;\n", 1)[0] if "\nclone)\n" in src else ""
    lines = [l.strip() for l in body.splitlines()]
    def at(text):
        return next((i for i, l in enumerate(lines) if text in l), -1)
    init, off, commit, rm = at('git -C "$D/src" init'), at('git -C "$D/src" config gc.auto 0'), at('git -C "$D/src" commit'), max(i for i, l in enumerate(lines) if l == 'rm -rf "$D/src"') if 'rm -rf "$D/src"' in lines else -1
    check(-1 < init < off < commit < rm, f"live clone: auto gc is not off between init and commit in the repository clone removes (init {init}, gc.auto 0 {off}, commit {commit}, rm {rm})")


catalog_checks()
backout_edge_check()
srctree_checks()
srcclone_checks()
verdict_checks()
fwdeclared_checks()
hostfw_checks()
launcher_key_mode_checks()
handoff_checks()
samples_checks()
burst_checks()
drive_rotate_checks()
gbrelease_checks()
joinsite_checks()
joinsite_lab_checks()
refused_marks_checks()
refused_changed_nothing_checks()
rotate_refusal_checks()
live_images_checks()
live_clone_gc_checks()
for f in FAIL:
    print(f"FAIL {f}")
print("ok: federation e2e harness checks" if not FAIL else f"{len(FAIL)} harness check(s) failed")
sys.exit(1 if FAIL else 0)
