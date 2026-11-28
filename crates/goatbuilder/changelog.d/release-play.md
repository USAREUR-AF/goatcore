---
heading: For someone deploying
title: "Added: a release component may run the play its own archive carries (`release.play`)"
---

A release component's `release.play` names a play inside its archive.
`goatbuilder deploy`, `join-node`, `rollback` and `deploy-component.sh` verify the
archive as before, unpack the play's directory from the verified bytes on the
operator's machine, hold what is there to the archive on every run, and run it
through `release-archive-play.yml`. The play reads this checkout's group_vars,
host_vars and roles; an archive that brings its own, or a role goatcore has, is
refused, as is one that brings Ansible modules, plugins or collections, or a play
whose `hosts:` is not exactly the target group (GB-381, check `play`). The play runs
with the operator's whole authority, so its signer is trusted with that much.

Fixed: a component's play in another repository (`source:`) reads this checkout's
group_vars and host_vars too. Before, Ansible read neither.
