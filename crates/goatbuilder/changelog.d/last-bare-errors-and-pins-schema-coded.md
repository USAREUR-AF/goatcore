---
heading: For someone deploying
title: "Fixed: standup's and tutor's last uncoded errors, and pins' schema refusals, carry a code"
---

Eight places in `goatbuilder standup` and `goatbuilder tutor` passed a system
error up with no catalog code, so a log of a run that met one showed no
code. A phase log that could not be opened for both of a phase's streams now
prints GB-254 and names the log. A tutor workspace whose fixture directory
could not be listed prints GB-254 and names the directory. Both still exit 2.
A failed wait for a phase, image pull or observability leg that standup
started now prints GB-322 and exits 1, as every GB-322 does, where it exited 2.

`goatbuilder pins`, and the verbs that run its check (`release`, the
preflight, the fast lane), refused a pins file that broke the pins schema with
no catalog code. Each of those refusals now carries the new code GB-365 and
names the file and the rule. It still exits 2. An unmigrated-pins list that
cannot be read now prints GB-254 and names the file, still exit 2.
