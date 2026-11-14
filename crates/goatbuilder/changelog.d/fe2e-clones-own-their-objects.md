---
heading: For someone contributing
title: Fixed: the federation test environment's other clones own their objects
---

The watcher's preview and reverts and the upgrade unit made `--shared`
clones, which read every object from the source and lose a commit once the
source prunes it. They now use `src_clone_ref` in
`ops/federation-e2e/lib/srctree.sh`: a plain local clone at a branch, tag
or sha, which keeps its objects and refuses a shallow source or an empty
argument before anything is removed. No harness script makes a `--shared`
clone any more (#643).
