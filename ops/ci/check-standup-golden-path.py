#!/usr/bin/env python3
"""check-standup-golden-path.py — the golden path is internally coherent.

A per-step gut check for the goatnet standup golden path: the driver
(ops/scripts/goat-standup.sh), the handbook (docs/operations/goatnet-standup.md),
and the three engines they chain (config-preflight = READY, the driver = ORDER,
run-acceptance = DONE). It catches the exact defect classes that a 5,000-line
rewrite introduced and that the two end gates cannot see:

  correctness  — every playbook/script the driver names exists; `hosts: all`
                 playbooks are scoped with `--limit`, never a bare `-e target=`
                 that does not narrow them (the F-376 class); site-bootstrap
                 carries its required extra-vars; `--plan` renders for both
                 profiles; no reference to a retired environment.
  completeness — every phase the driver runs is documented in the handbook and
                 vice versa; every handbook link and internal anchor resolves;
                 the three engines exist.

It is offline (only `--plan` is executed, which runs nothing) and finishes in
seconds, so it runs on every PR with no path filter — the condition for it to
become a required check.

Doctrine P6: `--self-test` plants violations in synthetic inputs and asserts the
checker flags them, so a green verdict on the real tree is trusted.

Usage:  python3 ops/ci/check-standup-golden-path.py [--self-test]
Exit:   0 = all checks pass · 1 = one or more failed · 2 = self-test failed
"""
import os
import re
import subprocess
import sys

REPO = subprocess.run(["git", "rev-parse", "--show-toplevel"],
                      capture_output=True, text=True).stdout.strip()
DRIVER = "ops/scripts/goat-standup.sh"
HANDBOOK = "docs/operations/goatnet-standup.md"
ENGINES = [
    "ops/scripts/standup-config-preflight.py",   # READY
    "ops/scripts/goat-standup.sh",               # ORDER
    "ops/validation/run-acceptance.py",          # DONE
]
# Environments removed from the tree; a live doc must not send a reader to them.
RETIRED_ENVS = ["prov_conus_hosts", "vega_hosts"]
# Plays the driver names that a public tree does not ship: what a reader meets
# when the driver reaches one, and the driver text that makes that true. A new
# absence fails the check until it is listed here. An entry stops excusing its
# play if the driver text goes away, and in a tree that declares a deployment
# no entry applies, because a real goatnet needs every play the driver names.
PLAYS_OUTSIDE_THIS_TREE = {
    "utility-host-bootstrap.yml": (
        "the driver refuses the obs leg and names --skip-obs",
        '[ ! -f "$REPO/ops/ansible/utility-host-bootstrap.yml" ]'),
    "zagros-zenoh-synth-vendors-deploy.yml": (
        "opt-in; the step fails when synth_vendors_enabled is true",
        "group_vars/${GROUP}/synth-vendors.yml 2>/dev/null; then"),
    "herd-connector-deploy.yml": (
        "ships in goatstuff; phase 14 fails when has_network_jump_node is true",
        "rescue_phase has_network_jump_node"),
}
DEPLOYMENT_MARKER = "ops/goatbuilder/goatbuilder.yaml"
# Fewer plays than this means the parser went blind, as it did when the driver
# moved from a literal ansible-preflight.sh to $ANSIBLE_PRE and every play check
# passed on an empty list.
MIN_DRIVER_PLAYS = 15

# ------- pure helpers (no filesystem), so --self-test can feed synthetic text --

PLAY_RUNNER = re.compile(r'ansible-preflight\.sh|\$ANSIBLE_PRE|\bansible-playbook\b')
PLAY_NAME = re.compile(r'(?<![/\w.-])([A-Za-z0-9_${}][A-Za-z0-9._${}-]*\.yml)')
PLAY_VARIABLE = re.compile(r'\b([A-Z][A-Z0-9_]*)="?([A-Za-z0-9_.${}-]+\.yml)"?')
VARIABLE_USE = re.compile(r'\$\{?([A-Z][A-Z0-9_]*)\}?')

def driver_playbooks(driver_text):
    """(playbook, invocation-line) for every ansible playbook the driver runs."""
    variables = {}
    for name, value in PLAY_VARIABLE.findall(driver_text):
        variables.setdefault(name, set()).add(value)
    out = []
    for line in driver_text.splitlines():
        if line.lstrip().startswith("#"):
            continue
        for seg in PLAY_RUNNER.split(line)[1:]:
            # Only this invocation: later text is another command or a message.
            cmd = re.split(r'\|\||&&|;', seg)[0]
            out.extend((m.group(1), line) for m in PLAY_NAME.finditer(cmd))
            # A play chosen earlier and run through a variable, as $RATHOLE_PLAY.
            for use in VARIABLE_USE.findall(cmd):
                out.extend((play, line) for play in sorted(variables.get(use, ())))
        # rescue_phase takes its play as a quoted argument on its own line.
        arg = re.match(r'\s*"([a-z0-9-]+\.yml)\b', line)
        if arg:
            out.append((arg.group(1), line))
    return out

