#!/usr/bin/env python3
"""slot-fragment-reload — load the portal's slot fragment into the router (GHSA-rr3p).

The portal splices the router's slot subjects (the slot fragment, rendered from
its registry) into the live config.json5 on every slot change. zenohd 1.9 reads
access_control only at start: a live admin-space update is refused ("updating
config is only supported for keys starting with `plugins/`",
zenoh/src/api/config.rs:272 in 1.9.0) and SIGHUP is a no-op (F-156). So the
change reaches the wire only when the router is recreated. This runs on the
router host (systemd .path unit on the config, plus a timer) and recreates it:

  - a change that REMOVES a leaf CN from any subject (revoke, rotate,
    reassignment, a co-publish grant revoked or expired, a carrier taken off a
    carry-<anchor> subject when a carriage rule ends, ADR 1251) is a
    revocation: recreate as soon as --min-gap seconds have passed since the
    last recreate;
  - a change that only ADDS CNs (onboard, collect, machine enrol, a new
    carriage rule) is batched:
    recreate at most once per --interval seconds.

Only after the restart command exits 0 AND the container is a new one (its id
or start time changed) AND it is running, it writes the fragment it loaded to
<config dir>/slot-fragment.loaded.json5. Any other outcome exits 1 and leaves
the record where it was, so the portal keeps answering "pending" and the next
run retries: a failed or no-op restart must never read as a completed
revocation.

A removal is prompt, but at most --prompt-burst prompt recreates happen per
--interval; beyond that, removals wait for the interval too, so nobody who can
revoke (a machine custodian revoking their own machine) can keep the fabric
restarting. The portal reads that file
(it already mounts the directory) to tell whether a slot's current CN is live,
without a docker socket. The portal never restarts anything.

Exit 0 when in sync, waiting for the batch window, or reloaded; 1 when a
recreate left the router down (the loaded state is then not advanced, so the
next run retries).

usage: slot-fragment-reload.py --config PATH --restart CMD --container NAME
                               [--interval S] [--min-gap S] [--settle S]
"""
import argparse
import hashlib
import os
import re
import subprocess
import sys
import time

BEGIN = "      // BEGIN: slot subjects"
END = "      // END: slot subjects"
HEAD = "      // slot fragment sha256:"
# (subject id, leaf CN) pairs: a CN that leaves ONE subject (a co-publish
# grant revoked or expired while the slot keeps its holder, so the CN stays on
# slot-<anchor>) is a revocation too (GHSA-rr3p review r9 F1). A carry-<anchor>
# subject lists several CNs (ADR 1251), so every CN in a list counts.
SUBJECT_RE = re.compile(r'id: "([^"]+)",\n\s*cert_common_names: \[([^\]]*)\]')
LEAF_RE = re.compile(r'"([0-9a-f]{32}(?:\.g[1-9][0-9]*)?)"')


def region(config_text):
    b = config_text.find(BEGIN)
    if b < 0:
        return None
    start = config_text.index("\n", b) + 1
    e = config_text.find(END, start)
    return None if e < 0 else config_text[start:e]


def digest_ok(fragment):
    head, _, body = fragment.partition("\n")
    if not head.startswith(HEAD):
        return False
    declared = head[len(HEAD):].split(" ", 1)[0]
    return hashlib.sha256(body.encode()).hexdigest() == declared


def leaf_cns(fragment):
    """The (subject id, leaf CN) pairs the fragment grants."""
    return {(sid, cn) for sid, cns in SUBJECT_RE.findall(fragment or "") for cn in LEAF_RE.findall(cns)}


def inspect(container):
    """(id, started_at, running) of the container, or None when absent."""
    r = subprocess.run(["docker", "inspect", "-f", "{{.Id}} {{.State.StartedAt}} {{.State.Running}}", container],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return None
    parts = r.stdout.split()
    if len(parts) != 3:
        return None
    return parts[0], parts[1], parts[2] == "true"


def read_reloads(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return [float(x) for x in fh.read().split()]
    except (FileNotFoundError, ValueError):
        return []


def atomic_write(path, text):
    tmp = f"{path}.tmp-{os.getpid()}"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write(text)
    os.chmod(tmp, 0o644)
    os.replace(tmp, path)


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", required=True)
    ap.add_argument("--restart", required=True, help="shell command that recreates the router")
    ap.add_argument("--container", required=True)
    ap.add_argument("--interval", type=int, default=300)
    ap.add_argument("--min-gap", type=int, default=15)
    ap.add_argument("--settle", type=int, default=5)
    ap.add_argument("--prompt-burst", type=int, default=4,
                    help="prompt (removal) recreates allowed per --interval; further removals wait for the interval")
    a = ap.parse_args(argv)

    loaded_path = os.path.join(os.path.dirname(a.config), "slot-fragment.loaded.json5")
    reloads_path = os.path.join(os.path.dirname(a.config), "slot-fragment.reloads")
    with open(a.config, encoding="utf-8") as fh:
        desired = region(fh.read())
    if desired is None:
        print(f"no slot-fragment region in {a.config}: nothing to load")
        return 0
    if not digest_ok(desired):
        print(f"slot fragment in {a.config} fails its digest: not reloading a config the portal did not write",
              file=sys.stderr)
        return 1
    try:
        with open(loaded_path, encoding="utf-8") as fh:
            loaded = fh.read()
        last = os.stat(loaded_path).st_mtime
    except FileNotFoundError:
        loaded, last = None, 0.0
    if loaded == desired:
        print("in sync")
        return 0

    removed = leaf_cns(loaded) - leaf_cns(desired) if loaded is not None else {"(unknown loaded state)"}
    added = leaf_cns(desired) - leaf_cns(loaded)
    now = time.time()
    # A stamp in the future means the clock was stepped back since it was
    # written: it says nothing about the last interval, so it does not count
    # (GHSA-rr3p review r3 F3).
    recent = [t for t in read_reloads(reloads_path) if 0 <= now - t < a.interval]
    prompt = bool(removed) and len(recent) < a.prompt_burst
    wait = a.min_gap if prompt else a.interval
    since = now - last
    if since < 0:
        # The load record is newer than the clock (a backwards step): treat it
        # as old rather than wait out the step with a revocation pending.
        since = float(wait)
    if since < wait:
        kind = "revocation" if prompt else ("revocation, throttled" if removed else "batched")
        print(f"pending: {len(added)} CN(s) to add, {len(removed)} to remove; next load in {int(wait - since)}s ({kind})")
        return 0

    before = inspect(a.container)
    print(f"reloading: {len(added)} CN(s) added, {len(removed)} removed")
    rc = subprocess.run(a.restart, shell=True, check=False).returncode
    time.sleep(a.settle)
    after = inspect(a.container)
    if rc != 0:
        print(f"restart command exited {rc}; loaded state not advanced (retried at the next run)", file=sys.stderr)
        return 1
    if after is None or not after[2]:
        print(f"router {a.container} is not running after the reload; loaded state not advanced", file=sys.stderr)
        return 1
    if before is not None and before[:2] == after[:2]:
        print(f"router {a.container} was not recreated (same container id and start time); loaded state not advanced",
              file=sys.stderr)
        return 1
    atomic_write(loaded_path, desired)
    atomic_write(reloads_path, " ".join(str(t) for t in recent + [now]) + "\n")
    print("reloaded")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
