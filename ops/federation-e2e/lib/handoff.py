#!/usr/bin/env python3
"""The two-driver hand-off object (docs/design/federation-two-drivers.md, "The hand-off object").

    handoff.py next REPORT           the stop's hand-off from a goatbuilder -o json report:
                                     prints {"run_on", "phase", "command", "carry", "result",
                                     "import", "then"} or {"run_on": null} when there is none
    handoff.py wayback REPORT        the edge run's way-back object: prints {"result_path",
                                     "result_sha256", "confirm_code", "import", "then"}; refuses a
                                     confirm code that is not the first 32 hex digits of the
                                     result's sha256 in groups of four
    handoff.py fill ARGV_JSON --result PATH [--code CODE]
                                     the argv with the whole-element tokens {result} and {code}
                                     replaced; refuses a token inside an element
    handoff.py text-holds TEXT ARGV_JSON
                                     the person's text holds the argv, quoted for a POSIX shell

The confirm code is read from the edge run's own report and nothing else: never from the
result file, the controller, or anything carried with the file.
"""
import json
import re
import shlex
import sys

TOKENS = ("{result}", "{code}")


def die(msg):
    print(f"handoff.py: {msg}", file=sys.stderr)
    sys.exit(2)


def load(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError) as e:
        die(f"{path}: not a JSON report ({e})")


def find_handoff(doc):
    """The `handoff` member, at the report's top level or under `report`."""
    if isinstance(doc, dict):
        if isinstance(doc.get("handoff"), dict):
            return doc["handoff"]
        for v in doc.values():
            h = find_handoff(v) if isinstance(v, (dict, list)) else None
            if h:
                return h
    if isinstance(doc, list):
        for v in doc:
            h = find_handoff(v)
            if h:
                return h
    return None


def argv(v, what, allow_null=False):
    if v is None and allow_null:
        return None
    if not isinstance(v, list) or not v or not all(isinstance(x, str) for x in v):
        die(f"{what} is not an argv (a non-empty list of strings): {v!r}")
    return v


def code_of(sha256):
    h = sha256.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", h):
        die(f"result_sha256 is not a sha256: {sha256!r}")
    d = h[:32]
    return " ".join(d[i:i + 4] for i in range(0, 32, 4))


def norm_code(c):
    return re.sub(r"[^0-9a-f]", "", c.lower())


def cmd_next(path):
    h = find_handoff(load(path))
    if not h:
        print(json.dumps({"run_on": None}))
        return
    run_on = h.get("run_on")
    if run_on not in ("edge", "controller"):
        die(f"run_on is {run_on!r}, not edge or controller")
    out = {"run_on": run_on, "phase": h.get("phase"),
           "command": argv(h.get("command"), "command", allow_null=True),
           "carry": h.get("carry") or [], "result": h.get("result"),
           "import": argv(h.get("import"), "import"), "then": argv(h.get("then"), "then")}
    for c in out["carry"]:
        if not (isinstance(c, dict) and isinstance(c.get("file"), str) and re.fullmatch(r"[0-9a-f]{64}", str(c.get("sha256", "")))):
            die(f"a carry entry is not {{file, sha256}}: {c!r}")
    print(json.dumps(out))


def cmd_wayback(path):
    h = find_handoff(load(path))
    if not h:
        die(f"{path}: no hand-off object (the edge run printed no way back)")
    if h.get("run_on") != "controller":
        die(f"the way-back object's run_on is {h.get('run_on')!r}, not controller")
    if h.get("command") is not None:
        die("the way-back object names a command to run on the edge; it must be null")
    sha, code, rp = h.get("result_sha256"), h.get("confirm_code"), h.get("result_path")
    if not isinstance(rp, str) or not rp:
        die("the way-back object names no result_path")
    if not isinstance(code, str) or norm_code(code) != norm_code(code_of(str(sha))):
        die(f"confirm_code {code!r} is not the first 32 hex digits of result_sha256 {sha!r}")
    print(json.dumps({"result_path": rp, "result_sha256": sha.lower(), "confirm_code": code,
                      "import": argv(h.get("import"), "import"), "then": argv(h.get("then"), "then")}))


def cmd_fill(a, result, code):
    a = argv(json.loads(a), "argv")
    out = []
    for x in a:
        for t in TOKENS:
            if t in x and x != t:
                die(f"the token {t} is inside the element {x!r}; tokens stand for whole elements")
        if x == "{result}":
            x = result
        elif x == "{code}":
            if code is None:
                die("the argv needs {code} and none was given")
            x = code
        out.append(x)
    print(json.dumps(out))


def cmd_text_holds(text_path, a):
    a = argv(json.loads(a), "argv")
    with open(text_path, encoding="utf-8", errors="replace") as fh:
        text = re.sub(r"\x1b\[[0-9;]*m", "", fh.read())
    line = shlex.join(a)
    if line not in text:
        die(f"the person's text does not hold the command: {line}")
    print(line)


def main(av):
    if len(av) >= 2 and av[0] == "next":
        return cmd_next(av[1])
    if len(av) >= 2 and av[0] == "wayback":
        return cmd_wayback(av[1])
    if len(av) >= 4 and av[0] == "fill" and av[2] == "--result":
        code = av[5] if len(av) >= 6 and av[4] == "--code" else None
        return cmd_fill(av[1], av[3], code)
    if len(av) == 3 and av[0] == "text-holds":
        return cmd_text_holds(av[1], av[2])
    die(__doc__.strip().splitlines()[0] + "; see the module text for the forms")


if __name__ == "__main__":
    main(sys.argv[1:])