def missing_plays(driver_text, exists, declared=PLAYS_OUTSIDE_THIS_TREE):
    """Plays the driver names that neither exist nor are declared absent."""
    names = {pb for pb, _ in driver_playbooks(driver_text) if "$" not in pb}
    return sorted(n for n in names if n not in declared and not exists(n))

def unhandled_absences(driver_text, exists, declared=PLAYS_OUTSIDE_THIS_TREE):
    """Declared-absent plays whose handling is no longer in the driver."""
    return sorted(n for n, (_, handling) in declared.items()
                  if not exists(n) and handling not in driver_text)

def scoping_violations(driver_text, hosts_all_playbooks):
    """A `hosts: all` playbook invoked with a bare `-e target=` and no `--limit`
    is not narrowed — it runs against every mgmt host (the F-376 class)."""
    bad = []
    for pb, line in driver_playbooks(driver_text):
        base = re.sub(r'^\$\{SITE[^}]*\}-?', "", pb)  # strip $SITE- prefix for match
        for hp in hosts_all_playbooks:
            if pb == hp or base == hp or pb.endswith(hp):
                if "--limit" not in line:
                    bad.append((pb, line.strip()))
    return bad

def gh_slug(heading):
    """GitHub's heading-anchor algorithm: lowercase; drop everything but word
    chars, spaces and hyphens; spaces -> hyphens. Crucially it does NOT collapse
    runs, so `A — B` (space, em-dash, space) becomes `a--b`."""
    s = heading.strip().lower()
    s = re.sub(r'[^\w\s-]', "", s)   # \w keeps a-z 0-9 _ ; drops em-dash, comma, etc.
    return s.replace(" ", "-")

def anchor_violations(handbook_text):
    headings = {gh_slug(m.group(1))
                for m in re.finditer(r'^#{2,6}\s+(.*)$', handbook_text, re.M)}
    return [m.group(1) for m in re.finditer(r'\]\(#([a-z0-9-]+)\)', handbook_text)
            if m.group(1) not in headings]

def expand_vars(line, driver_text):
    """Inline any $VAR / ${VAR} on a line with that var's definition elsewhere in
    the driver, so a check sees through `site-bootstrap.yml $BOOTSTRAP_EV`."""
    seen = line
    for m in re.finditer(r'\$\{?([A-Za-z_][A-Za-z0-9_]*)\}?', line):
        var = m.group(1)
        d = re.search(rf'^{var}=(.*)$', driver_text, re.M)
        if d:
            seen += " " + d.group(1)
    return seen

def retired_env_hits(text):
    return [e for e in RETIRED_ENVS if e in text]

# ------- filesystem-backed checks --------------------------------------------

def hosts_all_playbooks():
    """Playbooks under ops/ansible whose `hosts:` is the literal `all`."""
    d = os.path.join(REPO, "ops/ansible")
    found = []
    for f in os.listdir(d):
        if not f.endswith(".yml"):
            continue
        try:
            with open(os.path.join(d, f), encoding="utf-8") as source_file:
                txt = source_file.read()
        except OSError:
            continue
        if re.search(r'^\s*hosts:\s*all\s*$', txt, re.M):
            found.append(f)
    return set(found)

def link_targets(handbook_text):
    for m in re.finditer(r'\]\(([^)#]+)(?:#[^)]*)?\)', handbook_text):
        t = m.group(1).strip()
        if t.startswith(("http", "mailto:", "#")) or "<" in t or "%" in t:
            continue
        yield t

# ------- the checks -----------------------------------------------------------

