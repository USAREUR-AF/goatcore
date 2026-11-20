#!/usr/bin/env python3
"""check-fixture-expiry.py: fail well before a signed fixture or vector expires.

A test that reads the wall clock over a committed signed fixture passes until
the fixture's term ends and then fails, or worse, keeps passing while it
compares two refusals. This gate fails DAYS days (default 60) ahead instead,
naming the file, so the fixture is renewed while there is time.

What it reads, from the files git tracks:

  1. Every tracked JSON, YAML or TOML file with a signature member
     (`signature` or `sig`) and an expiry member (`expires`, `expires_at`,
     `not_after`, `notAfter`, `valid_until`). Each date must be at least DAYS
     days after today.
  2. Every PEM certificate in a tracked text file that decodes, with or
     without escaped newlines. Its notAfter must be at least DAYS days after
     today. A placeholder block that does not decode is not a certificate.

Exempt, BY NAME only: the directories listed in ops/ci/fixture-expiry-exempt.txt.
Each is a set of normative vectors that every reader evaluates on the date in
its own `now.txt`, never on the wall clock, so its dates are fixed by design
and are never renewed. An entry must name a tracked directory holding a
tracked now.txt, or the gate fails: the list cannot exempt anything else, and
a stale entry does not linger. A test that judges a vector on the wall clock
is the bug, not the vector's date.

usage:
  check-fixture-expiry.py [--today YYYY-MM-DD] [--days N]
  check-fixture-expiry.py --self-test

`--today` exists for the self-test and for asking "what fails on that date";
CI runs it without, on the UTC date.
"""

import base64
import datetime
import os
import re
import shutil
import subprocess
import sys
import tempfile

SIGNED = re.compile(rb'(?m)(^|[\s{,"])"?(signature|sig)"?\s*[:=]')
EXPIRY = re.compile(
    rb'(?m)(?:^|[\s{,"])"?(expires|expires_at|not_after|notAfter|valid_until)"?\s*[:=]\s*'
    rb'"?(\d{4}-\d{2}-\d{2})')
PEM = re.compile(r"-----BEGIN CERTIFICATE-----(.*?)-----END CERTIFICATE-----", re.S)
DATA_SUFFIXES = (".json", ".json5", ".yaml", ".yml", ".toml")
EXEMPT_LIST = "ops/ci/fixture-expiry-exempt.txt"


def tracked(root):
    out = subprocess.run(["git", "-C", root, "ls-files", "-z"], check=True,
                         capture_output=True).stdout
    return [p for p in out.decode("utf-8", "surrogateescape").split("\0") if p]


def read(root, rel):
    try:
        with open(os.path.join(root, rel), "rb") as fh:
            return fh.read()
    except OSError:
        return None


def cert_not_after(body):
    """notAfter of a PEM body, or None when it is not a certificate."""
    try:
        from cryptography import x509
    except ImportError:
        sys.exit("check-fixture-expiry: needs the `cryptography` package to read certificates")
    der_b64 = "".join(re.findall(r"[A-Za-z0-9+/=]+", body))
    try:
        cert = x509.load_der_x509_certificate(base64.b64decode(der_b64, validate=True))
    except Exception:
        return None
    return cert.not_valid_after_utc.date()


def exemptions(root, files):
    """The exempt directories, and a failure for each entry that is not a
    tracked directory holding a tracked now.txt with a date in it."""
    raw = read(root, EXEMPT_LIST)
    if raw is None:
        return [], ["%s: missing; it lists the vector sets exempt by name (it may list none)"
                    % EXEMPT_LIST]
    dirs, bad = [], []
    tracked_set = set(files)
    for n, line in enumerate(raw.decode("utf-8").splitlines(), 1):
        entry = line.split("#", 1)[0].strip().rstrip("/")
        if not entry:
            continue
        now = entry + "/now.txt"
        if now not in tracked_set:
            bad.append("%s:%d: %s is not a vector set (no tracked %s); only a directory "
                       "evaluated on its own now.txt may be exempt" % (EXEMPT_LIST, n, entry, now))
            continue
        try:
            datetime.date.fromisoformat(read(root, now).decode().strip())
        except (AttributeError, ValueError):
            bad.append("%s:%d: %s holds no YYYY-MM-DD date" % (EXEMPT_LIST, n, now))
            continue
        dirs.append(entry)
    return dirs, bad


def scan(root, today, days):
    """Return (failures, counts)."""
    floor = today + datetime.timedelta(days=days)
    files = tracked(root)
    exempt, failures = exemptions(root, files)
    counts = {"exempt vector sets": len(exempt), "signed-file dates": 0, "certificates": 0}

    def judge(rel, what, date):
        if date < floor:
            left = (date - today).days
            when = "expired %d days ago" % -left if left < 0 else "expires in %d days" % left
            failures.append("%s: %s %s %s; renew it %d or more days ahead"
                            % (rel, what, date, when, days))

    for rel in files:
        if any(rel.startswith(d + "/") for d in exempt):
            continue
        raw = read(root, rel)
        if raw is None or b"\0" in raw[:8192] and not rel.endswith(".json"):
            continue
        if rel.endswith(DATA_SUFFIXES) and SIGNED.search(raw):
            for m in EXPIRY.finditer(raw):
                try:
                    date = datetime.date.fromisoformat(m.group(2).decode())
                except ValueError:
                    continue
                counts["signed-file dates"] += 1
                judge(rel, m.group(1).decode(), date)
        if b"BEGIN CERTIFICATE" in raw:
            text = raw.decode("utf-8", "replace").replace("\\n", "\n")
            for m in PEM.finditer(text):
                date = cert_not_after(m.group(1))
                if date is not None:
                    counts["certificates"] += 1
                    judge(rel, "certificate notAfter", date)
    return failures, counts


