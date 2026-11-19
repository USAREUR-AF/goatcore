---
heading: For someone deploying
title: Changed: join-node's phase 1 says which case it hit when ssh fails and prints nothing
---

When `goatbuilder join-node`'s phase 1 reach check gets an ssh failure with
no words, GB-314 now says which case it was: ssh exited with its code and
printed nothing on stderr, or the file its stderr went to was gone before the
check read it. Before, both read only `ssh exited <code>`. The check also reads
ssh's stderr before it stops its own timeout, and the timeout can no longer
remove the run's files. A node that answers, or that fails with ssh's own
words, is reported as before.
