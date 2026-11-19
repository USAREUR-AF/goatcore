#!/usr/bin/env python3
"""A background subshell that a script kills by its pid clears the EXIT trap first.

THE FAILURE THIS EXISTS FOR. join-node's phase 1 check (ops/scripts/join-node-reach.sh)
removes its run's known_hosts and stderr files in an EXIT trap, and runs a watchdog
`( sleep "$limit"; ...; kill ... ) &` that it kills once ssh returns. A subshell is
forked with the parent's traps and resets them a moment later. Killed in that moment, it
runs the parent's EXIT trap. Measured on bash 5.3.9, a `( sleep 30 ) &` killed straight
after the fork ran the parent's EXIT trap 30 times in 200; killed 0.2 s after it started,
never. So the check, killing its watchdog as soon as a fast ssh returned, sometimes had
its own files removed before it read them, and a node that answered read as "does not
answer over SSH (ssh exited 255)" (CI, 19 Nov 2026; fixed by #1048). The window is too
narrow to hit on demand: run 200 times, the unfixed script failed once in three tries. So
this check holds the code to the fix: such a subshell begins with `trap - EXIT`.

WHAT IT CHECKS. In a shell script that sets an EXIT trap, every subshell started in the
background, `( ... ) &`, whose pid (`var=$!` on the next line) is later passed to `kill`,
must have `trap - EXIT` as its first command (other signals the parent traps may be
cleared in the same command: `trap - EXIT TERM`).

Usage:
  check-killed-subshells-clear-exit-trap.py [FILE ...]   default: every tracked *.sh
  check-killed-subshells-clear-exit-trap.py --self-test

Exit 0 = every such subshell clears the trap. Exit 1 = one does not; each is named as
<file>:<line>. Exit 2 = a file cannot be read.
"""
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WHY = "a killed background subshell inherits the EXIT trap; begin it with trap - EXIT"

SETS_EXIT_TRAP = re.compile(r"(^|[;&|]\s*|\s)trap\s+(?!-\s)(?!--\s)\S.*\bEXIT\b")
PID = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)=\$!\s*(#.*)?$")
# What may follow the closing parenthesis before the `&`: redirections only.
TAIL = re.compile(r"^\)\s*((?:\d*[<>]{1,2}&?\s*\S+\s*)*)&\s*(#.*)?$")


def subshells(lines):
    """Yield (line number, body) for each `( ... ) &` that starts a statement."""
    i = 0
    while i < len(lines):
        s = lines[i].lstrip()
        if not s.startswith("(") or s.startswith("(("):
            i += 1
            continue
        depth, body, j, done = 0, [], i, None
        text = s
        while j < len(lines):
            for k, ch in enumerate(text):
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                    if depth == 0:
                        body.append(text[:k])
                        done = (j, text[k:])
                        break
            if done:
                break
            body.append(text)
            j += 1
            text = lines[j] if j < len(lines) else ""
        if done and TAIL.match(done[1].strip()):
            inner = "\n".join(body)
            inner = inner[inner.index("(") + 1:]
            yield i + 1, done[0], inner
        i = (done[0] if done else i) + 1


def first_command(body):
    for part in re.split(r"[;\n]", body):
        part = part.strip()
        if part and not part.startswith("#"):
            return part
    return ""


def check_text(path, text):
    lines = text.splitlines()
    problems = []
    for start, end, body in subshells(lines):
        # Only a trap set before the subshell is inherited by it.
        if not any(SETS_EXIT_TRAP.search(l.split("#", 1)[0]) for l in lines[:start - 1]):
            continue
        nxt = next((l for l in lines[end + 1:] if l.strip()), "")
        m = PID.match(nxt)
        if not m:
            continue
        var = m.group(1)
        killed = re.compile(r"\bkill\b[^\n]*\$\{?%s\}?\b" % re.escape(var))
        if not any(killed.search(l.split("#", 1)[0]) for l in lines[end + 1:]):
            continue
        # `trap - EXIT`, alone or with other signals the parent also traps.
        if not re.fullmatch(r"trap\s+-\s+(?:[A-Z0-9]+\s+)*EXIT(?:\s+[A-Z0-9]+)*", first_command(body)):
            problems.append(f"{path}:{start}: {WHY}")
    return problems


def tracked_shell():
    out = subprocess.run(["git", "-C", str(REPO), "ls-files", "*.sh"], capture_output=True, text=True, check=True)
    return [REPO / p for p in out.stdout.split() if p]


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    files = [Path(a) for a in argv] or tracked_shell()
    problems = []
    for f in files:
        try:
            text = f.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            print(f"check-killed-subshells-clear-exit-trap: cannot read {f}: {e}", file=sys.stderr)
            return 2
        shown = f.relative_to(REPO) if f.is_absolute() and REPO in f.parents else f
        problems += check_text(shown, text)
    for p in problems:
        print(p)
    if problems:
        return 1
    print(f"ok: every killed background subshell clears the EXIT trap ({len(files)} file(s))")
    return 0


GOOD = """trap 'rm -f "$f"' EXIT
( trap - EXIT TERM; sleep 5; kill "$pid" ) &
w=$!
kill "$w"
sleep 9 &
other=$!
( trap - EXIT; sleep "$limit"; kill -TERM "$pid" ) </dev/null >/dev/null 2>&1 &
dog=$!
kill -TERM "$dog" 2>/dev/null
"""
BAD = """trap 'rm -f "$f"' EXIT
( sleep "$limit"; : > "$bound"; kill -TERM "$pid" ) </dev/null >/dev/null 2>&1 &
dog=$!
wait "$pid"
kill -TERM "$dog" 2>/dev/null
"""
BAD_MULTILINE = """trap cleanup EXIT
(
  sleep 5
  kill "$pid"
) &
w=$!
kill "$w"
"""
NOT_KILLED = """trap 'rm -f "$f"' EXIT
( sleep 1; echo done ) &
w=$!
wait "$w"
"""
NO_TRAP = """( sleep "$limit"; kill -TERM "$pid" ) &
dog=$!
kill -TERM "$dog"
"""
TRAP_AFTER = """( sleep 1 ) &
dog=$!
trap 'rm -f "$f"' EXIT
kill "$dog"
"""


def self_test():
    cases = [("good", GOOD, 0), ("bad", BAD, 1), ("bad-multiline", BAD_MULTILINE, 1),
             ("not-killed", NOT_KILLED, 0), ("no-trap", NO_TRAP, 0), ("trap-after", TRAP_AFTER, 0)]
    failed = 0
    for name, text, want in cases:
        got = check_text(name, text)
        if len(got) != want:
            failed += 1
            print(f"self-test FAIL {name}: want {want} finding(s), got {got}")
        elif want and not got[0].endswith(WHY):
            failed += 1
            print(f"self-test FAIL {name}: the finding does not say why: {got[0]}")
        else:
            print(f"  ok   {name}")
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "x.sh"
        p.write_text(BAD)
        rc = main([str(p)])
        if rc != 1:
            failed += 1
            print(f"self-test FAIL: a bad file exits {rc}, not 1")
    print("self-test: PASS" if not failed else "self-test: FAIL")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
