#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test-ledger-promote-pending.py — the deploy ledger must not claim a deploy
# that did not happen.
#
# THE FAILURE THIS EXISTS FOR. `rollback-ledger` is a meta-dependency, so
# ansible runs it BEFORE the role that can refuse the deploy. It used to write
# `action: "deploy"` at that point, and nothing corrected the row when the
# deploy was then refused. Between 2026-09-16 and 2026-09-19 that produced three
# rows across three goatnets asserting deploys that never happened — on
# zagros-prod the ledger named the new digest while the router ran an 11-day-old
# container. `deploy-component.sh` proves convergence by reading that ledger, so
# the check meant to catch an unconverged host confirmed it instead.
#
# The fix is a two-step row: written `pending`, promoted to `deploy` by a
# handler, which ansible runs at end of play and only for hosts that did not
# fail. This file tests the promotion half — that it moves the right row, leaves
# everything else alone, and never destroys a ledger it cannot parse.
#
# THE SECOND FAILURE. The play-end handler is defeated by
# `meta: flush_handlers`, which is PLAY-scoped: ~19 roles flush handlers BEFORE
# their own bring-up/health/smoke (to land a compose/cert change first), and the
# flush fires the promote handler too — stamping `deploy`+`promoted_at` before
# the health gate. A task failing after the flush then left a `deploy` row for a
# deploy that did not finish. #2123 (promoted_at) made that worse by also
# stamping a finish time the deploy never reached. The fix: such a role is
# `rollback_promote_mode: explicit` (its row is NOT queued for the handler) and
# promotes itself from the END of its deploy path via `tasks_from: promote`,
# after the gate — so a role that fails earlier leaves the row `pending`. The
# STRUCTURAL check below encodes that across the repo: {depends on rollback-ledger}
# ∩ {runs meta: flush_handlers} must all be explicit AND carry the promote include.
#
# Exit 0 clean, 1 on a failure.

import json
import re
import os
import subprocess
import sys
import tempfile

import yaml

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(
    REPO, "ansible", "roles", "rollback-ledger", "files", "promote-pending.py"
)

failures = []


def check(name, cond, detail=""):
    if cond:
        print("  ok    %s" % name)
    else:
        print("  FAIL  %s %s" % (name, detail))
        failures.append(name)


def run(path, ref):
    return subprocess.run(
        [sys.executable, SCRIPT, path, ref],
        capture_output=True, text=True, timeout=30,
    )


def read_text(path):
    with open(path) as fh:
        return fh.read()


def rows(path):
    out = []
    with open(path) as source_file:
        for line in source_file.read().splitlines():
            if not line.strip():
                continue
            try:
                out.append(json.loads(line))
            except ValueError:
                out.append({"_unparsed": line})
    return out


