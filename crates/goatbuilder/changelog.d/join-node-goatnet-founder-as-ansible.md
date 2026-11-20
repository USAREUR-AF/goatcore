---
heading: For someone deploying
title: "Fixed: join-node reads goatnet_founder as Ansible does"
---

`join-node` decided whether a site joins a goatnet by matching the line
`goatnet_founder: false` in the site group's vars.yml. So `False`, `"false"`,
`no` and `off` read as a founder, and a `goatnet_founder` nested under another
key read as the site's own. It now reads the top-level key with Ansible's
booleans, as site-bootstrap does (`goatnet_founder | default(true) | bool`).
