---
heading: For someone deploying
title: "Changed: Ansible caches gathered facts under the user's home directory"
---

`ops/ansible/ansible.cfg` now caches the facts a play gathers in
`~/.cache/goat/ansible-facts`, the running user's own directory, instead of
`/tmp/ansible_facts_cache`, which every local user shares.
`ANSIBLE_CACHE_PLUGIN_CONNECTION` still overrides it. The first run after the
change gathers facts again.