def self_test():
    made = []

    def tree(files):
        d = tempfile.mkdtemp(prefix="fixture-expiry-")
        made.append(d)
        subprocess.run(["git", "init", "-q", d], check=True)
        for rel, body in files.items():
            os.makedirs(os.path.join(d, os.path.dirname(rel)), exist_ok=True)
            with open(os.path.join(d, rel), "wb") as fh:
                fh.write(body)
        subprocess.run(["git", "-C", d, "add", "-A"], check=True)
        return d

    today = datetime.date(2026, 11, 15)
    listed = {EXEMPT_LIST: b"v  # vectors, evaluated on v/now.txt\n"}
    none_listed = {EXEMPT_LIST: b"# none\n"}
    vec = {"v/now.txt": b"2026-09-05\n", "v/a/p.json": b'{"signature":"AAAA","expires":"2026-11-20"}'}
    cases = [
        ("a signed vector in a set exempt by name passes", dict(vec, **listed), 0),
        ("the same vector set not on the list fails", dict(vec, **none_listed), 1),
        ("a missing exemption list fails", dict(vec), 1),
        ("an exempt entry with no now.txt fails",
         {EXEMPT_LIST: b"t\n", "t/f.json": b'{"signature":"AAAA","expires":"2099-12-31"}'}, 1),
        ("an exempt entry whose now.txt holds no date fails",
         {EXEMPT_LIST: b"v\n", "v/now.txt": b"soon\n"}, 1),
        ("a file beside an exempt set, not under it, is judged",
         dict(listed, **{"v/now.txt": b"2026-09-05\n", "vx/f.json": b'{"signature":"AAAA","expires":"2026-12-01"}'}), 1),
        ("a signed fixture outside a vector set fails 60 days ahead",
         dict(none_listed, **{"t/f.json": b'{"signature":"AAAA","expires":"2026-12-31"}'}), 1),
        ("a signed fixture 61 days out passes",
         dict(none_listed, **{"t/f.json": b'{"signature":"AAAA","expires":"2027-01-15"}'}), 0),
        ("a signed YAML fixture already expired fails",
         dict(none_listed, **{"t/f.yaml": b"sig: AAAA\nnot_after: 2020-01-01\n"}), 1),
        ("an unsigned file with an expiry is not a signed fixture",
         dict(none_listed, **{"t/f.yaml": b"expires: 2026-11-20\n"}), 0),
        ("a placeholder certificate block is not a certificate",
         dict(none_listed, **{"t/x.go": b'"-----BEGIN CERTIFICATE-----\\nMIIB...AB\\n-----END CERTIFICATE-----\\n"'}), 0),
    ]
    bad = 0
    for name, files, want in cases:
        failures, _ = scan(tree(files), today, 60)
        got = 1 if failures else 0
        print("  %s  %s" % ("ok  " if got == want else "FAIL", name))
        if got != want:
            bad += 1
            for f in failures:
                print("        " + f)
    if bad == 0:
        failures, _ = scan(tree(dict(vec, **none_listed)), today, 60)
        if not (len(failures) == 1 and failures[0].startswith("v/a/p.json: expires 2026-11-20 expires in 5 days")):
            print("  FAIL  a failure names the file, the date and the days left: %r" % failures)
            bad += 1
        else:
            print("  ok    a failure names the file, the date and the days left")
    for d in made:
        shutil.rmtree(d)
    print("check-fixture-expiry: self-test %s" % ("FAILED" if bad else "ok"))
    return 1 if bad else 0


def main(argv):
    today = datetime.datetime.now(datetime.timezone.utc).date()
    days = 60
    args = list(argv)
    if args == ["--self-test"]:
        return self_test()
    while args:
        a = args.pop(0)
        if a == "--today" and args:
            today = datetime.date.fromisoformat(args.pop(0))
        elif a == "--days" and args:
            days = int(args.pop(0))
        else:
            sys.exit(__doc__.split("usage:")[1].split("`--today`")[0])
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True,
                          capture_output=True, text=True).stdout.strip()
    failures, counts = scan(root, today, days)
    for f in failures:
        print("check-fixture-expiry: FAIL " + f)
    summary = ", ".join("%d %s" % (v, k) for k, v in counts.items())
    if failures:
        print("check-fixture-expiry: %d failing (%s read, judged on %s). Renew the fixture from "
              "its generator, or, if a test judges it on the wall clock and it is a normative "
              "vector, make the test pass the vectors' now.txt instead; never re-date a vector."
              % (len(failures), summary, today))
        return 1
    print("check-fixture-expiry: ok, nothing signed expires within %d days of %s (%s read)"
          % (days, today, summary))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
