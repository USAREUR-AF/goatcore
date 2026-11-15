---
heading: For someone deploying
title: "Fixed: `federate rotate <partner> carrier` puts the new key on the carrier, refuses a peering with no carrier, and re-runs phase 3"
---

Fixes to `goatbuilder federate rotate <partner> carrier`:
- The edge's carrier now restarts onto its new key. Before, the key file
  changed but the running carrier kept the old key, because its environment
  file names the key's path, not the key.
- A peering that declares no forwarder has no carrier. Rotating its carrier is
  now refused before anything runs, with GB-317 and exit 1. Before, the run
  passed with exit 0 and changed no key. A forwarder counts as declared in
  goatbuilder.yaml or, for a hand-run deployment, by
  `peering_wg_via_forwarder: true` in the edge group's group_vars.
- A forwarder declared that way by hand cannot take the edge's new key from
  the rotation. Rotating its carrier now stops before anything runs, with
  GB-312 and exit 1. Before, the run went ahead with exit 0 and the forwarder
  kept the edge's old key. The refusal names the forwarder's value and play.
  To rotate it, declare the forwarder in goatbuilder.yaml and run
  `goatbuilder federate allocate`. A forwarder declared in goatbuilder.yaml
  takes the new key in the same phase 2.
- The rotation re-runs phase 3. After phase 11 the federation interface
  carries the agreed MTU, which is the lower of both offers. When the partner
  offered less than we did, phase 3 refused our own offer as larger than the
  interface. While our declared offer is still the one we made in the
  agreement the interface carries, it now reports that cross-check as skipped
  and names the agreement. A changed offer is refused as blocked, asking for a
  measurement off the federation interface, since the interface cannot bound
  it. The same applies to a re-run after `rotate <partner> transport`. The
  role's state file counts only when it records a decision that applies, is a
  regular file owned by root, writable by root alone, and is not a link. Our
  offer in the agreement is read from `<work>/ours.composed.json`, the copy of
  our package that phase 6 now keeps beside the agreement; without it the
  cross-check is blocked. A phase 6 that cannot write the copy fails with
  GB-184, as for the working folder's lock and journal. A peering stood up by an earlier build has no copy:
  its first phase 3 takes ours.canonical.json when it is no newer than the
  agreement, and writes the copy from it.
- The peering and carrier key paths, and the ruleset paths beside them, are
  quoted wherever a shell or systemd's EnvironmentFile reads them, so a path
  with a space, `$` or a backtick is read as written. A path holding a newline
  is refused. The forwarder's back-out and the host firewall's apply hand the
  ruleset path to `nft` as one argument.