def run_checks(driver_text, handbook_text, do_fs=True):
    """Returns list of (ok, step, detail). Pure checks always run; filesystem +
    subprocess checks run only when do_fs (skipped in --self-test)."""
    r = []

    # completeness: three engines present
    if do_fs:
        for e in ENGINES:
            r.append((os.path.exists(os.path.join(REPO, e)),
                      f"engine present: {e}", ""))

    # correctness: every playbook the driver names exists
    if do_fs:
        plays = {pb for pb, _ in driver_playbooks(driver_text) if "$" not in pb}
        r.append((len(plays) >= MIN_DRIVER_PLAYS,
                  f"driver plays are visible to this check ({len(plays)} found)",
                  "" if len(plays) >= MIN_DRIVER_PLAYS else f"want at least {MIN_DRIVER_PLAYS}"))
        exists = lambda n: os.path.exists(os.path.join(REPO, "ops/ansible", n))
        declared = ({} if os.path.exists(os.path.join(REPO, DEPLOYMENT_MARKER))
                    else PLAYS_OUTSIDE_THIS_TREE)
        missing = set(missing_plays(driver_text, exists, declared))
        for pb in sorted(plays):
            r.append((pb not in missing, f"driver playbook exists: {pb}",
                      "" if pb not in missing else "not in ops/ansible/ and not in PLAYS_OUTSIDE_THIS_TREE"))
        unhandled = unhandled_absences(driver_text, exists, declared)
        r.append((not unhandled, "every declared-absent play is still handled by the driver",
                  ", ".join(unhandled)))

    # correctness: scripts the driver names exist
    if do_fs:
        for m in re.finditer(r'(ops/(?:scripts/)?[A-Za-z0-9._-]+\.(?:sh|py))', driver_text):
            s = m.group(1)
            ok = os.path.exists(os.path.join(REPO, s))
            r.append((ok, f"driver script exists: {s}", "" if ok else "missing"))

    # correctness: F-376 scoping — hosts:all playbooks need --limit
    hap = hosts_all_playbooks() if do_fs else {"ory-deploy.yml",
                                               "goat-netbird-account-init.yml",
                                               "goat-coredns.yml"}
    bad = scoping_violations(driver_text, hap)
    r.append((not bad, "scoping: hosts:all playbooks use --limit (not bare -e target=)",
              "" if not bad else "; ".join(f"{pb}" for pb, _ in bad)))

    # correctness: site-bootstrap carries its required extra-vars
    need = ["bootstrap_owner_email", "postgres_superuser_password",
            "netbird_encryption_key", "pgbackrest_cipher_pass"]
    sb = [expand_vars(l, driver_text) for l in driver_text.splitlines()
          if "site-bootstrap.yml" in l]
    if sb:
        missing = [v for v in need if not any(v in l for l in sb)]
        r.append((not missing, "site-bootstrap invocations carry required extra-vars",
                  "" if not missing else "missing: " + ", ".join(missing)))

    # completeness: handbook internal anchors resolve
    a = anchor_violations(handbook_text)
    r.append((not a, "handbook internal anchors resolve",
              "" if not a else "dangling: " + ", ".join(a)))

    # completeness: handbook relative links resolve
    if do_fs:
        broken = []
        base = os.path.join(REPO, os.path.dirname(HANDBOOK))
        for t in link_targets(handbook_text):
            if not os.path.exists(os.path.normpath(os.path.join(base, t))):
                broken.append(t)
        r.append((not broken, "handbook relative links resolve",
                  "" if not broken else "broken: " + ", ".join(broken)))

    # correctness: no reference to a retired environment
    hits = retired_env_hits(driver_text) + retired_env_hits(handbook_text)
    r.append((not hits, "no reference to a retired environment",
              "" if not hits else "found: " + ", ".join(sorted(set(hits)))))

    # completeness: every driver phase number is documented in the handbook
    if do_fs:
        phases = sorted({m.group(1) for m in re.finditer(r'^step\s+(\d+)\s', driver_text, re.M)},
                        key=int)
        undoc = [p for p in phases
                 if not re.search(rf'(?:phase|Phase)\s+{p}\b|^\|\s*{p}\s*\|', handbook_text, re.M)]
        r.append((not undoc, "every driver phase is documented in the handbook",
                  "" if not undoc else "undocumented phases: " + ", ".join(undoc)))

    # correctness: --plan renders for both profiles (runs nothing)
    if do_fs:
        for prof in ("lan", "internet"):
            p = subprocess.run(["bash", os.path.join(REPO, DRIVER),
                                "goldenpathcheck", "--plan", "--profile", prof],
                               capture_output=True, text=True)
            ok = p.returncode == 0 and "=== Phase 13 [GATE] acceptance" in p.stdout
            r.append((ok, f"--plan renders (profile={prof})",
                      "" if ok else f"rc={p.returncode}"))
            # The rendered plan is the ground truth for the source parser above:
            # a play the plan runs but the parser cannot see means it went blind.
            seen = {pb for pb, _ in driver_playbooks(driver_text)}
            unseen = sorted({pb for pb, _ in driver_playbooks(p.stdout)} - seen)
            r.append((not unseen, f"parser sees every play the plan runs (profile={prof})",
                      ", ".join(unseen)))

    # correctness: every goatnet in the door registry has its inventory blocks
    # established (env-door.sh arm + doors.ini + doors-groupvars.ini), so a
    # standup that declared a door actually rendered it. Drift-gate on ADR 1117.
    if do_fs:
        gen = os.path.join(REPO, "ops/scripts/generate-door-inventory.py")
        if os.path.exists(gen):
            p = subprocess.run(["python3", gen, "--all", "--check"],
                               capture_output=True, text=True)
            r.append((p.returncode == 0, "env-doors: registry matches the inventories (--all --check)",
                      "" if p.returncode == 0 else p.stdout.strip().replace("\n", " ")[:120]))
    return r

