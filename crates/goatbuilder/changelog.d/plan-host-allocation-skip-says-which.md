---
heading: For someone deploying
title: "Fixed: a host-allocation SKIP says which case it is and what clears it"
---

A plan whose host-allocation check was SKIP cannot be deployed: deploy does
not run that check again, so it refuses the plan (GB-188), and a new plan
records the same SKIP. The refusal used to say only "host-allocation was SKIP
when the plan was made". Now the plan view, the refusal and the run record
carry the check's detail, which gives the case and the route:
- the inventory names no host for this peering: run `goatbuilder federate
  allocate <partner>`, or name the hosts in `peering-edges.ini`;
- another peering on a shared machine has a value that does not resolve: each
  value is named with its key and the group whose vars set it;
- its agreement was not found: both paths that were looked at are named.

GB-188's catalog entry gains this cause (#742).
