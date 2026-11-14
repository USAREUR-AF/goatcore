---
heading: For someone contributing
title: Fixed: the goat client's doctor tests write their fake netbird from a child process
---

Two tests of `goat doctor` failed at random: each wrote a fake `netbird`
script and ran it at once, and when another test thread forked while the
script was still open for writing, running it failed with ETXTBSY. A test
helper now writes the script and sets its mode from a short-lived child
process, so the test process never holds it open. Only test code changes
(#314).
