---
heading: For someone deploying
title: Changed: the access-rule cutover has a cut-off mode that only updates existing policies
---

`ops/ansible/netbird-acl-cutover.yml -e cut_off=true` is for a site cut off
from the others. It updates the access-rule policies the site already has, by
their ids, and never creates or deletes one: NetBird assigns a created
policy's id, so two cut-off sites that each created the same rule would leave
two policies after the heal. A run that would create a policy, reap one
(`reap_orphans=true`), delete the Default policy, or tear down refuses with
GB-295 before it writes anything, naming each policy it would have created and
each id it would have deleted. Disabling the Default policy is an update and
still runs. After the heal, run the cutover at the hub without `cut_off` to
make the creates and deletes. Without `cut_off` the cutover runs as before.
