#!/usr/bin/env python3
"""answer — run one command on a terminal and answer its confirmations as an operator would.

    answer.py --transcript FILE --typed FILE [--label RUN] [--value NAME=SHELL]... [--deadline S] -- <command...>

The command runs on a pseudo-terminal (so `goatbuilder federate` sees a person and asks its
confirmations there, its released default). Everything it prints goes to FILE (the
transcript) and to stdout. Each question it asks is answered from what the other operator
reads aloud on the call, never from what arrived over the wire:

  "Did <them> read this CA fingerprint aloud?"   yes when the fingerprint the command shows
                                                  for them equals --value partner_ca_fp
  "Type the verifying key <them> reads aloud"     --value partner_key
  "Is this the package for this peering?"         yes when the digest shown equals
                                                  --value partner_package (else Enter)
  "Is theirs the same?" (the agreement digest)    yes when the digest shown equals
                                                  --value partner_agreement (else Enter:
                                                  the partner has not composed yet)
  "approve exactly this plan"                     yes (the operator read the plan above)
  "approve and restart our core router"           yes (the operator read the admission above)

Any other question is answered with Enter (stop) and recorded as unrecognized. Each
--value is a shell command run when its question is asked, so a value the partner has only
just produced is read fresh. Every answer goes to --typed as one JSON line: the question,
exactly what was typed, and why. The exit status is the command's.
"""
import argparse
import json
import os
import pty
import re
import select
import signal
import subprocess
import sys
import time

ap = argparse.ArgumentParser()
ap.add_argument("--transcript", required=True)
ap.add_argument("--typed", required=True)
ap.add_argument("--value", action="append", default=[])
ap.add_argument("--deadline", type=int, default=5400)
ap.add_argument("--label", default="")
argv = sys.argv[1:]
if "--" not in argv:
    ap.error("no command after --")
i = argv.index("--")
a = ap.parse_args(argv[:i])
cmd = argv[i + 1:]
values = dict(v.split("=", 1) for v in a.value)


def value(name):
    sh = values.get(name, "")
    if not sh:
        return ""
    p = subprocess.run(["bash", "-c", sh], capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else ""


def norm(s):
    s = re.sub(r"\s+", "", s or "")
    return (s[7:] if s.startswith("sha256:") else s).lower()


ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")


def shown(text, label):
    """The value the command last showed after `label`, as `N groups: abcd ef01 ...` or plain."""
    hits = re.findall(re.escape(label) + r"\s*(?:\d+ groups:)?\s*((?:sha256:)?[0-9A-Za-z+/=]+(?: [0-9A-Za-z+/=]{1,4})*)", text)
    return norm(hits[-1]) if hits else ""


def decide(question, text):
    q = question
    if "read this CA fingerprint aloud" in q:
        theirs = text.rsplit("Theirs (", 1)[-1] if "Theirs (" in text else ""
        got, want = shown(theirs, "CA fingerprint:"), norm(value("partner_ca_fp"))
        if got and got == want:
            return "yes", f"the CA fingerprint shown for the partner equals the one the partner read aloud ({want[:16]}...)"
        return "", f"the CA fingerprint shown ({got[:16] or 'none'}) is not the one the partner read aloud ({want[:16] or 'none'})"
    if "verifying key" in q and "reads aloud" in q:
        k = value("partner_key")
        return k, "the partner's verifying key, as the partner read it aloud" if k else "no key was read aloud"
    if "Is this the package for this peering" in q:
        want = norm(value("partner_package"))
        if want and want in norm(text[-4000:]):
            return "yes", "the package digest shown is the one the partner names for this peering"
        return "", "the package shown is not the one the partner names for this peering"
    if "Is theirs the same" in q:
        got = shown(text.rsplit("compare the agreement digest", 1)[-1], "listen to theirs:")
        want = norm(value("partner_agreement"))
        if got and want and got == want:
            return "yes", f"the partner read the same agreement digest ({want[:16]}...)"
        return "", f"the partner's agreement digest ({want[:16] or 'not composed yet'}) is not the one shown ({got[:16] or 'none'})"
    if "approve exactly this plan" in q:
        return "yes", "the operator read the plan shown above and approves it"
    if "approve and restart our core router" in q:
        return "yes", "the operator read what our core will admit, shown above, and approves it"
    return "", "unrecognized question: answered with Enter (stop)"


QUESTION = re.compile(r"(Type [^\n]*|[^\n]*\?[^\n]*(?:Type|press Enter)[^\n]*):\s*$")

pid, fd = pty.fork()
if pid == 0:
    os.environ["COLUMNS"] = "200"
    os.execvp(cmd[0], cmd)
try:
    import fcntl
    import struct
    import termios
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 200, 0, 0))
except Exception:
    # The window size only widens the transcript; the prompts are answered at any width.
    pass

with open(a.transcript, "ab") as out, open(a.typed, "a") as typed:
    text, answered_at = "", 0
    end = time.time() + a.deadline
    while True:
        if time.time() > end:
            os.kill(pid, signal.SIGTERM)
            typed.write(json.dumps({"run": a.label, "question": None, "typed": None, "why": f"no end after {a.deadline} s: terminated"}) + "\n")
            break
        r, _, _ = select.select([fd], [], [], 1.0)
        if fd in r:
            try:
                b = os.read(fd, 65536)
            except OSError:
                break
            if not b:
                break
            out.write(b); out.flush()
            sys.stdout.buffer.write(b); sys.stdout.buffer.flush()
            text += ANSI.sub("", b.decode("utf-8", "replace")).replace("\r", "")
            continue
        # Quiet for a second: a question waits at the end of the output, with no newline after it.
        tail = text[answered_at:].rstrip(" ")
        last = tail.rsplit("\n", 1)[-1] if tail else ""
        if last and QUESTION.search(last + ":") and (last.endswith(":") or last.endswith("?")):
            ans, why = decide(last, text)
            typed.write(json.dumps({"run": a.label, "question": last.strip(), "typed": ans, "why": why}) + "\n"); typed.flush()
            os.write(fd, (ans + "\n").encode())
            answered_at = len(text)
    _, status = os.waitpid(pid, 0)
    rc = os.waitstatus_to_exitcode(status)
sys.exit(rc if rc >= 0 else 128 - rc)
