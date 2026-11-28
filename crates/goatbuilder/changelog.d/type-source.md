---
heading: For someone deploying
title: "Added: a target type's plays may be in another repository, at a pinned commit"
---

A type file may declare `source:` with `repository` and `commit` (a full
commit), and mark its plays there `from: source`, each a path inside the
repository. `join-node` and `standup` fetch that commit with your own git access
before their first phase, hold the copy to it on every run, and run the plays
with goatcore's roles first; nothing of it is put in the checkout. A source that
cannot be reached, is not exactly the commit, has no such play, or carries a role
goatcore has is refused before any phase (GB-420). A play from a component's or a
type's source now reads the checkout's `group_vars` and `host_vars`; before, it
read neither.