def main():
    if not os.path.isfile(SCRIPT):
        sys.stderr.write("missing %s\n" % SCRIPT)
        return 2

    with tempfile.TemporaryDirectory() as d:
        # --- the row this play wrote is promoted; older rows are untouched
        p = os.path.join(d, "ledger.jsonl")
        with open(p, "w") as fh:
            fh.write(json.dumps({"ts": "1", "ref": "img@old", "action": "deploy"}) + "\n")
            fh.write(json.dumps({"ts": "2", "ref": "img@new", "action": "pending"}) + "\n")
        run(p, "img@new")
        r = rows(p)
        check("pending row is promoted", r[1]["action"] == "deploy", r[1])
        check("older row is untouched", r[0] == {"ts": "1", "ref": "img@old", "action": "deploy"}, r[0])
        # ts stays the play's start; promoted_at is the end, a fresh UTC stamp
        check("ts is kept as written", r[1]["ts"] == "2", r[1])
        check("promoted_at is an ISO UTC stamp",
              re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", r[1].get("promoted_at", "")) is not None, r[1])

        # --- running twice does not invent a second promotion
        with open(p) as source_file:
            before = source_file.read()
        run(p, "img@new")
        with open(p) as source_file:
            check("second run is a no-op", source_file.read() == before)

        # --- a refused deploy leaves its row readable as pending
        p2 = os.path.join(d, "refused.jsonl")
        with open(p2, "w") as fh:
            fh.write(json.dumps({"ts": "1", "ref": "img@refused", "action": "pending"}) + "\n")
        run(p2, "img@other")  # a different ref: the handler must not touch this
        check(
            "a row for another ref stays pending",
            rows(p2)[0]["action"] == "pending",
            rows(p2)[0],
        )

        # --- an unparseable line is preserved, not dropped
        p3 = os.path.join(d, "malformed.jsonl")
        with open(p3, "w") as fh:
            fh.write("NOT JSON AT ALL\n")
            fh.write(json.dumps({"ts": "1", "ref": "img@z", "action": "pending"}) + "\n")
        run(p3, "img@z")
        with open(p3) as source_file:
            r3 = source_file.read().splitlines()
        check("unparseable line survives", r3[0] == "NOT JSON AT ALL", r3[0])
        check("the parseable pending row still promotes", json.loads(r3[1])["action"] == "deploy")

        # --- absent ledger is not an error: plays run on hosts with no history
        res = run(os.path.join(d, "absent.jsonl"), "img@x")
        check("absent ledger exits 0", res.returncode == 0, res.stderr)

    # --- the task that CARRIES the notification, not just the script that acts
    #
    # The promotion script above is correct and was never the problem. The
    # second failure was upstream of it: the role's append task dedups on REF
    # alone, so a re-run after a refused deploy found its own `pending` row
    # already naming this ref, skipped the append, and notified nothing. The
    # handler cannot promote a row it is never told about, so the ledger read
    # `pending` on a host that had in fact deployed (mgmt-zagros-prod-echo-01-thor,
    # 2026-09-21). No test of promote-pending.py can see that, because the bug
    # is that promote-pending.py is never reached. So assert the task exists.
    tasks = os.path.join(
        REPO, "ansible", "roles", "rollback-ledger", "tasks", "main.yml"
    )
    with open(tasks) as fh:
        parsed = yaml.safe_load(fh)

    notifiers = [
        t
        for t in parsed
        if "rollback-ledger promote" in str(t.get("notify", ""))
    ]
    check(
        "at least two tasks notify the promote handler",
        len(notifiers) >= 2,
        "found %d: %s" % (len(notifiers), [t.get("name") for t in notifiers]),
    )

    stale = [
        t
        for t in notifiers
        if "pending" in str(t.get("when", "")) and "_rbl_prev" in str(t.get("when", ""))
    ]
    check(
        "one of them fires on a stale pending row for the same ref",
        len(stale) == 1,
        [t.get("name") for t in notifiers],
    )
    if stale:
        t = stale[0]
        check(
            "the stale-pending notifier reports changed (or it notifies nothing)",
            t.get("changed_when") is True,
            t.get("changed_when"),
        )
        check(
            "it only fires when the append was skipped",
            "not (_rbl_changed" in str(t.get("when", "")),
            t.get("when"),
        )

    # Served payloads: the promote reads each file back and records what the
    # host serves, not what the vars meant (goat-get served 0.5.6 on zagros-prod
    # while main declared 0.5.14 and nothing could see it, 2026-10-29).
    with tempfile.TemporaryDirectory() as d:
        led = os.path.join(d, "deploy-history.jsonl")
        with open(led, "w") as fh:
            fh.write(json.dumps({"ref": "nginx@sha256:aa", "action": "pending"}) + "\n")
        cli = os.path.join(d, "version.json")
        with open(cli, "w") as fh:
            json.dump({"version": "0.5.14", "tag": "goat-cli-v0.5.14"}, fh)
        bad = os.path.join(d, "bad.json")
        with open(bad, "w") as fh:
            fh.write("{not json")
        spec = {"goat-cli": cli, "goatbuilder": os.path.join(d, "goatbuilder", "version.json"), "sdk": bad}
        r = subprocess.run([sys.executable, SCRIPT, led, "nginx@sha256:aa", "--served", json.dumps(spec)],
                           capture_output=True, text=True, timeout=30)
        got = rows(led)[-1]
        check("--served: the promote still promotes", got.get("action") == "deploy", r.stdout + r.stderr)
        check("--served: a present file's tag is recorded", (got.get("served") or {}).get("goat-cli") == "goat-cli-v0.5.14", got)
        check("--served: a missing file is absent", (got.get("served") or {}).get("goatbuilder") == "absent", got)
        check("--served: an unparseable file is unreadable", (got.get("served") or {}).get("sdk") == "unreadable", got)
        with open(led, "w") as fh:
            fh.write(json.dumps({"ref": "nginx@sha256:aa", "action": "pending"}) + "\n")
        run(led, "nginx@sha256:aa")
        check("without --served the row carries no served field", "served" not in rows(led)[-1], rows(led)[-1])

    # --- F-530 fix 2: the handler is gated so an explicit role is never queued
    #
    # The queue task in the role must only run in handler mode, or a role that
    # flushes handlers mid-role would have its row promoted at that flush, before
    # its own health/smoke. Assert the guard is present on the queue task.
    with open(tasks) as fh:
        parsed = yaml.safe_load(fh)
    # Tasks use the FQCN module key (ansible.builtin.set_fact), so key off the
    # task name, which is stable, and confirm it is the _rbl_promote_queue write.
    queue = [t for t in parsed
             if "queue this row for the end-of-play promote" in str(t.get("name", ""))
             and "_rbl_promote_queue" in str(t)]
    check("the end-of-play queue task exists", len(queue) == 1, [t.get("name") for t in queue])
    if queue:
        when = str(queue[0].get("when", ""))
        check("the queue task is gated on handler mode (explicit rows are not queued)",
              "rollback_promote_mode == 'handler'" in when, when)

    # promote.yml exists and runs the same promotion script the handler runs
    promote_tasks = os.path.join(
        REPO, "ansible", "roles", "rollback-ledger", "tasks", "promote.yml"
    )
    check("rollback-ledger carries a tasks_from: promote entry point", os.path.isfile(promote_tasks))
    if os.path.isfile(promote_tasks):
        check("promote.yml runs promote-pending.py (one implementation with the handler)",
              "promote-pending.py" in read_text(promote_tasks))

    # --- THE STRUCTURAL INVARIANT (encodes the repo; can only hold, not regress)
    #
    # A role that BOTH depends on rollback-ledger AND runs `meta: flush_handlers`
    # in its tasks can leave a lying `deploy` row unless it promotes AFTER its
    # health gate. Every such role must therefore (a) set
    # `rollback_promote_mode: explicit` in its rollback-ledger dependency stanza
    # and (b) include `tasks_from: promote` in its tasks. And the two halves must
    # go together: a role that declares explicit mode but never promotes would
    # leave every deploy reading `pending` (the F-530 failure with the sign
    # flipped). A new deploying role that adds a mid-role flush and forgets either
    # half fails here.
    roles_dir = os.path.join(REPO, "ansible", "roles")

    def task_files(role):
        td = os.path.join(roles_dir, role, "tasks")
        out = []
        for dp, _, fns in os.walk(td):
            for fn in fns:
                if fn.endswith((".yml", ".yaml")):
                    out.append(os.path.join(dp, fn))
        return out

    def flushes_handlers(role):
        for f in task_files(role):
            txt = read_text(f)
            # ansible: `meta: flush_handlers`, with or without the FQCN.
            if re.search(r"meta:\s*flush_handlers", txt):
                return True
        return False

    def ledger_dep_vars(role):
        """Parse meta/main.yml; return the rollback-ledger dependency's vars dict,
        or None if the role does not really depend on it (a comment does not count)."""
        meta = os.path.join(roles_dir, role, "meta", "main.yml")
        if not os.path.isfile(meta):
            return None
        try:
            doc = yaml.safe_load(read_text(meta)) or {}
        except yaml.YAMLError:
            return None
        for dep in doc.get("dependencies", []) or []:
            if isinstance(dep, str) and dep == "rollback-ledger":
                return {}
            if isinstance(dep, dict) and dep.get("role") == "rollback-ledger":
                return dep.get("vars", {}) or {}
        return None

    def includes_promote(role):
        for f in task_files(role):
            if "tasks_from: promote" in read_text(f):
                return True
        return False

    all_roles = sorted(
        d for d in os.listdir(roles_dir)
        if os.path.isdir(os.path.join(roles_dir, d))
    )

    must_be_explicit = []      # deps ledger AND flushes handlers
    declared_explicit = []     # meta stanza says explicit
    for role in all_roles:
        if role == "rollback-ledger":
            continue
        dep_vars = ledger_dep_vars(role)
        if dep_vars is None:
            # Not a ledger-dependent role: a stray flush here writes no row.
            continue
        if dep_vars.get("rollback_promote_mode") == "explicit":
            declared_explicit.append(role)
        if flushes_handlers(role):
            must_be_explicit.append(role)

    # (1) every flush+ledger role is explicit AND promotes after smoke
    miss_mode = [r for r in must_be_explicit
                 if (ledger_dep_vars(r) or {}).get("rollback_promote_mode") != "explicit"]
    miss_incl = [r for r in must_be_explicit if not includes_promote(r)]
    check(
        "every ledger role that flushes handlers mid-role is rollback_promote_mode: explicit",
        not miss_mode,
        "not explicit: %s" % miss_mode,
    )
    check(
        "every ledger role that flushes handlers mid-role includes tasks_from: promote",
        not miss_incl,
        "no promote include: %s" % miss_incl,
    )

    # (2) the two halves go together — no explicit mode without the promote include
    orphan_mode = [r for r in declared_explicit if not includes_promote(r)]
    orphan_incl = [r for r in all_roles
                   if r != "rollback-ledger"
                   and includes_promote(r)
                   and (ledger_dep_vars(r) or {}).get("rollback_promote_mode") != "explicit"]
    check(
        "a role declaring explicit mode also includes the promote step (no stuck-pending)",
        not orphan_mode,
        "explicit but never promotes: %s" % orphan_mode,
    )
    check(
        "a role that includes the promote step also declares explicit mode",
        not orphan_incl,
        "promotes but not explicit: %s" % orphan_incl,
    )

    # The set is non-trivial: if this drops to zero the detection silently broke.
    check(
        "the flush+ledger set is detected (non-empty)",
        len(must_be_explicit) >= 15,
        "found %d: %s" % (len(must_be_explicit), must_be_explicit),
    )
    print("  info  %d ledger roles flush handlers mid-role (all explicit + promote): %s"
          % (len(must_be_explicit), ", ".join(must_be_explicit)))

    if failures:
        print("\nFAIL: %d check(s) failed" % len(failures))
        return 1
    print("\nok: ledger promote-pending behaves, and is reachable on a retry")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
