---
heading: For someone deploying
title: "Fixed: the mesh DNS play verifies a LAN goatnet's management against the goatnet's root"
---

`goat-coredns.yml` registers the site's NetBird nameserver group through the
management API from this machine, and a LAN goatnet's management serves the
goatnet's own certificate, so every call failed certificate verification. The
play now verifies against the goatnet's root in the repository, as the ACL
playbooks do (`netbird_api_ca`, default auto; `public` uses the system roots).
