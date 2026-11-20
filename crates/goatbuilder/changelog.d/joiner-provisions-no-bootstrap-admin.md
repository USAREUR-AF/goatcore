---
heading: For someone deploying
title: "Fixed: a joining site provisions no bootstrap administrator of its own"
---

`join-node` phase 13 runs `ory-deploy.yml` at a joining site, and the ory role
created a Kratos identity, with a generated password, for each `site_admins`
entry that site's Kratos did not yet hold. A joiner's identities arrive from its
founder by replication, so that gave each administrator a second identity at the
joiner. The role now runs its bootstrap-admin tasks only at a founder site:
`goatnet_founder` unset or true, read as site-bootstrap reads it. A founder's
run is unchanged.
