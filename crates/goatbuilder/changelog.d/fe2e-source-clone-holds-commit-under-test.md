---
heading: For someone contributing
title: Fixed: the federation test environment's source clone holds every object of the commit under test
---

The federation test environment builds each commit under test from its own
clone. From a shallow source repository, that clone could miss a commit
that no ref held, so the checkout failed with `fatal: unable to read tree`
and the build found no `crates/` directory. The clone
(`ops/federation-e2e/lib/srctree.sh`, used for goatcore and for the
saltlick directory) is now a plain clone, and fetches the commit by sha
when it cannot read its tree, so it holds the commit's objects itself. A
clone from an earlier run is reused only when it is checked out at that
commit and can read its tree. A failure stops with `no tree for <sha>`
instead of going on to build nothing, and a sha that is not 40 lower-case
hex characters is refused before anything is removed: an empty one would
have named the directory of every cached tree (#638).