def self_test():
    """P6: plant violations in synthetic inputs; the checker must flag each."""
    bad_driver = (
        'step 6 A "x" "... ansible-preflight.sh site-bootstrap.yml -e site_id=$SITE"\n'
        'step 7 A "y" "... ansible-preflight.sh ory-deploy.yml -e target=$GROUP"\n'   # scoping violation
        'step 13 A "z" "... ansible-preflight.sh goat-netbird-account-init.yml --limit $GROUP"\n'
        'copy prov_conus_hosts/vars.yml\n'                                            # retired env
    )
    bad_handbook = "## Real heading\nsee [x](#no-such-anchor)\n"                       # dangling anchor
    res = run_checks(bad_driver, bad_handbook, do_fs=False)
    by = {step: ok for ok, step, _ in res}
    expect_fail = [
        "scoping: hosts:all playbooks use --limit (not bare -e target=)",
        "site-bootstrap invocations carry required extra-vars",
        "handbook internal anchors resolve",
        "no reference to a retired environment",
    ]
    problems = [s for s in expect_fail if by.get(s, True)]
    # The shape the driver really uses, which the parser once could not see.
    planted = ('OBS="cd x && $ANSIBLE_PRE ${INV[*]} planted-missing.yml -e target=$G"\n'
               'step 12 A "y" "( grep -qE \'k\' group_vars/$G/vars.yml 2>/dev/null && '
               '$ANSIBLE_PRE ${INV[*]} found.yml || echo \'no other.yml\' )"\n'
               '# $ANSIBLE_PRE commented-out.yml\n')
    if missing_plays(planted, lambda n: n == "found.yml") != ["planted-missing.yml"]:
        problems.append("a missing play invoked through $ANSIBLE_PRE")
    bare = 'step 12 A "x" "cd y && ansible-playbook -i localhost, bare-missing.yml -e a=b"\n'
    if missing_plays(bare, lambda n: False) != ["bare-missing.yml"]:
        problems.append("a missing play run by a bare ansible-playbook")
    through_variable = 'PLAY=generic.yml\nstep 5 A "x" "$ANSIBLE_PRE ${INV[*]} $PLAY -e t=g"\n'
    if missing_plays(through_variable, lambda n: False) != ["generic.yml"]:
        problems.append("a missing play run through a variable")
    declared = {"gone.yml": ("what happens", "the handling")}
    uses_gone = '$ANSIBLE_PRE gone.yml\n'
    if missing_plays(uses_gone, lambda n: False, declared):
        problems.append("a declared absence should be excused in a public tree")
    if missing_plays(uses_gone, lambda n: False, {}) != ["gone.yml"]:
        problems.append("a deployment tree should get no excuses")
    if unhandled_absences(uses_gone, lambda n: False, declared) != ["gone.yml"]:
        problems.append("a declared absence whose handling left the driver")
    if problems:
        print("SELF-TEST FAILED — the checker did NOT catch planted violations:")
        for s in problems:
            print(f"  - should have failed: {s}")
        return 2
    print("self-test OK — the checker flags planted violations (scoping, missing "
          "extra-vars, dangling anchor, retired env).")
    return 0

def main():
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    with open(os.path.join(REPO, DRIVER), encoding="utf-8") as source_file:
        driver_text = source_file.read()
    with open(os.path.join(REPO, HANDBOOK), encoding="utf-8") as source_file:
        handbook_text = source_file.read()
    res = run_checks(driver_text, handbook_text, do_fs=True)
    failed = 0
    for ok, step, detail in res:
        mark = "ok  " if ok else "FAIL"
        line = f"  {mark}  {step}"
        if detail:
            line += f"  — {detail}"
        print(line)
        if not ok:
            failed += 1
    print()
    if failed:
        print(f"FAIL: {failed} golden-path check(s) failed.")
        sys.exit(1)
    print(f"ok: golden path coherent — {len(res)} checks passed.")

if __name__ == "__main__":
    main()
