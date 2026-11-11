#!/usr/bin/env python3
"""build-failure.py LOG: why a `docker build --progress=plain` failed, from the failing step only.

Prints "network: <line>" when the step that failed (the last "#N ERROR") could not reach a
package source, else "other: <its first error line>". A fetch that an earlier or the same
step retried successfully, followed by a real error, is "other".
"""
import re
import sys

NET = re.compile(r"Temporary failure resolving|Could not connect to|Connection (timed out|refused)|"
                 r"Unable to connect|Could not resolve|Network is unreachable|No route to host")

lines = open(sys.argv[1], encoding="utf-8", errors="replace").read().splitlines()
failed = None
for l in lines:
    m = re.match(r"#(\d+) ERROR", l)
    if m:
        failed = m.group(1)
if failed is None:
    print("other: no failing build step in the log")
    sys.exit(0)
step = [l for l in lines if l.startswith(f"#{failed} ")]
# apt prints the fetches it gave up on ("E: Failed to fetch ...") and why ("Temporary failure ...").
gave_up = [l for l in step if re.search(r"\bE: (Failed to fetch|Unable to fetch)", l)]
net = [l for l in step if NET.search(l)]
if gave_up and net:
    print("network: " + net[-1].split(" ", 2)[-1].strip())
else:
    err = [l for l in step if " ERROR" in l or re.search(r"\bE: ", l)]
    print("other: " + (err[0] if err else step[-1]).split(" ", 2)[-1].strip())
